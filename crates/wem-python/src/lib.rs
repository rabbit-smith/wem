//! PyO3 (abi3) native extension for the wem-core WEM encoder kernel.
//!
//! Exposes the module `wwise_wem._core` (imported in-package; the
//! repository-root maturin wheel installs it as `wwise_wem/_core.abi3.so`) — a thin, zero-drift shell over
//! the kernel:
//!
//! * `Encoder` — one-shot PCM-to-WEM encode (`wem_core::encoder::Encoder`)
//! * `StreamSession` — the core streaming lifecycle
//!   (Init -> chunks* -> Finish; `wem_core::stream::StreamSession`)
//! * `Decoder` — the streaming decode lifecycle with the data direction
//!   reversed (Init -> chunks* -> Finish; `wem_core::decoder::DecodeSession`),
//!   the mirror of `include/wem.h` section 5: WEM bytes in, the one-time
//!   header announcement and interleaved f32 PCM out. There are no callbacks
//!   here — each step *returns* what a C caller would receive through
//!   `header_cb` / `pcm_cb` — and the geometry the C ABI announces through
//!   `header_cb` arrives on the step that resolved it.
//! * [`WwiseVersion`] / [`WwiseProfile`] — the structured profile selector
//!   (`wem_core::{WwiseVersion, WwiseProfile}`; C ABI `WemVersion` /
//!   `WemProfile`): one Wwise generation plus the PCM geometry
//! * [`WemEncoderError`] — the single terminal exception; its `.code`
//!   carries the kernel error-code name (stable across the cross-language
//!   shells) and its message carries the kernel diagnostic
//!   (`EncoderError` Display text).
//!
//! Design rules (docs/reference/architecture.md, crates/AGENTS.md):
//! * No second implementation: validation, profile loading, checksums and
//!   encoding all run in wem-core / wem-profiles public APIs. This layer
//!   only converts Python values to kernel types and kernel errors to
//!   Python exceptions.
//! * Blocking/CPU-heavy kernel calls run under `Python::allow_threads` so
//!   the GIL is not held while the kernel works.
//! * No unwind into the interpreter: every kernel call is caught at this
//!   boundary. A kernel panic means the kernel broke an invariant, never
//!   that the caller passed something bad, and it reaches Python as
//!   [`WemEncoderError`] carrying `.code == "INTERNAL"` — the code the C ABI
//!   reports for the same fault, and the same `.code` every other kernel
//!   error travels with, so `except Exception` sees it (pyo3's own
//!   `PanicException` derives from `BaseException` and would escape that).
//!   A panic inside a handle is terminal for that handle (include/wem.h,
//!   "Panics"): the exception says so, and every later call on it raises
//!   `STATE_ERROR` instead of touching the kernel again.
//!
//! [`WemEncoderError`] is the single terminal exception for both directions,
//! exactly as `include/wem.h` declares one `WemError` table for both: the
//! encode classes plus the decode surface's `INPUT_MALFORMED`. The name is
//! the historical one; nothing about the class is encoder-specific.

use std::panic::{AssertUnwindSafe, UnwindSafe};
use std::sync::atomic::{AtomicBool, Ordering};

use pyo3::exceptions::{PyException, PyValueError};
use pyo3::ffi::c_str;
use pyo3::prelude::*;
use pyo3::type_object::PyTypeInfo;
use pyo3::types::{PyBytes, PyDict, PyList, PyTuple, PyType};

use wem_core::decoder::{
    DecodeSession as WemDecodeSession, DecodeStep as WemDecodeStep, DecodedHeader,
};
use wem_core::encoder::{EncodeResult as WemEncodeResult, Encoder as WemEncoder, Pcm16};
use wem_core::error::{DecoderError, EncoderError};
use wem_core::stream::StreamSession as WemStreamSession;
use wem_core::{WwiseProfile, WwiseVersion};

/// This shell's panic contract needs unwinding panics: `catch_unwind` stops
/// catching under `panic = "abort"`, where a kernel panic would abort the
/// interpreter instead of raising `INTERNAL`. No profile in this workspace
/// sets `panic`, so the default applies; this guard keeps a profile edit from
/// breaking the promise silently.
#[cfg(panic = "abort")]
compile_error!(
    "wem-python's panic contract requires unwinding: a kernel panic is caught and \
     raised as WemEncoderError with code INTERNAL, and under `panic = \"abort\"` \
     catch_unwind silently stops catching, aborting the interpreter instead. \
     Remove the `panic = \"abort\"` setting from the profile that builds this \
     crate."
);

// ---------------------------------------------------------------------------
// Error surface (kernel encoder error codes)
// ---------------------------------------------------------------------------

pyo3::create_exception!(
    wwise_wem._core,
    WemEncoderError,
    PyException,
    "Terminal wem-core encoder failure."
);

/// Stable kernel error-code names (the same classes every cross-language
/// shell maps from `EncoderError`).
const CODE_PROFILE_NOT_FOUND: &str = "PROFILE_NOT_FOUND";
const CODE_GEOMETRY_MISMATCH: &str = "GEOMETRY_MISMATCH";
const CODE_INPUT_TOO_SHORT: &str = "INPUT_TOO_SHORT";
const CODE_FORMAT_UNSUPPORTED: &str = "FORMAT_UNSUPPORTED";
const CODE_STATE_ERROR: &str = "STATE_ERROR";
const CODE_INTERNAL: &str = "INTERNAL";
/// The decode surface's malformed-input class (include/wem.h section 2). It is
/// never reported by an encode call, exactly as `WEM_ERR_INPUT_MALFORMED` is
/// never returned by an encode entry.
const CODE_INPUT_MALFORMED: &str = "INPUT_MALFORMED";

/// The single place a `.code` is attached to the raised exception: the stable
/// cross-language class plus a message, exactly as the caller will read both.
fn error_with_code(code: &str, message: String) -> PyErr {
    Python::with_gil(|py| {
        let pyerr = WemEncoderError::new_err(message);
        let instance = pyerr.value(py);
        let _ = instance.setattr("code", code);
        pyerr
    })
}

/// The stable code name of one kernel error.
fn code_of(err: &EncoderError) -> &'static str {
    match err {
        EncoderError::ProfileNotFound { .. } => CODE_PROFILE_NOT_FOUND,
        EncoderError::StateError { .. } => CODE_STATE_ERROR,
        EncoderError::GeometryMismatch { .. } => CODE_GEOMETRY_MISMATCH,
        EncoderError::InputTooShort { .. } => CODE_INPUT_TOO_SHORT,
        EncoderError::FormatUnsupported { .. } => CODE_FORMAT_UNSUPPORTED,
        EncoderError::Internal(_) => CODE_INTERNAL,
    }
}

/// The stable code name of one kernel *decode* error (include/wem.h section 5).
///
/// Four classes, one per row of that section, mapped exactly as
/// `crates/wem-capi` maps `DecoderError` onto `WemError`: the input's own bytes
/// do not parse; the container parses but names a configuration this build does
/// not carry; the call was made outside the lifecycle; or this library broke an
/// invariant. The match has no `_` arm on purpose — a variant added to
/// `DecoderError` fails this build until its class is written.
fn code_of_decoder(err: &DecoderError) -> &'static str {
    match err {
        DecoderError::Container(_)
        | DecoderError::Setup { .. }
        | DecoderError::SetupPadding { .. }
        | DecoderError::Truncated { .. }
        | DecoderError::MissingSetup { .. }
        | DecoderError::BlockSizeMismatch { .. }
        | DecoderError::Packet { .. }
        | DecoderError::ResidueBitstreamDefect { .. }
        | DecoderError::FrameCountMismatch { .. } => CODE_INPUT_MALFORMED,
        DecoderError::NotWwiseVorbis { .. }
        | DecoderError::ConfigurationUnsupported { .. }
        | DecoderError::SetupNotCarried { .. } => CODE_FORMAT_UNSUPPORTED,
        DecoderError::StateError { .. } => CODE_STATE_ERROR,
        DecoderError::Floor1 { .. } | DecoderError::Internal(_) => CODE_INTERNAL,
    }
}

/// One kernel error class as this shell reports it: the stable code plus the
/// kernel's own diagnostic text.
///
/// Both directions are covered by one rule because `include/wem.h` declares one
/// `WemError` table for both, so the code attachment and the panic rule below
/// are written once and apply to either error type.
trait KernelError: std::fmt::Display + Sized {
    /// The stable cross-language code name (the C ABI's `WemError` spelling).
    fn code(&self) -> &'static str;

    /// The Python exception carrying `.code` and the kernel's untouched
    /// message.
    fn to_pyerr(self) -> PyErr {
        error_with_code(self.code(), self.to_string())
    }
}

impl KernelError for EncoderError {
    fn code(&self) -> &'static str {
        code_of(self)
    }
}

impl KernelError for DecoderError {
    fn code(&self) -> &'static str {
        code_of_decoder(self)
    }
}

/// Map one kernel error to the Python exception: the stable `.code` string
/// plus the kernel's own diagnostic message (zero drift — the message is
/// `EncoderError`'s Display output, untouched).
fn error_to_pyerr(err: EncoderError) -> PyErr {
    err.to_pyerr()
}

/// Binding-level rejection that is semantically a kernel STATE_ERROR
/// (malformed input container; the kernel's own StateError Display is
/// used so the error pipeline stays single-sourced).
fn binding_state_error(message: String) -> PyErr {
    error_to_pyerr(EncoderError::StateError { message })
}

// ---------------------------------------------------------------------------
// The panic rule at this boundary (include/wem.h, "Panics")
// ---------------------------------------------------------------------------

/// Where one Python entry point sits in the panic rule.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum PanicScope {
    /// The call owns no state that survives it (constructing a handle): a
    /// panic means nothing was created, and the caller may try again.
    OneShot,
    /// The call ran inside a handle that outlives it: a panic leaves that
    /// handle's state unknown, so the handle is dead afterwards.
    Handle,
}

/// The exception a caught kernel panic becomes, for the entry of the given
/// scope.
///
/// Two things must hold, and this is the one place both are decided:
/// * the code is `INTERNAL`, the code the C ABI reports for a caught panic, so
///   a caller branching on `.code` sees the same class on every shell;
/// * the message names the consequence — the handle is unusable — instead of
///   leaving the caller to guess whether the object is still worth anything.
fn panic_error(scope: PanicScope, handle: &str) -> PyErr {
    let message = match scope {
        PanicScope::OneShot => format!(
            "kernel panic while creating the {handle}: no {handle} was created and \
             the library is in an unknown state; this is a defect in the kernel, \
             not a rejection of the input"
        ),
        PanicScope::Handle => format!(
            "kernel panic in the {handle}: this {handle} is unusable and must be \
             discarded; this is a defect in the kernel, not a rejection of the input"
        ),
    };
    error_with_code(CODE_INTERNAL, message)
}

/// The message of the rejection a dead handle answers with.
fn unusable_error(handle: &str) -> PyErr {
    error_with_code(
        CODE_STATE_ERROR,
        format!("{handle} is unusable: a kernel defect already killed it"),
    )
}

/// One guarded kernel call.
struct Guarded<T> {
    /// The call's result, with a caught panic already mapped (see
    /// [`panic_error`]).
    outcome: Result<T, PyErr>,
    /// Whether the handle the call ran in is dead afterwards: true for a
    /// defect — a caught panic, or the kernel's own `INTERNAL` — inside a
    /// [`PanicScope::Handle`] call. Every other code is a rejection of the
    /// call and leaves the handle usable, exactly as the kernel's own session
    /// survives it.
    handle_dead: bool,
}

/// The whole panic rule as a pure function, kept out of the `#[pymethods]`
/// bodies so it can be tested directly: forcing a real panic through a kernel
/// call would need a fault-injection switch inside the kernel, and shipping a
/// switch that can abort an encode is not worth the test.
///
/// Generic over the kernel error type: the rule is `include/wem.h`'s for every
/// entry, and both directions reach it with their own error enum.
fn guarded_outcome<T, E: KernelError>(
    scope: PanicScope,
    handle: &str,
    caught: std::thread::Result<Result<T, E>>,
) -> Guarded<T> {
    match caught {
        Ok(Ok(value)) => Guarded {
            outcome: Ok(value),
            handle_dead: false,
        },
        Ok(Err(error)) => {
            let handle_dead = error.code() == CODE_INTERNAL && scope == PanicScope::Handle;
            Guarded {
                outcome: Err(error.to_pyerr()),
                handle_dead,
            }
        }
        Err(_) => Guarded {
            outcome: Err(panic_error(scope, handle)),
            handle_dead: scope == PanicScope::Handle,
        },
    }
}

/// Run one kernel call under the panic rule.
fn guard<T, E: KernelError>(
    scope: PanicScope,
    handle: &str,
    work: impl FnOnce() -> Result<T, E> + UnwindSafe,
) -> Guarded<T> {
    guarded_outcome(scope, handle, std::panic::catch_unwind(work))
}

// ---------------------------------------------------------------------------
// PCM input conversion
// ---------------------------------------------------------------------------

/// Accept the two documented Python forms for `Encoder.encode_pcm`:
/// a list of per-channel int lists, or a 2-D signed-16 memoryview
/// (shape `(channels, frames)`, C-contiguous).
///
/// Content validation (rate/geometry/length) is left entirely to the
/// kernel (`Pcm16::new` + `Encoder::encode_pcm`) so rejection conditions
/// cannot drift.
///
/// The two forms are tried in that order, and when neither can be read the
/// error the caller gets is the one for the form its argument was written in.
/// A list or tuple *is* the row form — no buffer-protocol object is one — so
/// its own extraction error, which names the element that cannot be a sample,
/// is reported; the buffer error would describe a form the caller never used
/// and send it looking in the wrong place.
fn pcm_from_argument(sample_rate: i64, arg: &Bound<'_, PyAny>) -> PyResult<Pcm16> {
    match arg.extract::<Vec<Vec<i64>>>() {
        Ok(rows) => pcm_from_rows(sample_rate, rows),
        Err(row_error) => match pcm_from_memoryview(sample_rate, arg) {
            Ok(pcm) => Ok(pcm),
            Err(buffer_error) => {
                if arg.is_instance_of::<PyList>() || arg.is_instance_of::<PyTuple>() {
                    Err(row_error)
                } else {
                    Err(buffer_error)
                }
            }
        },
    }
}

fn pcm_from_rows(sample_rate: i64, rows: Vec<Vec<i64>>) -> PyResult<Pcm16> {
    let channels: Vec<Vec<i16>> = rows
        .iter()
        .enumerate()
        .map(|(channel, row)| {
            row.iter()
                .enumerate()
                .map(|(frame, sample)| {
                    i16::try_from(*sample).map_err(|_| {
                        binding_state_error(format!(
                            "PCM channel {channel} frame {frame} sample {sample} \
                             is outside signed-16 range"
                        ))
                    })
                })
                .collect::<PyResult<Vec<i16>>>()
        })
        .collect::<PyResult<Vec<Vec<i16>>>>()?;
    Pcm16::new(sample_rate, channels).map_err(error_to_pyerr)
}

/// A 2-D signed-16 memoryview argument, normalized through Python's own
/// `memoryview()` so any buffer-protocol object is accepted.
///
/// The abi3-py310 limited API does not expose the buffer protocol before
/// 3.11, so validation runs through stable memoryview attributes (`ndim`,
/// `format`, `C_CONTIGUOUS`, `shape`, `tobytes`); this is the single
/// code path for buffer arguments.
fn pcm_from_memoryview(sample_rate: i64, arg: &Bound<'_, PyAny>) -> PyResult<Pcm16> {
    let py = arg.py();
    let locals = PyDict::new(py);
    locals.set_item("arg", arg.clone())?;
    let mv: Bound<'_, PyAny> = py.eval(c_str!("memoryview(arg)"), None, Some(&locals))?;
    let ndim: usize = mv.getattr("ndim")?.extract()?;
    if ndim != 2 {
        return Err(binding_state_error(
            "PCM memoryview must be 2-D (channels, frames)".to_string(),
        ));
    }
    let format: String = mv.getattr("format")?.extract()?;
    if !matches!(format.as_str(), "h" | "=h") {
        return Err(binding_state_error(format!(
            "PCM memoryview must be signed-16 ('h'), got '{format}'"
        )));
    }
    let c_contiguous: bool = mv.getattr("c_contiguous")?.extract()?;
    if !c_contiguous {
        return Err(binding_state_error(
            "PCM memoryview must be C-contiguous".to_string(),
        ));
    }
    let shape: Vec<usize> = mv.getattr("shape")?.extract()?;
    let (channels, frames) = (shape[0], shape[1]);
    // C-order bytes of the whole view; the view keeps the source alive.
    let raw: Vec<u8> = mv.call_method0("tobytes")?.extract()?;
    if raw.len() != channels * frames * 2 {
        return Err(binding_state_error(
            "PCM memoryview byte length disagrees with its shape".to_string(),
        ));
    }
    // The view is C-contiguous, so each channel is one contiguous run of the
    // raw bytes — literally the channel-major layout the kernel accepts
    // directly (`Pcm16::from_channel_major_le`). The bytes go in as they
    // arrived: no i16 row structure is built here, and the single decode
    // happens inside `Pcm16::to_float_rows` on the way to the analysis
    // boundary.
    //
    // The remaining `tobytes()` copy is the floor here: `pyo3::buffer`
    // (and with it a borrowed, zero-copy view) is compiled out under the
    // `abi3-py310` limited API, where the buffer protocol only becomes
    // available at 3.11. Reading the buffer without a copy therefore needs
    // either an abi3 floor of 3.11 or a kernel input type that borrows
    // bytes instead of owning them. What it is not is a second copy: `raw`
    // is handed to the kernel by value, so the bytes move into the input
    // type's storage instead of being duplicated into it.
    Pcm16::from_channel_major_le(sample_rate, channels, raw).map_err(error_to_pyerr)
}

// ---------------------------------------------------------------------------
// Structured profile selection (Wwise generation + PCM geometry)
// ---------------------------------------------------------------------------

/// One kernel selection error as a Python `ValueError`: the message is the
/// kernel diagnostic (`ProfileError` Display, zero drift), exactly as the
/// encoder path maps it onto `EncoderError` codes.
fn selection_value_error(error: impl std::fmt::Display) -> PyErr {
    PyValueError::new_err(error.to_string())
}

/// Wwise generation selector (kernel `WwiseVersion`; C ABI `WemVersion`).
///
/// Codes are stable and append-only: a new Wwise generation appends a
/// variant and a code, it never renumbers or reuses one.
#[pyclass(name = "WwiseVersion", module = "wwise_wem._core", eq, hash, frozen)]
#[derive(Clone, Copy, PartialEq, Eq, Hash)]
enum PyWwiseVersion {
    /// Wwise 2013.2 (kernel code 0, the code the C ABI spells
    /// `WEM_WWISE_2013`).
    #[pyo3(name = "WWISE2013")]
    Wwise2013,
}

impl PyWwiseVersion {
    /// The kernel selector this variant stands for.
    const fn to_kernel(self) -> WwiseVersion {
        match self {
            PyWwiseVersion::Wwise2013 => WwiseVersion::Wwise2013,
        }
    }

    /// The variant standing for one kernel selector.
    const fn from_kernel(version: WwiseVersion) -> Self {
        match version {
            WwiseVersion::Wwise2013 => PyWwiseVersion::Wwise2013,
        }
    }

    /// The Python attribute name of one variant (pinned by the surface test
    /// together with the generated `repr`).
    const fn python_name(self) -> &'static str {
        match self {
            PyWwiseVersion::Wwise2013 => "WWISE2013",
        }
    }
}

#[pymethods]
impl PyWwiseVersion {
    /// Every selectable generation, in stable order (kernel
    /// `WwiseVersion::ALL`).
    #[classattr]
    const ALL: [PyWwiseVersion; 1] = [PyWwiseVersion::Wwise2013];

    /// The generation the auto-selection path uses when the caller names
    /// none (kernel `WwiseVersion::DEFAULT`): the generation this revision
    /// installs.
    #[classattr]
    const DEFAULT: PyWwiseVersion = PyWwiseVersion::Wwise2013;

    /// Stable cross-language code (`include/wem.h` `WemVersion`).
    #[getter]
    fn code(&self) -> u32 {
        self.to_kernel().code()
    }

    /// The short label this generation is spelled with on a command line.
    #[getter]
    fn label(&self) -> &'static str {
        self.to_kernel().label()
    }

    /// The profile-key `generation` string this generation resolves against.
    #[getter]
    fn generation(&self) -> &'static str {
        self.to_kernel().generation()
    }

    /// Decode a stable cross-language code (an unrecognized code is a
    /// `ValueError`, never a silent default).
    #[staticmethod]
    fn from_code(code: u32) -> PyResult<Self> {
        WwiseVersion::from_code(code)
            .map(Self::from_kernel)
            .map_err(selection_value_error)
    }

    /// Decode a profile-key `generation` string.
    #[staticmethod]
    fn from_generation(generation: &str) -> PyResult<Self> {
        WwiseVersion::from_generation(generation)
            .map(Self::from_kernel)
            .map_err(selection_value_error)
    }

    /// Decode a user-facing spelling: the short label or the full generation.
    #[staticmethod]
    fn parse(text: &str) -> PyResult<Self> {
        WwiseVersion::parse(text)
            .map(Self::from_kernel)
            .map_err(selection_value_error)
    }
}

/// Structured profile selection (kernel `WwiseProfile`; C ABI `WemProfile`):
/// one Wwise generation plus the PCM geometry to encode.
///
/// This is the only caller-facing profile selector: profile names, profile
/// paths and profile index bytes are kernel-internal addressing. A selection
/// that no installed profile satisfies is rejected when it reaches the
/// kernel, never substituted by a default.
#[pyclass(name = "WwiseProfile", module = "wwise_wem._core", eq, hash, frozen)]
#[derive(Clone, Copy, PartialEq, Eq, Hash)]
struct PyWwiseProfile {
    inner: WwiseProfile,
}

#[pymethods]
impl PyWwiseProfile {
    /// Build a selection; channels and sample rate must be positive.
    #[new]
    fn new(version: PyWwiseVersion, channels: i64, sample_rate: i64) -> PyResult<Self> {
        WwiseProfile::new(version.to_kernel(), channels, sample_rate)
            .map(|inner| Self { inner })
            .map_err(selection_value_error)
    }

    /// The selected Wwise generation.
    #[getter]
    fn version(&self) -> PyWwiseVersion {
        PyWwiseVersion::from_kernel(self.inner.version())
    }

    /// The selected PCM channel count.
    #[getter]
    fn channels(&self) -> i64 {
        self.inner.channels()
    }

    /// The selected PCM sample rate.
    #[getter]
    fn sample_rate(&self) -> i64 {
        self.inner.sample_rate()
    }

    fn __repr__(&self) -> String {
        format!(
            "WwiseProfile(version=WwiseVersion.{}, channels={}, sample_rate={})",
            PyWwiseVersion::from_kernel(self.inner.version()).python_name(),
            self.inner.channels(),
            self.inner.sample_rate(),
        )
    }
}

// ---------------------------------------------------------------------------
// Encoder
// ---------------------------------------------------------------------------

/// One-shot PCM-to-WEM encoder over one installed profile
/// (Python facade: `wwise_wem.application.encoder.Encoder`).
#[pyclass(name = "Encoder", module = "wwise_wem._core")]
struct PyEncoder {
    inner: WemEncoder,
    /// Terminal state of this handle: set when a call has reported `INTERNAL`
    /// (a kernel panic, or a defect the kernel reported), after which no call
    /// may touch the kernel again. Atomic because the handle is shareable —
    /// methods take `&self` and concurrent encodes are allowed, exactly as the
    /// C ABI's `WemEncoder` is documented.
    dead: AtomicBool,
}

impl PyEncoder {
    /// The rejection every call on this handle answers with once a defect has
    /// killed it.
    fn unusable(&self) -> PyErr {
        unusable_error("Encoder")
    }

    fn is_dead(&self) -> bool {
        self.dead.load(Ordering::SeqCst)
    }

    /// Apply one guarded call's verdict to the handle.
    fn settle<T>(&self, guarded: Guarded<T>) -> PyResult<T> {
        if guarded.handle_dead {
            self.dead.store(true, Ordering::SeqCst);
        }
        guarded.outcome
    }
}

#[pymethods]
impl PyEncoder {
    /// Construct the encoder from a structured [`PyWwiseProfile`] selection
    /// (`wem_core::encoder::Encoder::new_with_quality`).
    ///
    /// Profiles are compiled into the kernel. `quality`, when given,
    /// binds the quality factor before assembly; omitted quality keeps the
    /// historical bytes exactly. A selection no installed profile satisfies
    /// leaves [`WemEncoderError`] with code `PROFILE_NOT_FOUND`; any other
    /// `profile` argument is a Python `TypeError`.
    #[new]
    #[pyo3(signature = (profile, quality=None))]
    fn new(profile: PyRef<'_, PyWwiseProfile>, quality: Option<f64>) -> PyResult<Self> {
        // One-shot: a panic here created nothing, so it is reported and the
        // caller may construct an encoder again.
        let guarded = guard(PanicScope::OneShot, "Encoder", || {
            WemEncoder::new_with_quality(profile.inner, quality)
        });
        Ok(Self {
            inner: guarded.outcome?,
            dead: AtomicBool::new(false),
        })
    }

    /// Encode one independent PCM buffer into a complete WEM container.
    ///
    /// `channels` is a list of per-channel int lists, or a 2-D signed-16
    /// memoryview of shape `(channels, frames)` in C order.
    fn encode_pcm(
        &self,
        py: Python<'_>,
        sample_rate: i64,
        channels: &Bound<'_, PyAny>,
    ) -> PyResult<PyEncodeResult> {
        if self.is_dead() {
            return Err(self.unusable());
        }
        let pcm = pcm_from_argument(sample_rate, channels)?;
        // CPU-bound kernel work: release the GIL (the encoder is
        // immutable after construction, so the shared borrow is safe). A
        // panic unwinds through `allow_threads`, whose guard re-acquires the
        // GIL on the way out, so the interpreter is never left without it.
        let guarded = guard(
            PanicScope::Handle,
            "Encoder",
            AssertUnwindSafe(|| py.allow_threads(|| self.inner.encode_pcm(&pcm))),
        );
        Ok(PyEncodeResult {
            inner: self.settle(guarded)?,
        })
    }

    /// Encode interleaved little-endian signed-16 PCM bytes directly.
    fn encode_pcm16_interleaved(
        &self,
        py: Python<'_>,
        sample_rate: i64,
        channel_count: usize,
        data: &Bound<'_, PyBytes>,
    ) -> PyResult<PyEncodeResult> {
        if self.is_dead() {
            return Err(self.unusable());
        }
        let pcm = Pcm16::from_interleaved_le(sample_rate, channel_count, data.as_bytes())
            .map_err(error_to_pyerr)?;
        let guarded = guard(
            PanicScope::Handle,
            "Encoder",
            AssertUnwindSafe(|| py.allow_threads(|| self.inner.encode_pcm(&pcm))),
        );
        Ok(PyEncodeResult {
            inner: self.settle(guarded)?,
        })
    }
}

// ---------------------------------------------------------------------------
// EncodeResult
// ---------------------------------------------------------------------------

/// Immutable encode result (field set follows `wem_core::EncodeStats`
/// plus the container bytes).
///
/// Every field is an observation the caller cannot recompose: `pcm_frames`
/// (a streaming caller may never have counted the frames it pushed) and the
/// packet counts (they require parsing the container). The container's byte
/// length is not among them — it is `len(result.data)` — and neither is a
/// label naming the selected profile, which is the selection the caller
/// itself passed to the constructor.
#[pyclass(name = "EncodeResult", module = "wwise_wem._core")]
#[derive(Clone)]
struct PyEncodeResult {
    inner: WemEncodeResult,
}

#[pymethods]
impl PyEncodeResult {
    #[getter]
    fn data(&self) -> Vec<u8> {
        self.inner.data.clone()
    }

    #[getter]
    fn audio_packets(&self) -> i64 {
        self.inner.stats.audio_packets
    }

    #[getter]
    fn pcm_frames(&self) -> i64 {
        self.inner.stats.pcm_frames
    }

    #[getter]
    fn channels(&self) -> i64 {
        self.inner.stats.channels
    }

    #[getter]
    fn short_packets(&self) -> i64 {
        self.inner.stats.short_packets
    }

    #[getter]
    fn long_packets(&self) -> i64 {
        self.inner.stats.long_packets
    }
}

// ---------------------------------------------------------------------------
// StreamSession (streaming encode lifecycle)
// ---------------------------------------------------------------------------

/// One streaming encode session: `for_selection` (Init) -> `push`* (chunks)
/// -> `finish` (Finish), straight over the kernel's streaming session.
///
/// `push` returns the packets that just completed, in reply-stream order:
/// the first packet ever emitted is the setup packet (seq 0), then audio
/// packets in encoding order. Frames whose decision window is not yet
/// determined are withheld by the kernel and finalized at `finish`.
#[pyclass(name = "StreamSession", module = "wwise_wem._core")]
struct PyStreamSession {
    inner: WemStreamSession,
    next_seq: u32,
    /// Terminal state of this handle: set when a call has reported `INTERNAL`
    /// (a kernel panic, or a defect the kernel reported). The kernel's own
    /// session survives every rejection it reports; a defect is where this
    /// shell stops trusting it.
    dead: bool,
}

impl PyStreamSession {
    fn is_dead(&self) -> bool {
        self.dead
    }

    fn unusable(&self) -> PyErr {
        unusable_error("StreamSession")
    }
}

#[pymethods]
impl PyStreamSession {
    #[new]
    fn new() -> Self {
        Self {
            inner: WemStreamSession::new(),
            next_seq: 0,
            dead: false,
        }
    }

    /// Open a session on a structured `WwiseProfile` selection
    /// (`wem_core::stream::StreamSession::for_selection_quality`): the kernel
    /// resolves the selection, the returned session is already open and the
    /// next call is `push`.
    ///
    /// A selection no installed profile satisfies leaves
    /// [`WemEncoderError`] with code `PROFILE_NOT_FOUND`; `quality`, when
    /// given, binds the quality factor before assembly.
    #[classmethod]
    #[pyo3(signature = (selection, quality=None))]
    fn for_selection(
        _cls: &Bound<'_, PyType>,
        selection: PyRef<'_, PyWwiseProfile>,
        quality: Option<f64>,
    ) -> PyResult<Self> {
        // One-shot: a panic here opened no session.
        let guarded = guard(PanicScope::OneShot, "StreamSession", || {
            WemStreamSession::for_selection_quality(selection.inner, quality)
        });
        Ok(Self {
            inner: guarded.outcome?,
            next_seq: 0,
            dead: false,
        })
    }

    /// Push one chunk of little-endian signed-16 interleaved PCM bytes
    /// and return the packets that just completed.
    ///
    /// A rejection (`GEOMETRY_MISMATCH` for a trailing partial frame, say)
    /// leaves the session usable: it refused that chunk, nothing more. A
    /// defect — `INTERNAL` — kills it, and every later call raises
    /// `STATE_ERROR`.
    fn push(&mut self, py: Python<'_>, chunk: &Bound<'_, PyBytes>) -> PyResult<Vec<PyPacket>> {
        if self.is_dead() {
            return Err(self.unusable());
        }
        // Copy out before releasing the GIL so no Python object pointer
        // crosses the thread boundary.
        let data: Vec<u8> = chunk.as_bytes().to_vec();
        let guarded = guard(
            PanicScope::Handle,
            "StreamSession",
            AssertUnwindSafe(|| py.allow_threads(|| self.inner.push_pcm_chunk(&data))),
        );
        if guarded.handle_dead {
            self.dead = true;
        }
        let kernel_packets = guarded.outcome?;
        let packets = kernel_packets
            .into_iter()
            .map(|packet| {
                let seq = self.next_seq;
                self.next_seq += 1;
                PyPacket {
                    seq,
                    data: packet.data,
                }
            })
            .collect();
        Ok(packets)
    }

    /// Mark the end of the PCM stream and assemble the container
    /// (`Finish`); returns the container bytes.
    fn finish(&mut self, py: Python<'_>) -> PyResult<PyWemComplete> {
        if self.is_dead() {
            return Err(self.unusable());
        }
        let guarded = guard(
            PanicScope::Handle,
            "StreamSession",
            AssertUnwindSafe(|| py.allow_threads(|| self.inner.finish())),
        );
        if guarded.handle_dead {
            self.dead = true;
        }
        let result = guarded.outcome?;
        Ok(PyWemComplete { bytes: result.data })
    }
}

// ---------------------------------------------------------------------------
// Streaming reply types
// ---------------------------------------------------------------------------

/// One emitted packet in reply-stream order (the reply packet).
#[pyclass(name = "Packet", module = "wwise_wem._core")]
#[derive(Clone)]
struct PyPacket {
    #[pyo3(get)]
    seq: u32,
    #[pyo3(get)]
    data: Vec<u8>,
}

/// Terminal container result (the stream's completion value).
///
/// The container bytes and nothing derived from them: its length is
/// `len(complete.bytes)`, and a digest of it is the caller's to compute
/// from those bytes.
#[pyclass(name = "WemComplete", module = "wwise_wem._core")]
#[derive(Clone)]
struct PyWemComplete {
    /// The assembled WEM container bytes.
    #[pyo3(get)]
    bytes: Vec<u8>,
}

// ---------------------------------------------------------------------------
// Decoder (streaming decode lifecycle; include/wem.h section 5)
// ---------------------------------------------------------------------------

/// Frames per PCM block this shell hands back: the delivery size of the C ABI's
/// `pcm_cb` (`PCM_BLOCK_FRAMES` in `crates/wem-capi`), so a Python caller
/// receives the same bounded blocks a C caller is given.
const PCM_BLOCK_FRAMES: usize = 1024;

/// The one-time geometry and setup announcement of a decode session
/// (include/wem.h `WemHeaderCb`), as a returned value instead of a callback.
#[pyclass(name = "DecodedHeader", module = "wwise_wem._core")]
#[derive(Clone)]
struct PyDecodedHeader {
    /// PCM channel count the container declares.
    #[pyo3(get)]
    channels: u32,
    /// PCM sample rate the container declares.
    #[pyo3(get)]
    sample_rate: u32,
    /// `dw_total_pcm_frames`: the frame count the container declares, and so
    /// the number this session delivers if it finishes successfully. Announced
    /// by the kernel, which has already read it to reach this point.
    #[pyo3(get)]
    total_frames: u64,
    /// The setup packet this revision parsed, exactly as the container carried
    /// it.
    #[pyo3(get)]
    setup_packet: Vec<u8>,
}

impl PyDecodedHeader {
    fn from_kernel(header: DecodedHeader) -> Self {
        Self {
            channels: header.channels,
            sample_rate: header.sample_rate,
            total_frames: header.total_frames,
            setup_packet: header.setup_packet,
        }
    }
}

/// What one decode step produced, and how it ended (include/wem.h
/// `wem_decoder_push` / `wem_decoder_finish`).
///
/// The step's output is reported whether or not it was refused — a rejection
/// stops the step at the packet it could not read, and the blocks the earlier
/// packets completed are still real, so a caller receives them instead of
/// having them dropped. That is why the refusal travels *on* the step rather
/// than only as a raised exception: the C ABI delivers this step's samples
/// through `pcm_cb` and then returns the code, and a shell that raised first
/// would lose the prefix the kernel says it delivered.
#[pyclass(name = "DecodeStep", module = "wwise_wem._core")]
struct PyDecodeStep {
    /// The header announcement, on the one step that resolved it.
    #[pyo3(get)]
    header: Option<PyDecodedHeader>,
    /// Interleaved f32 PCM in bounded blocks of at most [`PCM_BLOCK_FRAMES`]
    /// frames each, at ±1.0 full scale.
    #[pyo3(get)]
    pcm: Vec<Vec<f32>>,
    /// Channel count the PCM is interleaved over (`0` before the header is
    /// known).
    #[pyo3(get)]
    channels: u32,
    /// Frames across every block of [`PyDecodeStep::pcm`].
    #[pyo3(get)]
    frames: usize,
    /// The rejection that stopped this step, as the exception it would be
    /// raised as: its `.code` is the stable cross-language class and its
    /// message is the kernel's own diagnostic. `None` when the step consumed
    /// everything it was given. A defect (`INTERNAL`) is never reported here —
    /// its output is not delivered at all, so it is raised instead.
    #[pyo3(get)]
    error: Option<Py<PyAny>>,
}

/// One streaming decode session (include/wem.h `wem_decoder_new` /
/// `wem_decoder_push` / `wem_decoder_finish`): Init -> push* -> Finish ->
/// release, with each step's output *returned* instead of delivered through the
/// C ABI's `header_cb` / `pcm_cb`.
///
/// Single-threaded ownership, exactly as the C ABI's `WemDecoder` documents:
/// one session must not be shared between threads.
#[pyclass(name = "Decoder", module = "wwise_wem._core")]
struct PyDecoder {
    inner: WemDecodeSession,
    /// `Finish` has run: the session is released, not reused, whatever the call
    /// returned (include/wem.h section 5).
    finished: bool,
    /// A defect killed this handle: every later call raises `STATE_ERROR`
    /// without touching the kernel again.
    dead: bool,
    /// The container's declared frame count, taken from the kernel's header
    /// announcement once it fires.
    total_frames: Option<u64>,
}

impl PyDecoder {
    /// The rejection a call after `Finish` answers with.
    fn finished_error(&self) -> PyErr {
        error_with_code(
            CODE_STATE_ERROR,
            "Decoder is finished: a decode session completes exactly once".to_string(),
        )
    }

    /// Apply one guarded call's verdict to the handle, read the declared frame
    /// count off the step that resolved the header, and convert the step.
    fn settle(
        &mut self,
        py: Python<'_>,
        guarded: Guarded<WemDecodeStep>,
    ) -> PyResult<PyDecodeStep> {
        if guarded.handle_dead {
            self.dead = true;
        }
        let step = guarded.outcome?;
        if let Some(header) = step.header.as_ref() {
            self.total_frames = Some(header.total_frames);
        }
        match decode_step(py, step) {
            Ok(py_step) => Ok(py_step),
            Err(error) => {
                // The step's own output was undeliverable, or it carried a
                // defect: the C ABI reports the same situations as
                // `WEM_ERR_INTERNAL` and kills the handle, because a session
                // whose output cannot be interpreted is not one a caller may
                // build on.
                self.dead = true;
                Err(error)
            }
        }
    }
}

#[pymethods]
impl PyDecoder {
    /// Open a decode session (`Init`).
    ///
    /// There is no profile argument and no selection: a WEM is self-describing,
    /// and the configuration is resolved from the container's own geometry,
    /// which arrives through the header announcement on the step that parses
    /// the setup packet.
    #[new]
    fn new() -> Self {
        Self {
            inner: WemDecodeSession::new(),
            finished: false,
            dead: false,
            total_frames: None,
        }
    }

    /// The container's declared frame count (`dwTotalPCMFrames`), readable once
    /// the header announcement has resolved; `None` before that. The kernel
    /// announces it, so no shell reads the container for it.
    #[getter]
    fn total_frames(&self) -> Option<u64> {
        self.total_frames
    }

    /// Push one chunk of WEM bytes (`wem_decoder_push`) and return what it
    /// completed: the header announcement, at most once per session, and the
    /// PCM the packets in this chunk completed.
    ///
    /// Chunk boundaries never affect the emitted samples; an empty chunk is a
    /// no-op. A rejection with any code but `INTERNAL` leaves the session
    /// usable — the bytes it could not read stay pending, and the next call
    /// reports the same rejection again — while `INTERNAL` is terminal for this
    /// handle.
    fn push(&mut self, py: Python<'_>, chunk: &Bound<'_, PyBytes>) -> PyResult<PyDecodeStep> {
        if self.finished {
            return Err(self.finished_error());
        }
        if self.dead {
            return Err(unusable_error("Decoder"));
        }
        // Copy out before releasing the GIL so no Python object pointer
        // crosses the thread boundary.
        let data: Vec<u8> = chunk.as_bytes().to_vec();
        let guarded = guard(
            PanicScope::Handle,
            "Decoder",
            AssertUnwindSafe(|| {
                Ok::<_, DecoderError>(py.allow_threads(|| self.inner.push_bytes(&data)))
            }),
        );
        self.settle(py, guarded)
    }

    /// Mark the end of the WEM bytes and complete the decode
    /// (`wem_decoder_finish`), returning the last frames.
    ///
    /// Terminal whatever it returns: after this call the session is released,
    /// not reused. On success the session has delivered exactly the container's
    /// declared frame count.
    fn finish(&mut self, py: Python<'_>) -> PyResult<PyDecodeStep> {
        if self.finished {
            return Err(self.finished_error());
        }
        if self.dead {
            return Err(unusable_error("Decoder"));
        }
        let guarded = guard(
            PanicScope::Handle,
            "Decoder",
            AssertUnwindSafe(|| Ok::<_, DecoderError>(py.allow_threads(|| self.inner.finish()))),
        );
        // Terminal whatever it returned, like the C ABI's `wem_decoder_finish`.
        self.finished = true;
        self.settle(py, guarded)
    }
}

/// Convert one kernel step into its returned Python form.
///
/// The PCM is handed back in bounded blocks — the delivery shape the C ABI's
/// `pcm_cb` receives — and each block is a whole number of frames. A defect is
/// raised rather than reported on the step, because the C ABI does not deliver
/// a defect's output: the state it left behind is not something a caller may
/// build on.
fn decode_step(py: Python<'_>, step: WemDecodeStep) -> PyResult<PyDecodeStep> {
    let channels = step.channels as usize;
    let frames = step.frames();
    let mut blocks: Vec<Vec<f32>> = Vec::new();
    if !step.pcm.is_empty() {
        if channels == 0 {
            // PCM without a geometry cannot be interpreted, and delivering it
            // under a guessed interleave would be worse than reporting the
            // invariant (the C ABI's `deliver_decode_step` refuses it too).
            return Err(error_with_code(
                CODE_INTERNAL,
                "kernel defect: a decode step delivered PCM without a geometry".to_string(),
            ));
        }
        let mut offset = 0usize;
        while offset < frames {
            let block = (frames - offset).min(PCM_BLOCK_FRAMES);
            blocks.push(step.pcm[offset * channels..(offset + block) * channels].to_vec());
            offset += block;
        }
    }
    let error = match &step.outcome {
        Ok(()) => None,
        Err(error) if code_of_decoder(error) == CODE_INTERNAL => {
            return Err(error.clone().to_pyerr());
        }
        Err(error) => Some(
            error_with_code(code_of_decoder(error), error.to_string())
                .value(py)
                .clone()
                .into_any()
                .unbind(),
        ),
    };
    Ok(PyDecodeStep {
        header: step.header.map(PyDecodedHeader::from_kernel),
        pcm: blocks,
        channels: step.channels,
        frames,
        error,
    })
}

// ---------------------------------------------------------------------------
// Compiled profile tables
// ---------------------------------------------------------------------------

/// The compiled profile tables as one canonical little-endian byte stream
/// (`wem_core::profile_tables_blob`, defined by `wem_profiles::blob`).
///
/// This is the data hand-off, not a second execution path: the kernel owns
/// the profile facts (they are Rust constants in this artifact) and every
/// other language reads them from here instead of carrying a copy. The pure
/// Python reference implementation is the intended consumer — it needs the
/// recorded values to reproduce bytes, and its own code still decides what
/// they mean.
///
/// Floats travel as their IEEE bit patterns; integer tables keep the width the
/// codec reads them at.
#[pyfunction]
fn profile_tables(py: Python<'_>) -> PyResult<Py<PyBytes>> {
    let blob = py.allow_threads(wem_core::profile_tables_blob);
    Ok(PyBytes::new(py, &blob).unbind())
}

// ---------------------------------------------------------------------------
// Module
// ---------------------------------------------------------------------------

#[pymodule]
fn _core(m: &Bound<'_, PyModule>) -> PyResult<()> {
    let py = m.py();
    m.add("WemEncoderError", WemEncoderError::type_object(py))?;
    m.add_function(wrap_pyfunction!(profile_tables, m)?)?;
    m.add_class::<PyWwiseVersion>()?;
    m.add_class::<PyWwiseProfile>()?;
    m.add_class::<PyEncoder>()?;
    m.add_class::<PyStreamSession>()?;
    m.add_class::<PyEncodeResult>()?;
    m.add_class::<PyPacket>()?;
    m.add_class::<PyWemComplete>()?;
    m.add_class::<PyDecodedHeader>()?;
    m.add_class::<PyDecodeStep>()?;
    m.add_class::<PyDecoder>()?;
    Ok(())
}

// ---------------------------------------------------------------------------
// Tests (pyo3 auto-initialize: run under a real embedded interpreter)
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::Path;

    fn repo_root() -> std::path::PathBuf {
        Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../..")
            .canonicalize()
            .expect("repo root resolves")
    }

    fn fixtures_dir() -> std::path::PathBuf {
        repo_root().join("tests/fixtures")
    }

    /// The committed reference container every byte-exact case compares
    /// against. The digest of those bytes is never re-typed here.
    fn reference_wem() -> Vec<u8> {
        std::fs::read(fixtures_dir().join("reference.wem")).expect("reference.wem reads")
    }

    /// Import the module under test (0.25 test pattern: wrap_pymodule).
    fn import_module(py: Python<'_>) -> PyResult<Bound<'_, PyModule>> {
        Ok(pyo3::wrap_pymodule!(_core)(py).into_bound(py))
    }

    /// The fixture profile selection (6 channels / 44100 Hz / Wwise 2013)
    /// as the Python object every entry point now takes.
    fn selection_object<'py>(m: &Bound<'py, PyModule>) -> Bound<'py, PyAny> {
        let version = m
            .getattr("WwiseVersion")
            .expect("WwiseVersion")
            .getattr("WWISE2013")
            .expect("WWISE2013");
        m.getattr("WwiseProfile")
            .expect("WwiseProfile")
            .call1((version, 6i64, 44_100i64))
            .expect("fixture selection builds")
    }

    /// The fixture WAV as interleaved little-endian i16 PCM bytes.
    fn fixture_pcm_bytes() -> (Vec<u8>, i64, usize, usize) {
        let wav = wem_core::usecases::wav::read_pcm16(&fixtures_dir().join("input.wav"))
            .expect("input.wav reads");
        (
            // Owned here: the WAV keeps its own bytes and this tuple is a
            // fixture copy handed to the Python-side calls.
            wav.interleaved_le_bytes().to_vec(),
            wav.sample_rate(),
            wav.channels(),
            wav.frames(),
        )
    }

    #[test]
    fn public_surface_is_complete_and_exception_is_catchable() {
        Python::with_gil(|py| {
            let m = import_module(py).expect("module imports");
            for name in [
                "Encoder",
                "StreamSession",
                "EncodeResult",
                "Packet",
                "WemComplete",
                "WemEncoderError",
                "WwiseVersion",
                "WwiseProfile",
                "Decoder",
                "DecodeStep",
                "DecodedHeader",
            ] {
                m.getattr(name)
                    .unwrap_or_else(|err| panic!("{name} missing; err={err}"));
            }
            // WemEncoderError must derive from Exception and carry .code.
            let globals = PyDict::new(py);
            globals.set_item("m", m.clone()).unwrap();
            py.run(
                c_str!(
                    r#"
exc = m.WemEncoderError("diagnostic message")
assert issubclass(m.WemEncoderError, Exception), "not an exception subclass"
e = None
try:
    raise m.WemEncoderError("x")
except m.WemEncoderError as caught:
    e = caught
assert e is not None, "exception not catchable"
assert str(e) == "x"
try:
    raise m.WemEncoderError("x")
except Exception as caught2:
    assert type(caught2) is m.WemEncoderError
"#
                ),
                Some(&globals),
                None,
            )
            .expect("exception semantics");
        });
    }

    #[test]
    fn version_surface_is_stable_and_parses_user_spellings() {
        Python::with_gil(|py| {
            let m = import_module(py).unwrap();
            let globals = PyDict::new(py);
            globals.set_item("m", m.clone()).unwrap();
            py.run(
                c_str!(
                    r#"
v = m.WwiseVersion.WWISE2013
assert repr(v) == "WwiseVersion.WWISE2013", repr(v)
assert v.code == 0, v.code
assert v.label == "2013", v.label
assert v.generation == "2013.2", v.generation
assert m.WwiseVersion.ALL == [v], m.WwiseVersion.ALL
assert m.WwiseVersion.DEFAULT == v, m.WwiseVersion.DEFAULT
assert m.WwiseVersion.from_code(0) == v
assert m.WwiseVersion.from_generation("2013.2") == v
assert m.WwiseVersion.parse("2013") == v
assert m.WwiseVersion.parse("2013.2") == v
assert hash(m.WwiseVersion.parse("2013")) == hash(v)
for bad in ("2014", "2013.1", "", "wwise2013"):
    try:
        m.WwiseVersion.parse(bad)
    except ValueError as error:
        assert "unsupported Wwise generation" in str(error), (bad, str(error))
    else:
        raise AssertionError(bad)
try:
    m.WwiseVersion.from_code(7)
except ValueError as error:
    assert "unknown Wwise version code 7" in str(error), str(error)
else:
    raise AssertionError("unknown version code accepted")
try:
    m.WwiseVersion.from_generation("2012.1")
except ValueError as error:
    assert "unsupported Wwise generation" in str(error), str(error)
else:
    raise AssertionError("unknown generation accepted")
for wrong in (2013, None, 2013.2):
    try:
        m.WwiseVersion.parse(wrong)
    except TypeError:
        pass
    else:
        raise AssertionError(wrong)
"#
                ),
                Some(&globals),
                None,
            )
            .expect("version selector surface");
        });
    }

    #[test]
    fn profile_surface_rejects_unresolvable_selections_and_bad_values() {
        Python::with_gil(|py| {
            let m = import_module(py).unwrap();
            let globals = PyDict::new(py);
            globals.set_item("m", m.clone()).unwrap();
            py.run(
                c_str!(
                    r#"
v = m.WwiseVersion.WWISE2013
selection = m.WwiseProfile(v, 6, 44100)
assert repr(selection) == (
    "WwiseProfile(version=WwiseVersion.WWISE2013, channels=6, sample_rate=44100)"
), repr(selection)
assert selection.version == v, selection.version
assert selection.channels == 6 and selection.sample_rate == 44100
assert selection == m.WwiseProfile(v, 6, 44100)
assert hash(selection) == hash(m.WwiseProfile(v, 6, 44100))
assert selection != m.WwiseProfile(v, 2, 48000)
assert selection != "WwiseProfile"
for name in ("version", "channels", "sample_rate"):
    try:
        setattr(selection, name, None)
    except AttributeError:
        pass
    else:
        raise AssertionError(name + " is writable")

# Both installed configurations resolve through the kernel.
m.Encoder(m.WwiseProfile(v, 6, 44100))
m.Encoder(m.WwiseProfile(v, 2, 48000))

# A geometry no installed profile satisfies is a structured kernel error.
for channels, rate in ((2, 44100), (6, 48000), (1, 22050)):
    try:
        m.Encoder(m.WwiseProfile(v, channels, rate))
    except m.WemEncoderError as error:
        assert error.code == "PROFILE_NOT_FOUND", (channels, rate, error.code)
    else:
        raise AssertionError((channels, rate))
try:
    m.StreamSession.for_selection(m.WwiseProfile(v, 2, 44100))
except m.WemEncoderError as error:
    assert error.code == "PROFILE_NOT_FOUND", error.code
else:
    raise AssertionError("unresolvable selection opened a session")

# Non-positive geometry and wrong types are Python-level rejections.
for channels, rate in ((0, 44100), (-1, 44100), (6, 0), (6, -44100)):
    try:
        m.WwiseProfile(v, channels, rate)
    except ValueError as error:
        assert "must be positive" in str(error), str(error)
    else:
        raise AssertionError((channels, rate))
for bad_version in ("2013", 0, None):
    try:
        m.WwiseProfile(bad_version, 6, 44100)
    except TypeError:
        pass
    else:
        raise AssertionError(bad_version)
for bad_geometry in ("6", None, 6.5):
    try:
        m.WwiseProfile(v, bad_geometry, 44100)
    except TypeError:
        pass
    else:
        raise AssertionError(bad_geometry)
# A profile name is not a selection: the literal is the input under
# rejection, not a way to mean "the 6ch profile".
for bad_profile in (2013, 6.5, None, v, "wwise2013-6ch-44100"):
    try:
        m.Encoder(bad_profile)
    except TypeError:
        pass
    else:
        raise AssertionError(bad_profile)
"#
                ),
                Some(&globals),
                None,
            )
            .expect("structured selection surface");
        });
    }

    #[test]
    fn structured_selection_encode_is_bit_exact() {
        let (raw, rate, channels, _frames) = fixture_pcm_bytes();
        Python::with_gil(|py| {
            let m = import_module(py).unwrap();
            let globals = PyDict::new(py);
            globals.set_item("m", m.clone()).unwrap();
            globals.set_item("raw", raw).unwrap();
            globals.set_item("rate", rate).unwrap();
            globals.set_item("channels", channels as i64).unwrap();
            globals.set_item("selection", selection_object(&m)).unwrap();
            globals.set_item("expected_bytes", reference_wem()).unwrap();
            py.run(
                c_str!(
                    r#"
import struct
n = len(raw) // 2
values = struct.unpack("<{n}h".format(n=n), raw)
frames = n // channels
rows = [
    [values[frame * channels + c] for frame in range(frames)]
    for c in range(channels)
]
by_selection = m.Encoder(selection).encode_pcm(rate, rows)
assert bytes(by_selection.data) == expected_bytes, "not the reference container"
assert by_selection.audio_packets == 205, by_selection.audio_packets
assert by_selection.pcm_frames == frames, by_selection.pcm_frames
assert by_selection.channels == channels, by_selection.channels
packed = m.Encoder(selection).encode_pcm16_interleaved(rate, channels, raw)
assert bytes(packed.data) == expected_bytes, "not the reference container"
assert bytes(packed.data) == bytes(by_selection.data)
"#
                ),
                Some(&globals),
                None,
            )
            .expect("structured selection encode must be bit-exact");
        });
    }

    #[test]
    fn stream_session_for_selection_is_bit_exact() {
        let (raw, rate, channels, frames) = fixture_pcm_bytes();
        Python::with_gil(|py| {
            let m = import_module(py).unwrap();
            let globals = PyDict::new(py);
            globals.set_item("m", m.clone()).unwrap();
            globals.set_item("raw", raw).unwrap();
            globals.set_item("rate", rate).unwrap();
            globals.set_item("channels", channels as i64).unwrap();
            globals.set_item("frames", frames as i64).unwrap();
            globals.set_item("expected_bytes", reference_wem()).unwrap();
            py.run(
                c_str!(
                    r#"
cuts = [0, 17000, 40500, 70600, 85600, frames]
selection = m.WwiseProfile(m.WwiseVersion.WWISE2013, channels, rate)
session = m.StreamSession.for_selection(selection)
step = 2 * channels
packets = []
for i in range(len(cuts) - 1):
    lo, hi = cuts[i] * step, cuts[i + 1] * step
    packets.extend(session.push(raw[lo:hi]))
complete = session.finish()
assert bytes(complete.bytes) == expected_bytes, "not the reference container"
assert [p.seq for p in packets] == list(range(len(packets))), "seq must be 0..n-1"
"#
                ),
                Some(&globals),
                None,
            )
            .expect("selection streaming encode must be bit-exact");
        });
    }

    #[test]
    fn encoder_unresolvable_selection_maps_to_profile_not_found() {
        Python::with_gil(|py| {
            let m = import_module(py).unwrap();
            let globals = PyDict::new(py);
            globals.set_item("m", m.clone()).unwrap();
            py.run(
                c_str!(
                    r#"
try:
    m.Encoder(m.WwiseProfile(m.WwiseVersion.WWISE2013, 2, 44100))
except m.WemEncoderError as e:
    assert e.code == "PROFILE_NOT_FOUND", e.code
    assert "2ch/44100Hz/2013" in str(e), str(e)
else:
    raise AssertionError("expected WemEncoderError")
"#
                ),
                Some(&globals),
                None,
            )
            .expect("unresolvable selection maps to PROFILE_NOT_FOUND");
        });
    }

    #[test]
    fn encoder_builds_from_both_installed_selections() {
        Python::with_gil(|py| {
            let m = import_module(py).unwrap();
            let globals = PyDict::new(py);
            globals.set_item("m", m.clone()).unwrap();
            py.run(
                c_str!(
                    r#"
v = m.WwiseVersion.WWISE2013
m.Encoder(m.WwiseProfile(v, 6, 44100))
m.Encoder(m.WwiseProfile(v, 2, 48000))
"#
                ),
                Some(&globals),
                None,
            )
            .expect("both installed selections build an encoder");
        });
    }

    #[test]
    fn encoder_encode_pcm_list_of_lists_is_bit_exact() {
        let (raw, rate, channels, _frames) = fixture_pcm_bytes();
        Python::with_gil(|py| {
            let m = import_module(py).unwrap();
            let globals = PyDict::new(py);
            globals.set_item("m", m.clone()).unwrap();
            globals.set_item("raw", raw).unwrap();
            globals.set_item("rate", rate).unwrap();
            globals.set_item("channels", channels as i64).unwrap();
            globals.set_item("selection", selection_object(&m)).unwrap();
            globals.set_item("expected_bytes", reference_wem()).unwrap();
            py.run(
                c_str!(
                    r#"
import struct
n = len(raw) // 2
vals = struct.unpack("<{n}h".format(n=n), raw)
per_channel = list(val for val in vals)
rows = [
    [per_channel[frame * channels + c] for frame in range(n // channels)]
    for c in range(channels)
]
res = m.Encoder(selection).encode_pcm(rate, rows)
assert bytes(res.data) == expected_bytes, "not the reference container"
assert res.audio_packets == 205, res.audio_packets
assert res.short_packets == 77, res.short_packets
assert res.long_packets == 128, res.long_packets
assert res.pcm_frames == n // channels, res.pcm_frames
assert res.channels == channels, res.channels
"#
                ),
                Some(&globals),
                None,
            )
            .expect("list-of-lists encode must be bit-exact");
        });
    }

    #[test]
    fn encoder_encode_pcm_memoryview_is_bit_exact() {
        let (raw, rate, channels, _frames) = fixture_pcm_bytes();
        Python::with_gil(|py| {
            let m = import_module(py).unwrap();
            let globals = PyDict::new(py);
            globals.set_item("m", m.clone()).unwrap();
            globals.set_item("raw", raw).unwrap();
            globals.set_item("rate", rate).unwrap();
            globals.set_item("channels", channels as i64).unwrap();
            globals.set_item("selection", selection_object(&m)).unwrap();
            globals.set_item("expected_bytes", reference_wem()).unwrap();
            py.run(
                c_str!(
                    r#"
import struct
n = len(raw) // 2
vals = struct.unpack('<{n}h'.format(n=n), raw)
frames = n // channels
rows_flat = [vals[f * channels + c]
             for c in range(channels) for f in range(frames)]
cm_bytes = b''.join(struct.pack('<h', v) for v in rows_flat)
mv = memoryview(cm_bytes).cast('h', [channels, frames])
res = m.Encoder(selection).encode_pcm(rate, mv)
assert bytes(res.data) == expected_bytes, "not the reference container"
"#
                ),
                Some(&globals),
                None,
            )
            .expect("memoryview encode must be bit-exact");
        });
    }

    /// The dispatch between the two documented PCM forms reports the error of
    /// the form the argument was written in (`pcm_from_argument`).
    ///
    /// A list of per-channel rows with a bad element is a row-form mistake,
    /// and the extraction error names the element. Reporting the buffer error
    /// instead would tell the caller its argument is not a buffer — a form it
    /// never used — and send it to fix the wrong thing.
    #[test]
    fn pcm_argument_failures_report_the_error_of_the_form_that_was_used() {
        Python::with_gil(|py| {
            // The sources vary per case, so the C strings are built here
            // rather than through `c_str!` (which takes literals).
            let eval = |source: &str| -> Bound<'_, PyAny> {
                let code = std::ffi::CString::new(source).expect("source has no NUL");
                py.eval(code.as_c_str(), None, None)
                    .unwrap_or_else(|error| panic!("{source} evaluates: {error}"))
            };

            // Row form, bad element: the row error, naming the element.
            for (source, element) in [
                ("[[1, 2, 3], [4, 5, 'x']]", "'str'"),
                ("[[1.5, 2.0, 3.0]]", "'float'"),
                ("[[1, 2], [3, None]]", "'NoneType'"),
            ] {
                let argument = eval(source);
                let error = pcm_from_argument(44_100, &argument)
                    .expect_err("a bad element is not a sample");
                let message = error.to_string();
                assert!(
                    !message.contains("memoryview") && !message.contains("bytes-like"),
                    "{source}: the row error must reach the caller, not the buffer \
                     error for a form it never used: {message}"
                );
                assert!(
                    message.contains(element),
                    "{source}: the error must name the offending element {element}: {message}"
                );
            }

            // Buffer form: neither of these is a list, so the buffer
            // diagnostics are the ones that belong to the argument.
            let one_dimensional = eval("b'\\x00\\x00'");
            let error = pcm_from_argument(44_100, &one_dimensional)
                .expect_err("one-dimensional bytes is not a 2-D buffer");
            assert!(
                error.to_string().contains("must be 2-D"),
                "the buffer form keeps its own diagnostic: {error}"
            );

            let not_a_buffer = eval("object()");
            let error = pcm_from_argument(44_100, &not_a_buffer)
                .expect_err("a plain object is neither form");
            assert!(
                !error.to_string().contains("must be 2-D"),
                "a non-sequence must not be reported as a malformed buffer: {error}"
            );

            // Both accepted forms still reach the kernel: the valid row list
            // builds, and so does the valid 2-D signed-16 buffer.
            let rows = pcm_from_argument(44_100, &eval("[[0] * 4096] * 6"))
                .expect("the documented row form still builds");
            assert_eq!(rows.channel_count(), 6);
            assert_eq!(rows.frame_count(), 4096);

            let bytes = vec![0u8; 6 * 4096 * 2];
            let globals = PyDict::new(py);
            globals
                .set_item("arg", pyo3::types::PyBytes::new(py, &bytes))
                .unwrap();
            let view = py
                .eval(
                    c_str!("memoryview(arg).cast('h', [6, 4096])"),
                    None,
                    Some(&globals),
                )
                .expect("the buffer form builds through memoryview");
            let buffered =
                pcm_from_argument(44_100, &view).expect("the documented buffer form still builds");
            assert_eq!(buffered.channel_count(), 6);
            assert_eq!(buffered.frame_count(), 4096);
        });
    }

    #[test]
    fn encoder_encode_pcm16_interleaved_is_bit_exact() {
        let (raw, rate, channels, frames) = fixture_pcm_bytes();
        Python::with_gil(|py| {
            let m = import_module(py).unwrap();
            let encoder = m
                .getattr("Encoder")
                .unwrap()
                .call1((selection_object(&m),))
                .unwrap();
            let result = encoder
                .call_method1(
                    "encode_pcm16_interleaved",
                    (rate, channels, PyBytes::new(py, &raw)),
                )
                .unwrap();
            let data: Vec<u8> = result.getattr("data").unwrap().extract().unwrap();
            let pcm_frames: i64 = result.getattr("pcm_frames").unwrap().extract().unwrap();
            assert_eq!(
                data,
                reference_wem(),
                "the extension's container bytes must be reference.wem"
            );
            assert_eq!(pcm_frames, frames as i64);
        });
    }

    #[test]
    fn encoder_encode_pcm16_interleaved_rejections_are_structured() {
        Python::with_gil(|py| {
            let m = import_module(py).unwrap();
            let globals = PyDict::new(py);
            globals.set_item("m", m.clone()).unwrap();
            globals.set_item("selection", selection_object(&m)).unwrap();
            globals.set_item("max_channels", usize::MAX).unwrap();
            py.run(
                c_str!(
                    r#"
enc = m.Encoder(selection)
for channels, data, code in (
    (0, b"", "STATE_ERROR"),
    (max_channels, b"\0\0", "STATE_ERROR"),
    (max_channels // 2, b"", "STATE_ERROR"),
    (2, b"\0\0\0", "GEOMETRY_MISMATCH"),
):
    try:
        enc.encode_pcm16_interleaved(44100, channels, data)
    except m.WemEncoderError as error:
        assert error.code == code, (channels, error.code)
    else:
        raise AssertionError((channels, code))
"#
                ),
                Some(&globals),
                None,
            )
            .expect("packed PCM rejections remain structured errors");
        });
    }

    #[test]
    fn encoder_geometry_mismatch_maps_to_geometry_mismatch() {
        Python::with_gil(|py| {
            let m = import_module(py).unwrap();
            let globals = PyDict::new(py);
            globals.set_item("m", m.clone()).unwrap();
            globals.set_item("selection", selection_object(&m)).unwrap();
            py.run(
                c_str!(
                    r#"
# 44100 Hz is the installed rate but 2 channels is not the installed
# geometry (the profile is 6 channels).
enc = m.Encoder(selection)
try:
    enc.encode_pcm(44100, [[0] * 32 for _ in range(2)])
except m.WemEncoderError as e:
    assert e.code == "GEOMETRY_MISMATCH", e.code
else:
    raise AssertionError("expected WemEncoderError")
"#
                ),
                Some(&globals),
                None,
            )
            .expect("geometry mismatch maps to GEOMETRY_MISMATCH");
        });
    }

    #[test]
    fn encoder_input_too_short_maps_to_input_too_short() {
        Python::with_gil(|py| {
            let m = import_module(py).unwrap();
            let globals = PyDict::new(py);
            globals.set_item("m", m.clone()).unwrap();
            globals.set_item("selection", selection_object(&m)).unwrap();
            py.run(
                c_str!(
                    r#"
enc = m.Encoder(selection)
try:
    # 6 channels at the installed rate, but below the 4096-frame minimum.
    enc.encode_pcm(44100, [[0] * 100 for _ in range(6)])
except m.WemEncoderError as e:
    assert e.code == "INPUT_TOO_SHORT", e.code
    assert "4096" in str(e), str(e)
else:
    raise AssertionError("expected WemEncoderError")
"#
                ),
                Some(&globals),
                None,
            )
            .expect("short input maps to INPUT_TOO_SHORT");
        });
    }

    #[test]
    fn encoder_out_of_range_sample_maps_to_state_error() {
        Python::with_gil(|py| {
            let m = import_module(py).unwrap();
            let globals = PyDict::new(py);
            globals.set_item("m", m.clone()).unwrap();
            globals.set_item("selection", selection_object(&m)).unwrap();
            py.run(
                c_str!(
                    r#"
enc = m.Encoder(selection)
try:
    enc.encode_pcm(44100, [[70000] * 10 for _ in range(6)])
except m.WemEncoderError as e:
    assert e.code == "STATE_ERROR", e.code
    assert "signed-16" in str(e), str(e)
else:
    raise AssertionError("expected WemEncoderError")
"#
                ),
                Some(&globals),
                None,
            )
            .expect("out-of-range sample maps to STATE_ERROR");
        });
    }

    #[test]
    fn stream_session_five_uneven_chunks_is_bit_exact() {
        let (raw, _rate, channels, frames) = fixture_pcm_bytes();
        // The committed reference container: its setup packet is what the
        // kernel's stream emits as seq 0, and its full bytes are what the
        // case below compares against.
        let reference = reference_wem();
        let parts = wem_container::load_wem_parts_bytes(&reference).expect("wem parses");
        let setup_packet = parts
            .setup_packet
            .expect("reference has a setup packet")
            .to_vec();
        Python::with_gil(|py| {
            let m = import_module(py).unwrap();
            let globals = PyDict::new(py);
            globals.set_item("m", m.clone()).unwrap();
            globals.set_item("raw", raw).unwrap();
            globals.set_item("selection", selection_object(&m)).unwrap();
            globals
                .set_item("expected_bytes", reference.clone())
                .unwrap();
            globals.set_item("setup_packet", setup_packet).unwrap();
            globals.set_item("channels", channels as i64).unwrap();
            globals.set_item("frames", frames as i64).unwrap();
            py.run(
                c_str!(
                    r#"
# Five unequal frame-count chunks (same spirit as the kernel test).
cuts = [0, 17000, 40500, 70600, 85600, frames]
session = m.StreamSession.for_selection(selection)
step = 2 * channels
packets = []
for i in range(len(cuts) - 1):
    lo, hi = cuts[i] * step, cuts[i + 1] * step
    packets.extend(session.push(raw[lo:hi]))
complete = session.finish()
assert bytes(complete.bytes) == expected_bytes, "not the reference container"
assert [p.seq for p in packets] == list(range(len(packets))), "seq must be 0..n-1"
assert len(packets) >= 1
# The setup packet leaves first and equals the reference container's.
assert bytes(packets[0].data) == bytes(setup_packet), "seq 0 must be the setup packet"
"#
                ),
                Some(&globals),
                None,
            )
            .expect("streaming encode must be bit-exact across chunks");
        });
    }

    #[test]
    fn stream_session_lifecycle_violations_are_state_errors() {
        Python::with_gil(|py| {
            let m = import_module(py).unwrap();
            let globals = PyDict::new(py);
            globals.set_item("m", m.clone()).unwrap();
            globals.set_item("selection", selection_object(&m)).unwrap();
            py.run(
                c_str!(
                    r#"
def expect_state_error(label, fn):
    try:
        fn()
    except m.WemEncoderError as e:
        assert e.code == "STATE_ERROR", (label, e.code)
    else:
        raise AssertionError(label + " did not raise STATE_ERROR")

# 1) push before the session is opened
s1 = m.StreamSession()
expect_state_error("push before open", lambda: s1.push(b"\x00" * 12))

# 2) finish before the session is opened
s2 = m.StreamSession()
expect_state_error("finish before open", lambda: s2.finish())

# 3) push after finish
s3 = m.StreamSession.for_selection(selection)
try:
    s3.finish()  # zero frames -> terminal INPUT_TOO_SHORT, session ends
except m.WemEncoderError as e3:
    assert e3.code == "INPUT_TOO_SHORT", e3.code
expect_state_error("push after finish", lambda: s3.push(b"\x00" * 12))
expect_state_error("second finish", lambda: s3.finish())
"#
                ),
                Some(&globals),
                None,
            )
            .expect("lifecycle violations map to STATE_ERROR");
        });
    }

    #[test]
    fn stream_session_unresolvable_selection_is_profile_not_found() {
        Python::with_gil(|py| {
            let m = import_module(py).unwrap();
            let globals = PyDict::new(py);
            globals.set_item("m", m.clone()).unwrap();
            py.run(
                c_str!(
                    r#"
try:
    m.StreamSession.for_selection(
        m.WwiseProfile(m.WwiseVersion.WWISE2013, 2, 44100)
    )
except m.WemEncoderError as e:
    assert e.code == "PROFILE_NOT_FOUND", e.code
    assert "2ch/44100Hz/2013" in str(e), str(e)
else:
    raise AssertionError("expected WemEncoderError")
"#
                ),
                Some(&globals),
                None,
            )
            .expect("unresolvable selection maps to PROFILE_NOT_FOUND");
        });
    }

    #[test]
    fn stream_session_trailing_partial_chunk_is_geometry_mismatch() {
        Python::with_gil(|py| {
            let m = import_module(py).unwrap();
            let globals = PyDict::new(py);
            globals.set_item("m", m.clone()).unwrap();
            globals.set_item("selection", selection_object(&m)).unwrap();
            py.run(
                c_str!(
                    r#"
s = m.StreamSession.for_selection(selection)
try:
    # 6 channels * 2 bytes = 12 bytes/frame; 13 bytes leaves a partial frame.
    s.push(b"\x00" * 13)
except m.WemEncoderError as e:
    assert e.code == "GEOMETRY_MISMATCH", e.code
else:
    raise AssertionError("expected WemEncoderError")
"#
                ),
                Some(&globals),
                None,
            )
            .expect("partial chunk maps to GEOMETRY_MISMATCH");
        });
    }

    /// The panic rule at this boundary, as the mapping every guarded kernel
    /// call applies.
    ///
    /// The mapping is tested directly rather than by panicking inside a kernel
    /// call: forcing that would need a fault-injection switch in `src/`. The
    /// caught panic below is a real one, produced in the test.
    #[test]
    fn guarded_outcome_maps_a_panic_to_internal_and_kills_only_a_handle() {
        Python::with_gil(|py| {
            // A call that returns: nothing is terminal, either scope.
            for scope in [PanicScope::OneShot, PanicScope::Handle] {
                let guarded = guarded_outcome::<u32, EncoderError>(scope, "Encoder", Ok(Ok(7)));
                assert_eq!(guarded.outcome.ok(), Some(7), "{scope:?}");
                assert!(!guarded.handle_dead, "{scope:?}: success kills nothing");
            }

            // A rejection keeps its own code, and none of them is terminal.
            for (error, code) in [
                (
                    EncoderError::GeometryMismatch {
                        message: "trailing partial PCM frame".into(),
                    },
                    CODE_GEOMETRY_MISMATCH,
                ),
                (
                    EncoderError::StateError {
                        message: "sample outside signed-16 range".into(),
                    },
                    CODE_STATE_ERROR,
                ),
                (
                    EncoderError::InputTooShort { want: 4096, got: 1 },
                    CODE_INPUT_TOO_SHORT,
                ),
                (
                    EncoderError::ProfileNotFound {
                        requested: "2ch/44100Hz/2013".into(),
                    },
                    CODE_PROFILE_NOT_FOUND,
                ),
                (
                    EncoderError::FormatUnsupported {
                        message: "float64 memoryview".into(),
                    },
                    CODE_FORMAT_UNSUPPORTED,
                ),
            ] {
                let guarded = guarded_outcome::<u32, EncoderError>(
                    PanicScope::Handle,
                    "Encoder",
                    Ok(Err(error)),
                );
                let pyerr = guarded.outcome.expect_err("a rejection stays a rejection");
                assert_eq!(
                    code_attribute(py, &pyerr),
                    code,
                    "{code} must survive the crossing, not be swallowed"
                );
                assert!(
                    !guarded.handle_dead,
                    "{code} refuses one call; the handle stays usable"
                );
            }

            // A defect — reported by the kernel or raised as a panic — is
            // INTERNAL, and only a handle-carrying entry dies on it.
            for scope in [PanicScope::OneShot, PanicScope::Handle] {
                let terminal = scope == PanicScope::Handle;

                let reported = guarded_outcome::<u32, EncoderError>(
                    scope,
                    "Encoder",
                    Ok(Err(EncoderError::Internal(
                        wem_core::InternalError::Invariant { message: "x" },
                    ))),
                );
                let pyerr = reported.outcome.expect_err("a defect is an error");
                assert_eq!(code_attribute(py, &pyerr), CODE_INTERNAL);
                assert_eq!(reported.handle_dead, terminal, "{scope:?}");

                let panicked = guarded_outcome::<u32, EncoderError>(
                    scope,
                    "Encoder",
                    std::panic::catch_unwind(|| -> Result<u32, EncoderError> {
                        panic!("injected kernel invariant violation")
                    }),
                );
                let pyerr = panicked
                    .outcome
                    .expect_err("a caught panic is raised, never re-raised");
                assert_eq!(
                    code_attribute(py, &pyerr),
                    CODE_INTERNAL,
                    "{scope:?}: a caught panic carries the code the C ABI reports"
                );
                // The message names the consequence for this scope: a
                // one-shot entry created nothing, a handle is unusable.
                let message = pyerr.to_string();
                let expected = match scope {
                    PanicScope::OneShot => "no Encoder was created",
                    PanicScope::Handle => "unusable",
                };
                assert!(
                    message.contains(expected),
                    "{scope:?}: the message must say what the caller lost \
                     ({expected:?}), got {message:?}"
                );
                assert_eq!(panicked.handle_dead, terminal, "{scope:?}");

                // The exception type is the one the facade re-raises and
                // `except Exception` sees: not pyo3's PanicException.
                assert!(
                    pyerr.is_instance_of::<WemEncoderError>(py),
                    "{scope:?}: a caught panic must arrive as WemEncoderError"
                );
                assert!(
                    !pyerr.is_instance_of::<pyo3::exceptions::PyBaseException>(py)
                        || pyerr.is_instance_of::<pyo3::exceptions::PyException>(py),
                    "{scope:?}: the error must stay inside `except Exception`"
                );
            }
        });
    }

    /// The other half of the rule: a handle a defect has killed answers every
    /// later call with the state error, without touching the kernel again.
    #[test]
    fn a_dead_handle_refuses_every_later_call() {
        Python::with_gil(|py| {
            let m = import_module(py).unwrap();
            let selection = selection_object(&m);
            let encoder = m
                .getattr("Encoder")
                .unwrap()
                .call1((selection.clone(),))
                .unwrap();

            // The live handle encodes; then the defect is applied to it.
            let rows = vec![vec![0i64; 4096]; 6];
            let live = encoder
                .call_method1("encode_pcm", (44_100i64, rows.clone()))
                .expect("a live handle encodes");
            assert_eq!(
                live.getattr("pcm_frames")
                    .unwrap()
                    .extract::<i64>()
                    .unwrap(),
                4096
            );
            {
                let handle = encoder
                    .extract::<PyRef<'_, PyEncoder>>()
                    .expect("the Python name is this Rust class");
                handle.dead.store(true, Ordering::SeqCst);
            }
            let pyerr = encoder
                .call_method1("encode_pcm", (44_100i64, rows))
                .expect_err("a dead handle must refuse");
            assert_eq!(code_attribute(py, &pyerr), CODE_STATE_ERROR);
            assert!(
                pyerr.to_string().contains("unusable"),
                "the rejection must say the handle is unusable, got {pyerr}"
            );

            // The same rule for the streaming session.
            let session = m
                .getattr("StreamSession")
                .unwrap()
                .call_method1("for_selection", (selection,))
                .unwrap();
            {
                let mut handle = session
                    .extract::<PyRefMut<'_, PyStreamSession>>()
                    .expect("the Python name is this Rust class");
                handle.dead = true;
            }
            let pyerr = session
                .call_method1("push", (PyBytes::new(py, &[0u8; 12]),))
                .expect_err("a dead session must refuse a push");
            assert_eq!(code_attribute(py, &pyerr), CODE_STATE_ERROR);
            let pyerr = session
                .call_method0("finish")
                .expect_err("a dead session must refuse to finish");
            assert_eq!(code_attribute(py, &pyerr), CODE_STATE_ERROR);
        });
    }

    /// Adversarial input that only exists in Python: a quality factor, a
    /// float64 PCM buffer, samples outside the signed-16 range. Each must
    /// arrive as a typed kernel error — never as a `PanicException` (which
    /// `except Exception` would not even see), and never with a code that says
    /// "defect" about the caller's own input.
    #[test]
    fn adversarial_python_inputs_are_typed_errors_not_panics() {
        Python::with_gil(|py| {
            let m = import_module(py).unwrap();
            let globals = PyDict::new(py);
            globals.set_item("m", m.clone()).unwrap();
            globals.set_item("selection", selection_object(&m)).unwrap();
            py.run(
                c_str!(
                    r#"
import array


def code_of(call, case):
    """The kernel code one rejected input produces; anything else is the
    failure, and it names the case."""
    try:
        call()
    except m.WemEncoderError as error:
        return error.code
    except BaseException as error:  # PanicException included, deliberately
        raise AssertionError(
            case + ": raised " + type(error).__name__ +
            " instead of a typed kernel error: " + str(error)
        )
    raise AssertionError(case + ": the kernel accepted input it must reject")


# A quality factor is caller input: non-finite values are a malformed
# argument, so they carry STATE_ERROR — not INTERNAL, which means the library
# broke an invariant, and not a PanicException.
for quality in (float("nan"), float("inf"), float("-inf")):
    case = "quality " + repr(quality)
    code = code_of(lambda q=quality: m.Encoder(selection, q), case)
    assert code == "STATE_ERROR", (case, code)

enc = m.Encoder(selection)

# Non-finite PCM in a float64 memoryview: the buffer format is refused before
# a single sample is read, and no NaN reaches the kernel.
nan = array.array("d", [float("nan")] * (6 * 4096))
# Two steps: a cast between two non-byte formats is not allowed, so the bytes
# go through the byte view first.
view = memoryview(nan).cast("B").cast("d", [6, 4096])
code = code_of(lambda: enc.encode_pcm(44100, view), "float64 memoryview of NaN")
assert code == "STATE_ERROR", ("float64 memoryview", code)

# Out-of-range samples: rejected by the signed-16 conversion, never wrapped.
for value in (70000, -70000, 2 ** 40, -(2 ** 40), 2 ** 63 - 1):
    case = "sample " + str(value)
    code = code_of(lambda v=value: enc.encode_pcm(44100, [[v] * 4096] * 6), case)
    assert code == "STATE_ERROR", (case, code)

# None of those was a defect: the handle is still the live handle it was, and
# the encode that follows is a normal one.
result = enc.encode_pcm(44100, [[0] * 4096] * 6)
assert result.pcm_frames == 4096, result.pcm_frames
assert result.channels == 6, result.channels
"#
                ),
                Some(&globals),
                None,
            )
            .expect("adversarial Python inputs must stay typed errors");
        });
    }

    /// The `.code` attribute one raised error carries.
    fn code_attribute(py: Python<'_>, error: &PyErr) -> String {
        error
            .value(py)
            .getattr("code")
            .expect("every WemEncoderError carries .code")
            .extract()
            .expect(".code is a string")
    }

    // -----------------------------------------------------------------------
    // Decode surface (include/wem.h section 5)
    // -----------------------------------------------------------------------

    /// The fixture WAV's interleaved source samples at the kernel's own
    /// normalization (`i16 / 32768.0`, the inverse of the encoder's input path).
    fn fixture_source_samples() -> (Vec<f32>, usize) {
        let (raw, _rate, channels, _frames) = fixture_pcm_bytes();
        let samples = raw
            .chunks_exact(2)
            .map(|pair| i16::from_le_bytes([pair[0], pair[1]]) as f32 / 32768.0)
            .collect();
        (samples, channels)
    }

    /// The largest `max(|a - b|)` a reconstruction of `source` may leave, and
    /// the RMS difference: a structural bound on the decode, not a quality
    /// threshold (the kernel's own decode test is where the live, relative
    /// comparison lives).
    fn error_against(decoded: &[f32], source: &[f32]) -> (f32, f64) {
        let shared = decoded.len().min(source.len());
        let mut peak = 0.0f32;
        let mut sum = 0.0f64;
        for index in 0..shared {
            let difference = f64::from(decoded[index] - source[index]);
            peak = peak.max(difference.abs() as f32);
            sum += difference * difference;
        }
        let rms = if shared == 0 {
            0.0
        } else {
            (sum / shared as f64).sqrt()
        };
        (peak, rms)
    }

    /// The setup packet of the committed reference container, read with the
    /// kernel's own container reader (dev-dependency).
    fn reference_setup_packet() -> Vec<u8> {
        let parts =
            wem_container::load_wem_parts_bytes(&reference_wem()).expect("the fixture parses");
        parts
            .setup_packet
            .expect("the reference container carries a setup packet")
    }

    /// Push `wem` through one `_core.Decoder` in `chunk_bytes`-sized chunks and
    /// return the announced header, every PCM block the shell handed back, and
    /// the number of steps that produced PCM.
    fn decode_through_shell<'py>(
        py: Python<'py>,
        module: &Bound<'py, PyModule>,
        wem: &[u8],
        chunk_bytes: usize,
    ) -> (Bound<'py, PyAny>, Vec<Vec<f32>>, usize) {
        let decoder = module
            .getattr("Decoder")
            .expect("Decoder is exported")
            .call0()
            .expect("a decode session opens");
        let mut header: Option<Bound<'py, PyAny>> = None;
        let mut blocks: Vec<Vec<f32>> = Vec::new();
        let mut producing_steps = 0usize;
        let chunk = chunk_bytes.max(1);
        for bytes in wem.chunks(chunk) {
            let step = decoder
                .call_method1("push", (PyBytes::new(py, bytes),))
                .unwrap_or_else(|error| panic!("the session accepts the real WEM: {error}"));
            let step_header = step.getattr("header").expect("DecodeStep.header");
            if !step_header.is_none() {
                assert!(header.is_none(), "the header is announced exactly once");
                header = Some(step_header);
            }
            assert!(
                step.getattr("error").expect("DecodeStep.error").is_none(),
                "the session accepts the real WEM"
            );
            let produced: Vec<Vec<f32>> = step
                .getattr("pcm")
                .expect("DecodeStep.pcm")
                .extract()
                .expect("pcm blocks are lists of floats");
            if !produced.is_empty() {
                producing_steps += 1;
            }
            blocks.extend(produced);
        }
        let step = decoder
            .call_method0("finish")
            .expect("Finish completes the decode");
        assert!(
            step.getattr("error").unwrap().is_none(),
            "Finish completes the decode"
        );
        let produced: Vec<Vec<f32>> = step.getattr("pcm").unwrap().extract().unwrap();
        if !produced.is_empty() {
            producing_steps += 1;
        }
        blocks.extend(produced);
        (
            header.expect("the header was announced"),
            blocks,
            producing_steps,
        )
    }

    /// The decode shell end to end: a real paired-build WEM through
    /// `_core.Decoder`, with the geometry and the declared frame count readable
    /// before the stream is consumed, bounded PCM blocks, exactly the declared
    /// number of frames, and every sample a reconstruction of the WAV the
    /// container was produced from.
    #[test]
    fn decoder_decodes_the_reference_wem_through_the_shell() {
        let wem = reference_wem();
        let setup_packet = reference_setup_packet();
        let (source, source_channels) = fixture_source_samples();
        Python::with_gil(|py| {
            let m = import_module(py).unwrap();
            let decoder = m.getattr("Decoder").unwrap().call0().unwrap();

            // Nothing is known before the header region arrives.
            assert!(
                decoder.getattr("total_frames").unwrap().is_none(),
                "the declared frame count is not readable before the header resolves"
            );

            // One bounded first chunk: the header announcement, the geometry
            // and the declared frame count all arrive before the rest of the
            // stream is pushed.
            let first = wem.len().min(8192);
            let step = decoder
                .call_method1("push", (PyBytes::new(py, &wem[..first]),))
                .expect("the first chunk is accepted");
            let header = step.getattr("header").unwrap();
            assert!(!header.is_none(), "the header announcement arrives");
            assert_eq!(
                header
                    .getattr("channels")
                    .unwrap()
                    .extract::<u32>()
                    .unwrap(),
                6
            );
            assert_eq!(
                header
                    .getattr("sample_rate")
                    .unwrap()
                    .extract::<u32>()
                    .unwrap(),
                44_100
            );
            assert_eq!(
                header
                    .getattr("setup_packet")
                    .unwrap()
                    .extract::<Vec<u8>>()
                    .unwrap(),
                setup_packet,
                "the announced setup packet is the container's own bytes"
            );
            assert_eq!(
                decoder
                    .getattr("total_frames")
                    .unwrap()
                    .extract::<u32>()
                    .unwrap(),
                139_398,
                "the container's dw_total_pcm_frames"
            );

            // The rest of the stream, in bounded chunks, through the same
            // session.
            let mut pcm: Vec<f32> = Vec::new();
            let push = |step: Bound<'_, PyAny>, pcm: &mut Vec<f32>| {
                for block in step
                    .getattr("pcm")
                    .unwrap()
                    .extract::<Vec<Vec<f32>>>()
                    .unwrap()
                {
                    assert!(block.len() % 6 == 0, "a block is a whole number of frames");
                    assert!(
                        block.len() <= PCM_BLOCK_FRAMES * 6,
                        "a block is bounded by the C ABI's delivery size"
                    );
                    pcm.extend_from_slice(&block);
                }
            };
            push(step, &mut pcm);
            for bytes in wem[first..].chunks(4096) {
                let step = decoder
                    .call_method1("push", (PyBytes::new(py, bytes),))
                    .expect("the session accepts the rest of the container");
                push(step, &mut pcm);
            }
            let step = decoder.call_method0("finish").expect("Finish succeeds");
            push(step, &mut pcm);

            assert_eq!(pcm.len() % 6, 0);
            let frames = pcm.len() / 6;
            assert_eq!(frames, 139_398, "exactly the declared frame count");
            assert_eq!(
                frames,
                source.len() / source_channels,
                "the source's own length"
            );

            let (peak, rms) = error_against(&pcm, &source);
            let baseline = source.iter().fold(0.0f32, |peak, s| peak.max(s.abs()));
            println!(
                "shell decode: {frames} frames x 6ch, max|error| {peak:.6e} \
                 ({:.3}% of the source peak {baseline:.6}), rms {rms:.6e}",
                peak / baseline * 100.0
            );
            assert!(
                peak <= 2.0 * baseline,
                "the shell's decode is not a reconstruction of the source: \
                 max|error| {peak} against a source peak of {baseline}"
            );

            // Finish is terminal whatever it returned (include/wem.h).
            let error = decoder
                .call_method1("push", (PyBytes::new(py, &wem[..16]),))
                .expect_err("a finished session refuses a push");
            assert_eq!(code_attribute(py, &error), CODE_STATE_ERROR);
            let error = decoder
                .call_method0("finish")
                .expect_err("Finish completes exactly once");
            assert_eq!(code_attribute(py, &error), CODE_STATE_ERROR);
        });
    }

    /// Chunk boundaries never move a sample, through the shell's own surface:
    /// the same WEM pushed in one chunk and in 997-byte chunks emits the same
    /// samples. (The *blocking* follows the chunking — a step hands back what
    /// its own bytes completed — so the sample stream is what must agree.)
    #[test]
    fn decoder_chunk_boundaries_never_move_a_sample() {
        let wem = reference_wem();
        Python::with_gil(|py| {
            let m = import_module(py).unwrap();
            let (_, whole, _) = decode_through_shell(py, &m, &wem, wem.len());
            let (_, uneven, _) = decode_through_shell(py, &m, &wem, 997);
            let flatten =
                |blocks: Vec<Vec<f32>>| -> Vec<f32> { blocks.into_iter().flatten().collect() };
            assert_eq!(
                flatten(whole),
                flatten(uneven),
                "chunk size 997 changed the samples"
            );
        });
    }

    /// Every decode rejection reaches Python with the class include/wem.h
    /// section 5 documents for it — the same mapping `crates/wem-capi` applies
    /// — and none of them is a `PanicException`.
    ///
    /// The class travels on the step the call returns, because the C ABI
    /// delivers that step's samples and *then* returns the code: a shell that
    /// raised first would drop the prefix the kernel delivered.
    #[test]
    fn decoder_rejections_carry_the_documented_codes() {
        Python::with_gil(|py| {
            let m = import_module(py).unwrap();

            // The rejection one step reports, as the exception it would raise.
            let step_error = |step: &Bound<'_, PyAny>| -> PyErr {
                let error = step.getattr("error").expect("DecodeStep.error");
                assert!(!error.is_none(), "the step was expected to be refused");
                PyErr::from_value(error.downcast_into().expect("an exception instance"))
            };

            // A container that does not parse: the input's own bytes.
            let decoder = m.getattr("Decoder").unwrap().call0().unwrap();
            let step = decoder
                .call_method1("push", (PyBytes::new(py, b"NOTARIFF\x00\x00\x00\x00...."),))
                .expect("a refusal is reported on the step, which is returned");
            assert!(step
                .getattr("pcm")
                .unwrap()
                .extract::<Vec<Vec<f32>>>()
                .unwrap()
                .is_empty());
            let error = step_error(&step);
            assert_eq!(code_attribute(py, &error), CODE_INPUT_MALFORMED);
            assert!(
                error.to_string().contains("container"),
                "the kernel's own diagnostic travels: {error}"
            );

            // A container that parses but names a geometry this build does not
            // hold: unsupported, never malformed. The header region is the
            // only thing changed, so the container still parses.
            let mut patched = reference_wem();
            let fmt_payload = patched
                .windows(4)
                .position(|window| window == b"fmt ")
                .expect("the fixture carries a fmt chunk")
                + 8;
            patched[fmt_payload + 0x02..fmt_payload + 0x04].copy_from_slice(&3u16.to_le_bytes());
            let decoder = m.getattr("Decoder").unwrap().call0().unwrap();
            let step = decoder
                .call_method1("push", (PyBytes::new(py, &patched),))
                .expect("a refusal is reported on the step");
            let error = step_error(&step);
            assert_eq!(code_attribute(py, &error), CODE_FORMAT_UNSUPPORTED);
            assert!(
                error.to_string().contains("3ch/44100Hz"),
                "the kernel names the geometry it could not resolve: {error}"
            );

            // A call outside the lifecycle: STATE_ERROR, as at the C ABI.
            let decoder = m.getattr("Decoder").unwrap().call0().unwrap();
            let step = decoder
                .call_method0("finish")
                .expect("an empty session reports its shortfall on the step");
            assert_eq!(code_attribute(py, &step_error(&step)), CODE_INPUT_MALFORMED);
            let error = decoder
                .call_method1("push", (PyBytes::new(py, b"\x00"),))
                .expect_err("Finish is terminal whatever it returned");
            assert_eq!(code_attribute(py, &error), CODE_STATE_ERROR);

            // The class mapping itself: representative variants of each row,
            // with the exhaustive match in `code_of_decoder` making a new
            // variant a build failure until its class is written.
            for (error, code) in [
                (
                    DecoderError::Container(wem_container::error::ContainerError::NotRiff),
                    CODE_INPUT_MALFORMED,
                ),
                (
                    DecoderError::Truncated {
                        stream_offset: 0,
                        need: "a packet payload",
                    },
                    CODE_INPUT_MALFORMED,
                ),
                (
                    DecoderError::MissingSetup { data_size: 0 },
                    CODE_INPUT_MALFORMED,
                ),
                (
                    DecoderError::BlockSizeMismatch {
                        container: [256, 2048],
                        profile: [128, 1024],
                    },
                    CODE_INPUT_MALFORMED,
                ),
                (
                    DecoderError::NotWwiseVorbis { format_tag: 0 },
                    CODE_FORMAT_UNSUPPORTED,
                ),
                (
                    DecoderError::StateError {
                        message: "x".into(),
                    },
                    CODE_STATE_ERROR,
                ),
                (
                    DecoderError::Internal(wem_core::InternalError::Invariant { message: "x" }),
                    CODE_INTERNAL,
                ),
            ] {
                assert_eq!(code_of_decoder(&error), code, "{error}");
                let pyerr = error.to_pyerr();
                assert_eq!(code_attribute(py, &pyerr), code);
            }
        });
    }

    /// The decode shell's PCM geometry comes from the kernel, never from a
    /// caller: the session opens with no selection at all, and the pcm-bearing
    /// steps report the container's channel count.
    #[test]
    fn decoder_takes_no_selection_and_reports_the_containers_geometry() {
        Python::with_gil(|py| {
            let m = import_module(py).unwrap();
            let decoder = m.getattr("Decoder").unwrap();
            let error = decoder
                .call1((6i64, 44_100i64))
                .expect_err("Decoder takes no selection");
            assert!(
                error.to_string().contains("takes no arguments")
                    || error.to_string().contains("arguments"),
                "the constructor's own arity error: {error}"
            );

            let wem = reference_wem();
            let (_, blocks, producing_steps) = decode_through_shell(py, &m, &wem, 8192);
            assert!(producing_steps > 1, "the stream is delivered in steps");
            assert!(!blocks.is_empty(), "the decode produced PCM");
        });
    }
}
