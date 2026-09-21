//! PyO3 (abi3) native extension for the wem-core WEM encoder kernel.
//!
//! Exposes the module `wwise_wem._core` (imported in-package; the
//! repository-root maturin wheel installs it as `wwise_wem/_core.abi3.so`) — a thin, zero-drift shell over
//! the kernel:
//!
//! * [`Encoder`] — one-shot PCM-to-WEM encode (`wem_core::encoder::Encoder`)
//! * [`StreamSession`] — the core streaming lifecycle
//!   (Init -> chunks* -> Finish; `wem_core::stream::StreamSession`)
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

use pyo3::exceptions::PyException;
use pyo3::ffi::c_str;
use pyo3::prelude::*;
use pyo3::type_object::PyTypeInfo;
use pyo3::types::{PyBytes, PyDict};

use wem_core::encoder::{EncodeResult as WemEncodeResult, Encoder as WemEncoder, Pcm16};
use wem_core::error::EncoderError;
use wem_core::stream::{ProfileRef, StreamPacket, StreamSession as WemStreamSession};

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

/// Map one kernel error to the Python exception: the stable `.code` string
/// plus the kernel's own diagnostic message (zero drift — the message is
/// `EncoderError`'s Display output, untouched).
fn error_to_pyerr(err: EncoderError) -> PyErr {
    let code = match &err {
        EncoderError::ProfileNotFound { .. } => CODE_PROFILE_NOT_FOUND,
        EncoderError::StateError { .. } => CODE_STATE_ERROR,
        EncoderError::GeometryMismatch { .. } => CODE_GEOMETRY_MISMATCH,
        EncoderError::InputTooShort { .. } => CODE_INPUT_TOO_SHORT,
        EncoderError::FormatUnsupported { .. } => CODE_FORMAT_UNSUPPORTED,
        EncoderError::Internal(_) => CODE_INTERNAL,
    };
    let message = err.to_string();
    Python::with_gil(|py| {
        let pyerr = WemEncoderError::new_err(message);
        let instance = pyerr.value(py);
        let _ = instance.setattr("code", code);
        pyerr
    })
}

/// Binding-level rejection that is semantically a kernel STATE_ERROR
/// (malformed input container; the kernel's own StateError Display is
/// used so the error pipeline stays single-sourced).
fn binding_state_error(message: String) -> PyErr {
    error_to_pyerr(EncoderError::StateError { message })
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
fn pcm_from_argument(sample_rate: i64, arg: &Bound<'_, PyAny>) -> PyResult<Pcm16> {
    if let Ok(rows) = arg.extract::<Vec<Vec<i64>>>() {
        return pcm_from_rows(sample_rate, rows);
    }
    pcm_from_memoryview(sample_rate, arg)
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
    // Copy out the channel-major rows (the kernel takes ownership).
    // Zero-copy PCM intake would need a kernel API that borrows the
    // Python buffer directly — deferred to P3-2 (the facade cutover).
    let samples: Vec<i16> = raw
        .chunks_exact(2)
        .map(|pair| i16::from_le_bytes([pair[0], pair[1]]))
        .collect();
    let rows: Vec<Vec<i16>> = (0..channels)
        .map(|c| samples[c * frames..(c + 1) * frames].to_vec())
        .collect();
    Pcm16::new(sample_rate, rows).map_err(error_to_pyerr)
}

// ---------------------------------------------------------------------------
// Encoder
// ---------------------------------------------------------------------------

/// One-shot PCM-to-WEM encoder over one installed profile
/// (Python facade: `wwise_wem.application.encoder.Encoder`).
#[pyclass(name = "Encoder", module = "wwise_wem._core")]
struct PyEncoder {
    inner: WemEncoder,
}

#[pymethods]
impl PyEncoder {
    #[new]
    #[pyo3(signature = (profile_name, quality=None))]
    fn new(profile_name: &str, quality: Option<f64>) -> PyResult<Self> {
        // Profiles are compiled into the kernel. `quality`, when given,
        // binds the quality factor before assembly; omitted quality keeps the
        // historical bytes exactly.
        let inner =
            WemEncoder::from_profile_quality(profile_name, quality).map_err(error_to_pyerr)?;
        Ok(Self { inner })
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
        let pcm = pcm_from_argument(sample_rate, channels)?;
        // CPU-bound kernel work: release the GIL (the encoder is
        // immutable after construction, so the shared borrow is safe).
        let result = py
            .allow_threads(|| self.inner.encode_pcm(&pcm))
            .map_err(error_to_pyerr)?;
        Ok(PyEncodeResult { inner: result })
    }

    /// Encode interleaved little-endian signed-16 PCM bytes directly.
    fn encode_pcm16_interleaved(
        &self,
        py: Python<'_>,
        sample_rate: i64,
        channel_count: usize,
        data: &Bound<'_, PyBytes>,
    ) -> PyResult<PyEncodeResult> {
        let pcm = Pcm16::from_interleaved_le(sample_rate, channel_count, data.as_bytes())
            .map_err(error_to_pyerr)?;
        let result = py
            .allow_threads(|| self.inner.encode_pcm(&pcm))
            .map_err(error_to_pyerr)?;
        Ok(PyEncodeResult { inner: result })
    }
}

// ---------------------------------------------------------------------------
// EncodeResult
// ---------------------------------------------------------------------------

/// Immutable encode result (field set follows `wem_core::EncodeStats`
/// plus the container bytes; `bytes_out` names the stats `bytes`).
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
    fn bytes_out(&self) -> i64 {
        self.inner.stats.bytes
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

    #[getter]
    fn metadata_source(&self) -> String {
        self.inner.stats.metadata_source.clone()
    }

    /// SHA-256 of the encoded bytes, lowercase hex (kernel-computed).
    fn sha256(&self) -> String {
        self.inner.sha256()
    }
}

// ---------------------------------------------------------------------------
// StreamSession (streaming encode lifecycle)
// ---------------------------------------------------------------------------

/// One streaming encode session: `start` (Init) -> `push`* (chunks) ->
/// `finish` (Finish), straight over the kernel's streaming session.
///
/// `push` returns the packets that just completed, in reply-stream order:
/// the first packet ever emitted is the setup packet (seq 0), then audio
/// packets in encoding order. Frames whose decision window is not yet
/// determined are withheld by the kernel and finalized at `finish`.
#[pyclass(name = "StreamSession", module = "wwise_wem._core")]
struct PyStreamSession {
    inner: WemStreamSession,
    next_seq: u32,
}

#[pymethods]
impl PyStreamSession {
    #[new]
    fn new() -> Self {
        Self {
            inner: WemStreamSession::new(),
            next_seq: 0,
        }
    }

    /// Open the session on one installed profile (`Init`).
    ///
    /// `setup_sha256` is a hard identity assertion (lowercase hex);
    /// `name`, when given, is a soft cross-check that must match the
    /// resolved profile name. `quality`, when given, binds the quality
    /// factor to the resolved profile before assembly (the profile
    /// quality-curves are then interpolated); omitted keeps the
    /// historical bytes exactly.
    #[pyo3(signature = (setup_sha256, name=None, quality=None))]
    fn start(
        &mut self,
        setup_sha256: &str,
        name: Option<&str>,
        quality: Option<f64>,
    ) -> PyResult<()> {
        let profile_ref = ProfileRef {
            setup_sha256: setup_sha256.to_string(),
            name: name.map(str::to_string),
        };
        self.inner
            .init_profile_quality(&profile_ref, quality)
            .map_err(error_to_pyerr)
    }

    /// Push one chunk of little-endian signed-16 interleaved PCM bytes
    /// and return the packets that just completed.
    fn push(&mut self, py: Python<'_>, chunk: &Bound<'_, PyBytes>) -> PyResult<Vec<PyPacket>> {
        // Copy out before releasing the GIL so no Python object pointer
        // crosses the thread boundary.
        let data: Vec<u8> = chunk.as_bytes().to_vec();
        let kernel_packets: Vec<StreamPacket> = py
            .allow_threads(|| self.inner.push_pcm_chunk(&data))
            .map_err(error_to_pyerr)?;
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
    /// (`Finish`); returns the container summary.
    fn finish(&mut self, py: Python<'_>) -> PyResult<PyWemComplete> {
        let result = py
            .allow_threads(|| self.inner.finish())
            .map_err(error_to_pyerr)?;
        let sha256 = result.sha256();
        let total_len = result.data.len() as u64;
        Ok(PyWemComplete {
            bytes: result.data,
            sha256,
            total_len,
        })
    }

    /// PCM frames accumulated so far (streaming observability).
    #[getter]
    fn pcm_frames(&self) -> i64 {
        self.inner.pcm_frames()
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

/// Terminal container summary (the stream's completion result).
#[pyclass(name = "WemComplete", module = "wwise_wem._core")]
#[derive(Clone)]
struct PyWemComplete {
    /// The assembled WEM container bytes.
    #[pyo3(get)]
    bytes: Vec<u8>,
    /// SHA-256 of the container bytes, lowercase hex (64 chars).
    #[pyo3(get)]
    sha256: String,
    /// Byte length of the container.
    #[pyo3(get)]
    total_len: u64,
}

// ---------------------------------------------------------------------------
// Module
// ---------------------------------------------------------------------------

#[pymodule]
fn _core(m: &Bound<'_, PyModule>) -> PyResult<()> {
    let py = m.py();
    m.add("WemEncoderError", WemEncoderError::type_object(py))?;
    m.add_class::<PyEncoder>()?;
    m.add_class::<PyStreamSession>()?;
    m.add_class::<PyEncodeResult>()?;
    m.add_class::<PyPacket>()?;
    m.add_class::<PyWemComplete>()?;
    Ok(())
}

// ---------------------------------------------------------------------------
// Tests (pyo3 auto-initialize: run under a real embedded interpreter)
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::Path;

    const PROFILE_NAME: &str = "wwise2013-6ch-44100";
    const SETUP_SHA256: &str = "3ef56cbd6e6a66a5474005db05912624487faa555fb2cdfed130f606b322e4e3";
    const REFERENCE_SHA256: &str =
        "17851d26c6210b85e498ae0452d2562d7b9e2c3e9e795c459656b9c9d8d35247";

    fn repo_root() -> std::path::PathBuf {
        Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../..")
            .canonicalize()
            .expect("repo root resolves")
    }

    fn fixtures_dir() -> std::path::PathBuf {
        repo_root().join("tests/fixtures")
    }

    /// Import the module under test (0.25 test pattern: wrap_pymodule).
    fn import_module(py: Python<'_>) -> PyResult<Bound<'_, PyModule>> {
        Ok(pyo3::wrap_pymodule!(_core)(py).into_bound(py))
    }

    /// The fixture WAV as interleaved little-endian i16 PCM bytes.
    fn fixture_pcm_bytes() -> (Vec<u8>, i64, usize, usize) {
        let wav = wem_core::usecases::wav::read_pcm16(&fixtures_dir().join("input.wav"))
            .expect("input.wav reads");
        (
            wav.interleaved_le_bytes(),
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
            ] {
                m.getattr(name)
                    .unwrap_or_else(|err| panic!("{name} missing; err={err}"));
            }
            // WemEncoderError must derive from Exception and carry .code.
            let globals = PyDict::new(py);
            globals.set_item("m", m).unwrap();
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
    fn encoder_unknown_profile_maps_to_profile_not_found() {
        Python::with_gil(|py| {
            let m = import_module(py).unwrap();
            let globals = PyDict::new(py);
            globals.set_item("m", m).unwrap();
            py.run(
                c_str!(
                    r#"
try:
    m.Encoder("definitely-not-installed")
except m.WemEncoderError as e:
    assert e.code == "PROFILE_NOT_FOUND", e.code
    assert "definitely-not-installed" in str(e), str(e)
else:
    raise AssertionError("expected WemEncoderError")
"#
                ),
                Some(&globals),
                None,
            )
            .expect("unknown profile maps to PROFILE_NOT_FOUND");
        });
    }

    #[test]
    fn encoder_loads_embedded_profile() {
        Python::with_gil(|py| {
            let m = import_module(py).unwrap();
            let globals = PyDict::new(py);
            globals.set_item("m", m).unwrap();
            globals.set_item("profile", PROFILE_NAME).unwrap();
            py.run(
                c_str!(
                    r#"
enc = m.Encoder(profile)
"#
                ),
                Some(&globals),
                None,
            )
            .expect("embedded profile selection");
        });
    }

    #[test]
    fn encoder_encode_pcm_list_of_lists_is_bit_exact() {
        let (raw, rate, channels, _frames) = fixture_pcm_bytes();
        Python::with_gil(|py| {
            let m = import_module(py).unwrap();
            let globals = PyDict::new(py);
            globals.set_item("m", m).unwrap();
            globals.set_item("raw", raw).unwrap();
            globals.set_item("rate", rate).unwrap();
            globals.set_item("channels", channels as i64).unwrap();
            globals.set_item("profile", PROFILE_NAME).unwrap();
            globals.set_item("expected_sha", REFERENCE_SHA256).unwrap();
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
res = m.Encoder(profile).encode_pcm(rate, rows)
assert res.sha256() == expected_sha, res.sha256()
assert len(bytes(res.data)) == res.bytes_out
assert res.audio_packets == 205, res.audio_packets
assert res.short_packets == 77, res.short_packets
assert res.long_packets == 128, res.long_packets
assert res.pcm_frames == n // channels, res.pcm_frames
assert res.channels == channels, res.channels
assert res.metadata_source == "profile:" + profile, res.metadata_source
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
            globals.set_item("m", m).unwrap();
            globals.set_item("raw", raw).unwrap();
            globals.set_item("rate", rate).unwrap();
            globals.set_item("channels", channels as i64).unwrap();
            globals.set_item("profile", PROFILE_NAME).unwrap();
            globals.set_item("expected_sha", REFERENCE_SHA256).unwrap();
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
res = m.Encoder(profile).encode_pcm(rate, mv)
assert res.sha256() == expected_sha, res.sha256()
assert res.bytes_out == len(bytes(res.data))
"#
                ),
                Some(&globals),
                None,
            )
            .expect("memoryview encode must be bit-exact");
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
                .call1((PROFILE_NAME,))
                .unwrap();
            let result = encoder
                .call_method1(
                    "encode_pcm16_interleaved",
                    (rate, channels, PyBytes::new(py, &raw)),
                )
                .unwrap();
            let digest: String = result.call_method0("sha256").unwrap().extract().unwrap();
            let pcm_frames: i64 = result.getattr("pcm_frames").unwrap().extract().unwrap();
            assert_eq!(digest, REFERENCE_SHA256);
            assert_eq!(pcm_frames, frames as i64);
        });
    }

    #[test]
    fn encoder_encode_pcm16_interleaved_rejections_are_structured() {
        Python::with_gil(|py| {
            let m = import_module(py).unwrap();
            let globals = PyDict::new(py);
            globals.set_item("m", m).unwrap();
            globals.set_item("profile", PROFILE_NAME).unwrap();
            globals.set_item("max_channels", usize::MAX).unwrap();
            py.run(
                c_str!(
                    r#"
enc = m.Encoder(profile)
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
            globals.set_item("m", m).unwrap();
            globals.set_item("profile", PROFILE_NAME).unwrap();
            py.run(
                c_str!(
                    r#"
# 44100 Hz is the installed rate but 2 channels is not the installed
# geometry (the profile is 6 channels).
enc = m.Encoder(profile)
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
            globals.set_item("m", m).unwrap();
            globals.set_item("profile", PROFILE_NAME).unwrap();
            py.run(
                c_str!(
                    r#"
enc = m.Encoder(profile)
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
            globals.set_item("m", m).unwrap();
            globals.set_item("profile", PROFILE_NAME).unwrap();
            py.run(
                c_str!(
                    r#"
enc = m.Encoder(profile)
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
        // The reference WEM's setup packet, parsed by the kernel's own
        // container reader (the kernel's stream emits it as seq 0).
        let reference =
            std::fs::read(fixtures_dir().join("reference.wem")).expect("reference reads");
        let parts = wem_container::load_wem_parts_bytes(&reference).expect("wem parses");
        let setup_packet = parts
            .setup_packet
            .expect("reference has a setup packet")
            .to_vec();
        Python::with_gil(|py| {
            let m = import_module(py).unwrap();
            let globals = PyDict::new(py);
            globals.set_item("m", m).unwrap();
            globals.set_item("raw", raw).unwrap();
            globals.set_item("profile", PROFILE_NAME).unwrap();
            globals.set_item("setup_sha", SETUP_SHA256).unwrap();
            globals.set_item("expected_sha", REFERENCE_SHA256).unwrap();
            globals.set_item("setup_packet", setup_packet).unwrap();
            globals.set_item("channels", channels as i64).unwrap();
            globals.set_item("frames", frames as i64).unwrap();
            py.run(
                c_str!(
                    r#"
# Five unequal frame-count chunks (same spirit as the kernel test).
cuts = [0, 17000, 40500, 70600, 85600, frames]
session = m.StreamSession()
session.start(setup_sha, name=profile)
step = 2 * channels
packets = []
for i in range(len(cuts) - 1):
    lo, hi = cuts[i] * step, cuts[i + 1] * step
    packets.extend(session.push(raw[lo:hi]))
complete = session.finish()
assert complete.sha256 == expected_sha, complete.sha256
assert complete.total_len == len(bytes(complete.bytes))
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
            globals.set_item("m", m).unwrap();
            globals.set_item("setup_sha", SETUP_SHA256).unwrap();
            globals.set_item("profile", PROFILE_NAME).unwrap();
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

# 1) push before start
s1 = m.StreamSession()
expect_state_error("push before start", lambda: s1.push(b"\x00" * 12))

# 2) double start
s2 = m.StreamSession()
s2.start(setup_sha)
expect_state_error("second start", lambda: s2.start(setup_sha))

# 3) finish without start
s3 = m.StreamSession()
expect_state_error("finish without start", lambda: s3.finish())

# 4) name soft-check mismatch
s4 = m.StreamSession()
expect_state_error("name mismatch", lambda: s4.start(setup_sha, name="wrong-name"))

# 5) push after finish
s5 = m.StreamSession()
s5.start(setup_sha)
try:
    s5.finish()  # zero frames -> terminal INPUT_TOO_SHORT, session ends
except m.WemEncoderError as e5:
    assert e5.code == "INPUT_TOO_SHORT", e5.code
expect_state_error("push after finish", lambda: s5.push(b"\x00" * 12))
expect_state_error("second finish", lambda: s5.finish())
"#
                ),
                Some(&globals),
                None,
            )
            .expect("lifecycle violations map to STATE_ERROR");
        });
    }

    #[test]
    fn stream_session_unknown_setup_sha_is_profile_not_found() {
        Python::with_gil(|py| {
            let m = import_module(py).unwrap();
            let globals = PyDict::new(py);
            globals.set_item("m", m).unwrap();
            py.run(
                c_str!(
                    r#"
s = m.StreamSession()
try:
    s.start("0" * 64)
except m.WemEncoderError as e:
    assert e.code == "PROFILE_NOT_FOUND", e.code
else:
    raise AssertionError("expected WemEncoderError")
"#
                ),
                Some(&globals),
                None,
            )
            .expect("unknown setup sha maps to PROFILE_NOT_FOUND");
        });
    }

    #[test]
    fn stream_session_trailing_partial_chunk_is_geometry_mismatch() {
        Python::with_gil(|py| {
            let m = import_module(py).unwrap();
            let globals = PyDict::new(py);
            globals.set_item("m", m).unwrap();
            globals.set_item("setup_sha", SETUP_SHA256).unwrap();
            py.run(
                c_str!(
                    r#"
s = m.StreamSession()
s.start(setup_sha)
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
}
