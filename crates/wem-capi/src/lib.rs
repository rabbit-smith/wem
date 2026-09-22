//! wem-capi — the C ABI core surface of the WEM encoder kernel.
//!
//! This crate is the single cross-language integration point: a thin,
//! stable C ABI over `wem-core`. The interface is declared in
//! `include/wem.h` (lifecycle, reply framing, error codes, memory
//! ownership, integration rules); the signatures here mirror it 1:1.
//!
//! Design rules (crates/AGENTS.md, "C ABI surface"):
//! * Zero drift: validation, profile loading, checksums and encoding run
//!   only in the kernel (`wem_core::Encoder`, `wem_core::StreamSession`);
//!   this layer only converts C values to kernel types and kernel errors
//!   to `WemError`.
//! * No unwind across the FFI: every kernel call is wrapped in
//!   `catch_unwind`; a kernel panic becomes `WEM_ERR_INTERNAL`.
//! * Every entry validates its arguments (NULL checks, length checks)
//!   before touching kernel state.
//!
//! The panic rule (normative: include/wem.h, "Panics"). A caught panic means
//! the kernel broke an invariant, never that the caller passed something bad,
//! and it is reported as `WEM_ERR_INTERNAL`. What it costs the caller is
//! decided by the entry, not by the fault:
//! * One-shot entries — `wem_encode_pcm16_interleaved`, `wem_encoder_new`,
//!   `wem_session_new` — carry no state across calls: the failing call is
//!   rejected and the caller may call again.
//! * Handle-carrying entries — `wem_encoder_encode`, `wem_session_push`,
//!   `wem_session_finish`, `wem_decoder_push`, `wem_decoder_finish` — run
//!   inside a handle whose state the panic leaves unknown, so the handle is
//!   dead and every later call on it returns `WEM_ERR_STATE_ERROR` until it is
//!   freed. A rejection carrying any other code (`WEM_ERR_GEOMETRY_MISMATCH`,
//!   `WEM_ERR_INPUT_MALFORMED`, ...) says nothing about the handle: it stays
//!   usable, exactly as the kernel's own session survives it.
//!
//! The decode surface (include/wem.h section 5) mirrors the encode surface
//! with the data direction reversed: `wem_decoder_new` is one-shot like
//! `wem_session_new`, and `wem_decoder_push` / `wem_decoder_finish` carry the
//! handle. `wem_decoder_finish` is terminal whatever it returns, and a
//! callback that returns non-`WEM_OK` aborts the decode — the same two
//! terminal-by-their-own-rule outcomes the encode surface has. What is *not*
//! the same is which kernel call owns the PCM: the kernel session hands back
//! a step's samples and this layer delivers them, so a rejection can deliver
//! the frames the earlier packets completed and still report its code.
//!
//! `catch_unwind` only catches while panics unwind, so this crate must be
//! built with the default `panic = "unwind"` (include/wem.h, "Panics"; the
//! `compile_error!` guard below makes the alternative a build failure).
//!
//! No handle holds a lock, so there is no poisoned-mutex path to answer here:
//! the kernel encoder is `Sync` and needs no interior mutability, the session
//! is single-threaded, and the terminal flags are a `bool` and an
//! `AtomicBool`. If a lock is ever introduced under a handle, poisoning is a
//! defect like any other and must take the same terminal path as a caught
//! panic — mark the handle dead and report the code — never `unwrap()`, which
//! would turn one panic into a second one inside the guard.
//!
//! Thread-safety semantics (normative, mirrored in include/wem.h):
//! * [`WemEncoder`] is shareable: concurrent `wem_encoder_encode` /
//!   `wem_encode_pcm16_interleaved` calls may share one handle.
//! * [`WemSession`] has single-threaded ownership: never share it
//!   between threads (it is intentionally not `Send`).

#[cfg(panic = "abort")]
compile_error!(
    "wem-capi's panic contract requires unwinding: a kernel panic is caught and \
     reported as WEM_ERR_INTERNAL (include/wem.h, \"Panics\"), and under \
     `panic = \"abort\"` catch_unwind silently stops catching, turning every \
     one of them into a process abort. Remove the `panic = \"abort\"` setting \
     from the profile that builds this crate, or build it with the default \
     unwinding strategy."
);

use std::ffi::c_void;
use std::panic::{AssertUnwindSafe, UnwindSafe};
use std::sync::atomic::{AtomicBool, Ordering};

use wem_core::decoder::{DecodeSession, DecodeStep};
use wem_core::encoder::{Encoder, Pcm16};
use wem_core::error::{DecoderError, EncoderError};
use wem_core::stream::StreamSession;
use wem_core::{WwiseProfile, WwiseVersion};

/// ABI revision this crate implements (include/wem.h `WEM_ABI_REVISION`).
///
/// Revision 2 replaced the `profile_name` + `data_dir` argument pair of
/// `wem_encoder_new`, `wem_encode_pcm16_interleaved` and `wem_session_new`
/// with one `const WemProfile *` selection. Revision 3 removed the
/// `out_meta` out-parameter from `wem_session_finish` together with the
/// `WemMeta` struct it filled: a terminal summary of length and digest is
/// either what the client's own write callback already counts or something it
/// computes from the bytes it received. The header and this crate are
/// pinned together by the `header_matches_this_crate` integration test.
pub const ABI_REVISION: u32 = 3;

/// Callback that receives output bytes in blocks: the one-shot container
/// bytes and the terminal streaming container bytes. The `data` pointer
/// is valid only during the call; a non-`WEM_OK` return aborts the encode.
pub type WemWriteFn =
    unsafe extern "C" fn(data: *const u8, len: usize, user_data: *mut c_void) -> WemError;

/// Callback that receives emitted packets in reply order; `seq` starts at
/// 0 (the Vorbis setup packet) and increases monotonically across calls.
pub type WemPacketFn =
    unsafe extern "C" fn(seq: u32, data: *const u8, len: usize, user_data: *mut c_void) -> WemError;

/// Stable error codes (include/wem.h): 1:1 with `EncoderError` variants,
/// append-only, never renumbered. The discriminants are part of the
/// cross-language interface (see the capi_e2e stability test).
#[repr(u32)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WemError {
    /// Success.
    Ok = 0,
    /// No installed profile matches the requested reference.
    ProfileNotFound = 1,
    /// A request arrived outside the documented lifecycle, or an
    /// argument was malformed (NULL where a value is required).
    StateError = 2,
    /// The PCM geometry disagrees with the profile.
    GeometryMismatch = 3,
    /// The PCM stream is shorter than the 4096-frame minimum.
    InputTooShort = 4,
    /// The requested sample layout is not supported by this revision.
    FormatUnsupported = 5,
    /// An internal fault (including a caught kernel panic).
    Internal = 6,
    /// The decode input's own bytes do not parse (decode surface only).
    InputMalformed = 7,
}

impl WemError {
    /// The stable error code of one kernel failure.
    fn from_kernel(error: &EncoderError) -> Self {
        match error {
            EncoderError::ProfileNotFound { .. } => Self::ProfileNotFound,
            EncoderError::StateError { .. } => Self::StateError,
            EncoderError::GeometryMismatch { .. } => Self::GeometryMismatch,
            EncoderError::InputTooShort { .. } => Self::InputTooShort,
            EncoderError::FormatUnsupported { .. } => Self::FormatUnsupported,
            EncoderError::Internal(_) => Self::Internal,
        }
    }

    /// The stable error code of one decode failure (include/wem.h section 5).
    ///
    /// Four classes, one per row of that section: the input's own bytes do not
    /// parse; the container parses but names a configuration this build does
    /// not carry; the call was made outside the lifecycle; or this library
    /// broke an invariant. A variant added to `DecoderError` fails to compile
    /// here until its class is written, which is the point of the exhaustive
    /// match.
    fn from_decoder(error: &DecoderError) -> Self {
        match error {
            DecoderError::Container(_)
            | DecoderError::Setup { .. }
            | DecoderError::SetupPadding { .. }
            | DecoderError::Truncated { .. }
            | DecoderError::MissingSetup { .. }
            | DecoderError::BlockSizeMismatch { .. }
            | DecoderError::Packet { .. }
            | DecoderError::ResidueBitstreamDefect { .. }
            | DecoderError::FrameCountMismatch { .. } => Self::InputMalformed,
            DecoderError::NotWwiseVorbis { .. }
            | DecoderError::ConfigurationUnsupported { .. }
            | DecoderError::SetupNotCarried { .. } => Self::FormatUnsupported,
            DecoderError::StateError { .. } => Self::StateError,
            DecoderError::Floor1 { .. } | DecoderError::Internal(_) => Self::Internal,
        }
    }
}

/// Wwise generation selector (include/wem.h `WemVersion`).
///
/// Codes are stable and append-only, exactly like [`WemError`]: a new Wwise
/// generation appends a variant and a code, it never renumbers or reuses one.
#[repr(u32)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WemVersion {
    /// Wwise 2013.2.
    Wwise2013 = 0,
}

/// Structured profile selection (include/wem.h `WemProfile`): one Wwise
/// generation plus the PCM geometry. This is the whole profile selection of
/// the ABI — no entry accepts a profile name, a profile directory, profile
/// bytes, or an environment variable.
#[repr(C)]
#[derive(Debug, Clone, Copy)]
pub struct WemProfile {
    /// Wwise generation (`WemVersion`).
    pub version: WemVersion,
    /// PCM channel count to encode (must be positive).
    pub channels: i32,
    /// PCM sample rate to encode (must be positive).
    pub sample_rate: i32,
}

impl WemProfile {
    /// Decode one C selection into the kernel's structured selector.
    ///
    /// An unrecognized version code is a value this revision does not
    /// support; a non-positive geometry is a malformed argument. Neither is
    /// silently replaced by a default.
    fn selection(&self) -> Result<WwiseProfile, WemError> {
        let version = WwiseVersion::from_code(self.version as u32)
            .map_err(|_| WemError::FormatUnsupported)?;
        WwiseProfile::new(
            version,
            i64::from(self.channels),
            i64::from(self.sample_rate),
        )
        .map_err(|_| WemError::StateError)
    }
}

/// Borrow one caller-supplied selection, rejecting NULL.
///
/// # Safety
///
/// `profile` must be NULL or point to a readable `WemProfile` of this
/// revision that stays valid for the duration of the call.
unsafe fn read_profile(profile: *const WemProfile) -> Result<WwiseProfile, WemError> {
    if profile.is_null() {
        return Err(WemError::StateError);
    }
    let profile = unsafe { &*profile };
    profile.selection()
}

/// Shareable profile-resolved encoder (include/wem.h `WemEncoder`).
pub struct WemEncoder {
    encoder: Encoder,
    /// Terminal state of this handle (include/wem.h, "Panics"): set when a
    /// call has reported `WEM_ERR_INTERNAL`, after which the state the panic
    /// left behind is not something a caller may build on.
    ///
    /// An atomic instead of a `Mutex<..>`/`bool` because the handle is
    /// shareable: `wem_encoder_encode` takes `*const WemEncoder` — a shared
    /// borrow — so the flag is the one piece of state a call mutates. Sharing
    /// it this way also keeps the handle free of a lock, and a lock that can
    /// be poisoned would need the same terminal mapping as the flag (see
    /// [`WemEncoder::mark_dead`]).
    dead: AtomicBool,
}

impl WemEncoder {
    /// One live handle over a kernel encoder.
    fn new(encoder: Encoder) -> Self {
        Self {
            encoder,
            dead: AtomicBool::new(false),
        }
    }

    /// Whether a call has already reported `WEM_ERR_INTERNAL` on this handle.
    fn is_dead(&self) -> bool {
        self.dead.load(Ordering::SeqCst)
    }

    /// Mark the handle dead after a defect.
    ///
    /// This is the terminal path of include/wem.h, "Panics". Any future
    /// interior state that can fail independently — a poisoned lock, a
    /// half-written buffer — must call this and report `WEM_ERR_INTERNAL`
    /// (and `WEM_ERR_STATE_ERROR` on every later call), never `unwrap()` its
    /// way into a second panic: a second panic inside a guard would abort the
    /// escape a caller was promised, which is exactly the failure mode the
    /// rule exists to remove. There is no lock on this handle today — the
    /// kernel encoder is `Sync` and needs no interior mutability, so
    /// `PoisonError` cannot arise here; this is the path a lock would have to
    /// take if one is ever added.
    fn mark_dead(&self) {
        self.dead.store(true, Ordering::SeqCst);
    }
}

/// The header's shareability promise, machine-checked.
///
/// include/wem.h states normatively: *"WemEncoder is shareable: concurrent
/// encodes may use one handle on different threads"*, and declares the handle
/// as `/* Profile-resolved shareable encoder (concurrent encodes OK). */`.
/// `wem_encoder_encode` takes `*const WemEncoder` and encodes through a
/// `&WemEncoder`, so sharing one handle across threads is sound only while the
/// inner kernel type stays `Sync` — and a handle created on one thread and used
/// on another also needs `Send` (a handle is not `!Send` merely because the
/// header hands it out as a raw pointer).
///
/// A future `Rc`, `RefCell`, or raw-pointer field anywhere under
/// [`Encoder`] would turn that documented FFI invariant into UB with nothing
/// failing. This `const` item is evaluated in every build (not only under
/// `cfg(test)`); the closure body is never called — the bound is checked when
/// the generic function is instantiated.
const _: fn() = || {
    fn assert_send_sync<T: Send + Sync>() {}
    assert_send_sync::<WemEncoder>();
    assert_send_sync::<Encoder>();
};

/// Streaming encode session (include/wem.h `WemSession`):
/// single-threaded ownership; `failed` marks the terminal state.
pub struct WemSession {
    session: StreamSession,
    write_cb: Option<WemWriteFn>,
    packet_cb: Option<WemPacketFn>,
    user_data: *mut c_void,
    /// Next reply-packet sequence number (seq 0 = setup packet).
    next_seq: u32,
    /// Terminal state: set by `finish` (success or error), by a `packet_cb`
    /// that aborts the encode, and by a call that reported
    /// `WEM_ERR_INTERNAL`. Every entry checks it before touching the kernel
    /// session again.
    failed: bool,
}

/// Output block size for the write callback (bounded, no size limit on
/// the whole container).
const WRITE_BLOCK: usize = 65536;

/// Where one entry point sits in the panic rule (include/wem.h, "Panics").
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum PanicScope {
    /// The call owns no state that survives it: a one-shot encode, or the
    /// construction of a handle. A caught panic rejects this call alone and
    /// the caller may call again.
    OneShot,
    /// The call ran inside a handle that outlives it. A caught panic leaves
    /// that handle's state unknown, so the handle is dead afterwards.
    Handle,
}

/// One guarded kernel call, as the entry points report it.
struct Guarded<T> {
    /// What the entry returns.
    outcome: Result<T, WemError>,
    /// Whether the handle the call ran in is dead afterwards. True only for
    /// `WEM_ERR_INTERNAL` inside a [`PanicScope::Handle`] call: the defect
    /// code is what says the state can no longer be trusted, whether the
    /// kernel panicked or reported the fault (`guarded_outcome` below).
    handle_dead: bool,
}

impl<T> Guarded<T> {
    /// A call the boundary rejected before the kernel saw it: the handle is
    /// untouched.
    fn rejected(code: WemError) -> Self {
        Self {
            outcome: Err(code),
            handle_dead: false,
        }
    }

    /// Continue with the guarded value, keeping the terminal verdict: the
    /// callback that delivers output runs after the kernel call, and it may
    /// not resurrect a handle a panic just killed.
    fn and_then<U>(self, next: impl FnOnce(T) -> Result<U, WemError>) -> Guarded<U> {
        Guarded {
            outcome: self.outcome.and_then(next),
            handle_dead: self.handle_dead,
        }
    }
}

impl Guarded<()> {
    /// The code the entry returns for a call that produces no value.
    fn into_code(self) -> WemError {
        match self.outcome {
            Ok(()) => WemError::Ok,
            Err(code) => code,
        }
    }
}

/// The whole panic rule, in one place: what a kernel failure produces for an
/// entry point of the given scope.
///
/// This is the mapping the FFI bodies apply, kept apart from them so it can be
/// tested as a pure function. Driving a real panic through an entry point
/// would need a fault-injection switch inside the kernel, and a switch that
/// can abort an encode is not something to ship in `src/`.
fn guarded_outcome<T>(
    scope: PanicScope,
    caught: std::thread::Result<Result<T, EncoderError>>,
) -> Guarded<T> {
    match caught {
        Ok(Ok(value)) => Guarded {
            outcome: Ok(value),
            handle_dead: false,
        },
        // A rejection the kernel reported: its own stable code, and the
        // handle is only dead if that code is the defect code.
        Ok(Err(error)) => {
            let code = WemError::from_kernel(&error);
            Guarded {
                outcome: Err(code),
                handle_dead: code == WemError::Internal && scope == PanicScope::Handle,
            }
        }
        // A caught panic: include/wem.h pins this as WEM_ERR_INTERNAL, which
        // means a defect and never bad input.
        Err(_) => Guarded {
            outcome: Err(WemError::Internal),
            handle_dead: scope == PanicScope::Handle,
        },
    }
}

/// Wrap one kernel call so a panic cannot cross the FFI.
fn guard<T>(
    scope: PanicScope,
    work: impl FnOnce() -> Result<T, EncoderError> + UnwindSafe,
) -> Guarded<T> {
    guarded_outcome(scope, std::panic::catch_unwind(work))
}

/// Deliver one byte blob through the write callback in bounded blocks.
fn emit_bytes(
    bytes: &[u8],
    cb: Option<WemWriteFn>,
    user_data: *mut c_void,
) -> Result<(), WemError> {
    let Some(cb) = cb else {
        return Err(WemError::StateError);
    };
    for block in bytes.chunks(WRITE_BLOCK) {
        let code = unsafe { cb(block.as_ptr(), block.len(), user_data) };
        if code != WemError::Ok {
            return Err(code);
        }
    }
    Ok(())
}

/// Encode one interleaved PCM buffer with one encoder; deliver the
/// container bytes through the write callback.
///
/// `scope` is the caller's side of the panic rule: the same encode run for a
/// one-shot entry and for a handle differs only in what a caught panic costs.
fn encode_with(
    scope: PanicScope,
    encoder: &Encoder,
    pcm: *const u8,
    frames: usize,
    write_cb: Option<WemWriteFn>,
    user_data: *mut c_void,
) -> Guarded<()> {
    if frames == 0 || pcm.is_null() || write_cb.is_none() {
        // The caller guarantees `frames` complete frames behind `pcm`;
        // NULL/zero arguments are malformed calls, never encodable input.
        return Guarded::rejected(WemError::StateError);
    }
    let channels = encoder.profile().channels() as usize;
    let byte_len = match frames.checked_mul(channels * 2) {
        Some(byte_len) => byte_len,
        None => return Guarded::rejected(WemError::StateError),
    };
    let bytes = unsafe { std::slice::from_raw_parts(pcm, byte_len) };
    // The client's buffer is borrowed for this call only and the kernel owns
    // its input, so this is the ownership boundary where the samples are
    // copied in — a reviewed design decision, not an oversight.
    let pcm16 = match Pcm16::from_interleaved_le(encoder.profile().sample_rate(), channels, bytes) {
        Ok(pcm16) => pcm16,
        Err(error) => return Guarded::rejected(WemError::from_kernel(&error)),
    };
    // CPU-bound kernel work behind the FFI boundary: catch any panic.
    guard(scope, AssertUnwindSafe(move || encoder.encode_pcm(&pcm16)))
        .and_then(|result| emit_bytes(&result.data, write_cb, user_data))
}

// ---------------------------------------------------------------------------
// Encoder handle (shareable)
// ---------------------------------------------------------------------------

/// Resolve one profile selection into a shareable encoder handle
/// (include/wem.h `wem_encoder_new`).
///
/// One-shot: nothing was handed out when this fails, so a caught panic here
/// is retryable, like any other rejection.
///
/// `*out_encoder` is written on every exit: the handle on `WEM_OK`, NULL on
/// every failure — an out-parameter a C caller can always read, and pass to
/// `wem_encoder_free` (include/wem.h, "MEMORY OWNERSHIP"). The one call that
/// cannot write is a NULL `out_encoder` itself, which is rejected.
///
/// # Safety
///
/// - `profile` must be NULL or a readable `WemProfile` valid for the call
///   (NULL is rejected, no UB);
/// - `out_encoder` must not be NULL.
#[no_mangle]
pub unsafe extern "C" fn wem_encoder_new(
    profile: *const WemProfile,
    out_encoder: *mut *mut WemEncoder,
) -> WemError {
    if out_encoder.is_null() {
        return WemError::StateError;
    }
    let selection = match unsafe { read_profile(profile) } {
        Ok(selection) => selection,
        Err(code) => {
            unsafe {
                *out_encoder = std::ptr::null_mut();
            }
            return code;
        }
    };
    let outcome = guard(PanicScope::OneShot, move || Encoder::new(selection));
    match outcome.outcome {
        Ok(encoder) => {
            unsafe {
                *out_encoder = Box::into_raw(Box::new(WemEncoder::new(encoder)));
            }
            WemError::Ok
        }
        Err(code) => {
            unsafe {
                *out_encoder = std::ptr::null_mut();
            }
            code
        }
    }
}

/// Release one encoder handle (include/wem.h `wem_encoder_free`);
/// NULL is a no-op. A dead handle is freed exactly like a live one.
///
/// # Safety
///
/// `encoder` must be NULL or a live handle from `wem_encoder_new`; it
/// must not have been released and must not be in use by another call.
#[no_mangle]
pub unsafe extern "C" fn wem_encoder_free(encoder: *mut WemEncoder) {
    if !encoder.is_null() {
        unsafe {
            drop(Box::from_raw(encoder));
        }
    }
}

/// Encode one interleaved signed-16 PCM buffer with a shareable handle
/// (include/wem.h `wem_encoder_encode`). The container bytes are
/// delivered through `write_cb` in bounded blocks; concurrent calls on
/// different threads sharing the handle are safe.
///
/// Terminal on `WEM_ERR_INTERNAL`: the defect code means the handle's state
/// can no longer be trusted, so this call marks it dead and every later call
/// on it returns `WEM_ERR_STATE_ERROR` until it is freed.
///
/// # Safety
///
/// - `encoder` must be NULL or a live handle (rejected when NULL);
/// - `pcm` must hold at least `frames * channels * 2` bytes (channels
///   from the profile) for the duration of the call;
/// - `write_cb` must be a live callback (or NULL: rejected).
#[no_mangle]
pub unsafe extern "C" fn wem_encoder_encode(
    encoder: *const WemEncoder,
    pcm: *const u8,
    frames: usize,
    write_cb: Option<WemWriteFn>,
    user_data: *mut c_void,
) -> WemError {
    if encoder.is_null() {
        return WemError::StateError;
    }
    let handle = unsafe { &*encoder };
    if handle.is_dead() {
        return WemError::StateError;
    }
    let outcome = encode_with(
        PanicScope::Handle,
        &handle.encoder,
        pcm,
        frames,
        write_cb,
        user_data,
    );
    if outcome.handle_dead {
        handle.mark_dead();
    }
    outcome.into_code()
}

// ---------------------------------------------------------------------------
// One-shot convenience entry
// ---------------------------------------------------------------------------

/// One-shot encode: resolve the selection, encode, deliver the container
/// bytes (include/wem.h `wem_encode_pcm16_interleaved`).
///
/// `pcm` must hold `frames * channels` little-endian signed-16 samples
/// (interleaved, `channels` from the selection). Output goes through
/// `write_cb` in bounded blocks — callback-style output, so there is no
/// container size limit. One-shot: a caught panic rejects this call and the
/// caller may call again.
///
/// # Safety
///
/// - `profile` as in `wem_encoder_new`;
/// - `pcm` must hold at least `frames * channels * 2` bytes for the
///   duration of the call;
/// - `write_cb` must be a live callback (or NULL: rejected).
#[no_mangle]
pub unsafe extern "C" fn wem_encode_pcm16_interleaved(
    profile: *const WemProfile,
    pcm: *const u8,
    frames: usize,
    write_cb: Option<WemWriteFn>,
    user_data: *mut c_void,
) -> WemError {
    let selection = match unsafe { read_profile(profile) } {
        Ok(selection) => selection,
        Err(code) => return code,
    };
    let outcome = guard(PanicScope::OneShot, move || Encoder::new(selection));
    match outcome.outcome {
        Ok(encoder) => encode_with(
            PanicScope::OneShot,
            &encoder,
            pcm,
            frames,
            write_cb,
            user_data,
        )
        .into_code(),
        Err(code) => code,
    }
}

// ---------------------------------------------------------------------------
// Streaming session
// ---------------------------------------------------------------------------

/// Open a streaming session on one profile selection
/// (include/wem.h `wem_session_new`).
///
/// Lifecycle: `wem_session_new` (Init) -> `wem_session_push`* ->
/// `wem_session_finish` (Finish) -> `wem_session_free`. Emitted packets
/// are delivered through `packet_cb` (NULL discards them); the terminal
/// container bytes are delivered through `write_cb` at `finish`
/// (required). The session has single-threaded ownership. One-shot like
/// `wem_encoder_new`: a caught panic here hands out no handle, so the
/// caller may call again.
///
/// `*out_session` is written on every exit: the handle on `WEM_OK`, NULL on
/// every failure — an out-parameter a C caller can always read, and pass to
/// `wem_session_free` (include/wem.h, "MEMORY OWNERSHIP"). The one call that
/// cannot write is a NULL `out_session` itself, which is rejected.
///
/// # Safety
///
/// - `profile` as in `wem_encoder_new`;
/// - `write_cb` must be a live callback (or NULL: rejected); `packet_cb`
///   must be NULL or a live callback;
/// - `out_session` must not be NULL;
/// - the returned session is owned by the calling thread (single-threaded
///   lifetime; never share it).
#[no_mangle]
pub unsafe extern "C" fn wem_session_new(
    profile: *const WemProfile,
    write_cb: Option<WemWriteFn>,
    packet_cb: Option<WemPacketFn>,
    user_data: *mut c_void,
    out_session: *mut *mut WemSession,
) -> WemError {
    if out_session.is_null() {
        return WemError::StateError;
    }
    let selection = match unsafe { read_profile(profile) } {
        Ok(selection) => selection,
        Err(code) => {
            unsafe {
                *out_session = std::ptr::null_mut();
            }
            return code;
        }
    };
    // A session without its write callback cannot deliver the container, so
    // the missing callback is a malformed call — rejected here, never carried
    // into the handle as a state a later call has to unwrap. The out-pointer
    // is cleared first: every failure of this entry leaves `*out_session` NULL
    // (include/wem.h, "MEMORY OWNERSHIP"), including this one.
    let Some(write_cb) = write_cb else {
        unsafe {
            *out_session = std::ptr::null_mut();
        }
        return WemError::StateError;
    };
    let outcome = guard(PanicScope::OneShot, move || {
        StreamSession::for_selection(selection).map(|session| WemSession {
            session,
            write_cb: Some(write_cb),
            packet_cb,
            user_data,
            next_seq: 0,
            failed: false,
        })
    });
    match outcome.outcome {
        Ok(session) => {
            unsafe {
                *out_session = Box::into_raw(Box::new(session));
            }
            WemError::Ok
        }
        Err(code) => {
            unsafe {
                *out_session = std::ptr::null_mut();
            }
            code
        }
    }
}

/// Push one chunk of little-endian signed-16 interleaved PCM bytes
/// (include/wem.h `wem_session_push`) and deliver the packets that just
/// completed through `packet_cb`. Chunk boundaries never affect the
/// output bytes; an empty chunk is a no-op.
///
/// Terminal on `WEM_ERR_INTERNAL` (a defect the kernel panicked on or
/// reported): the session is marked dead, so every later call on it returns
/// `WEM_ERR_STATE_ERROR`. A rejection with any other code — a chunk that
/// carries a trailing partial frame, say — leaves the session usable, exactly
/// as the kernel's own session survives it. A `packet_cb` that returns
/// non-`WEM_OK` aborts the session on purpose.
///
/// # Safety
///
/// - `session` must be NULL or a live handle owned by this thread
///   (rejected when NULL or terminal);
/// - `data` must hold at least `len` bytes for the duration of the call
///   (a NULL `data` with `len > 0` is rejected, not dereferenced);
/// - the callbacks must be callable from this thread.
#[no_mangle]
pub unsafe extern "C" fn wem_session_push(
    session: *mut WemSession,
    data: *const u8,
    len: usize,
) -> WemError {
    if session.is_null() {
        return WemError::StateError;
    }
    if len > 0 && data.is_null() {
        return WemError::StateError;
    }
    // The kernel call borrows the session; the borrow ends with the
    // closure, before the reply callbacks run below.
    let bytes = if len > 0 {
        unsafe { std::slice::from_raw_parts(data, len) }
    } else {
        &[] as &[u8]
    };
    let guarded = {
        let state = unsafe { &mut *session };
        if state.failed {
            return WemError::StateError;
        }
        guard(
            PanicScope::Handle,
            AssertUnwindSafe(move || state.session.push_pcm_chunk(bytes)),
        )
    };
    match guarded.outcome {
        Ok(packets) => {
            let state = unsafe { &mut *session };
            for packet in packets {
                if let Some(cb) = state.packet_cb {
                    let code = unsafe {
                        cb(
                            state.next_seq,
                            packet.data.as_ptr(),
                            packet.data.len(),
                            state.user_data,
                        )
                    };
                    if code != WemError::Ok {
                        state.failed = true;
                        return code;
                    }
                    state.next_seq += 1;
                }
            }
            WemError::Ok
        }
        Err(code) => {
            if guarded.handle_dead {
                unsafe {
                    (*session).failed = true;
                }
            }
            code
        }
    }
}

/// Mark the end of the PCM stream, complete the encode, and deliver the
/// container bytes through the write callback (include/wem.h
/// `wem_session_finish`). Terminal: after this call (regardless of outcome)
/// the session must be released, not used.
///
/// There is no terminal summary to write and no out-parameter to write it
/// through: the container length is what the client's own write callback
/// already counted, and a digest of the bytes that callback received is the
/// client's to compute. The return code is the whole result.
///
/// # Safety
///
/// - `session` must be NULL or a live handle owned by this thread;
/// - the write callback must be callable from this thread.
#[no_mangle]
pub unsafe extern "C" fn wem_session_finish(session: *mut WemSession) -> WemError {
    if session.is_null() {
        return WemError::StateError;
    }
    if unsafe { (*session).failed } {
        return WemError::StateError;
    }
    let guarded = {
        let state = unsafe { &mut *session };
        guard(
            PanicScope::Handle,
            AssertUnwindSafe(move || state.session.finish()),
        )
    };
    match guarded.outcome {
        Ok(result) => {
            let state = unsafe { &mut *session };
            if let Err(code) = emit_bytes(&result.data, state.write_cb, state.user_data) {
                state.failed = true;
                return code;
            }
            state.failed = true;
            WemError::Ok
        }
        Err(code) => {
            unsafe {
                (*session).failed = true;
            }
            code
        }
    }
}

/// Release one streaming session (include/wem.h `wem_session_free`);
/// NULL is a no-op, safe before or after `finish`, and safe on a session a
/// defect has marked dead.
///
/// # Safety
///
/// `session` must be NULL or a live handle owned by this thread; it must
/// not have been released.
#[no_mangle]
pub unsafe extern "C" fn wem_session_free(session: *mut WemSession) {
    if !session.is_null() {
        unsafe {
            drop(Box::from_raw(session));
        }
    }
}

// ---------------------------------------------------------------------------
// Decode surface (include/wem.h section 5)
// ---------------------------------------------------------------------------

/// Callback that receives the one-time header announcement: the PCM geometry,
/// the frame count the container declares, and the setup packet this revision
/// parsed. The pointers are valid only during the call. A non-`WEM_OK` return
/// aborts the decode.
///
/// `total_frames` is announced here rather than left to the caller because the
/// alternative is that every shell locates the container's `fmt` chunk and
/// reads one field itself, which is container layout knowledge in a layer the
/// integration topology keeps free of it. It is the same number the session
/// will deliver if it finishes with `WEM_OK`.
pub type WemHeaderFn = unsafe extern "C" fn(
    channels: u32,
    sample_rate: u32,
    total_frames: u64,
    setup: *const u8,
    setup_len: usize,
    user_data: *mut c_void,
) -> WemError;

/// Callback that receives interleaved f32 PCM at ±1.0 full scale, in bounded
/// blocks. The pointer is valid only during the call. A non-`WEM_OK` return
/// aborts the decode.
pub type WemPcmFn = unsafe extern "C" fn(
    interleaved: *const f32,
    frames: usize,
    user_data: *mut c_void,
) -> WemError;

/// Frames per PCM callback block. The kernel hands back everything a push
/// completed — bounded by the input that push carried, not by the stream —
/// and this layer delivers it in fixed-size blocks so a caller never sees a
/// block whose size depends on how it chunked its input.
const PCM_BLOCK_FRAMES: usize = 1024;

/// Streaming decode session (include/wem.h `WemDecoder`):
/// single-threaded ownership; `failed` marks the terminal state.
pub struct WemDecoder {
    session: DecodeSession,
    header_cb: WemHeaderFn,
    pcm_cb: WemPcmFn,
    user_data: *mut c_void,
    /// Terminal state: set by `finish` (success or error), by a callback that
    /// aborts the decode, and by a call that reported `WEM_ERR_INTERNAL`.
    failed: bool,
}

/// Deliver one decode step's output and turn its outcome into the entry's
/// return code.
///
/// What a step produced is delivered even when the step was refused: the
/// kernel keeps the bytes it could not read and hands back the frames the
/// packets before them completed, and dropping them here would be the silent
/// truncation the decode contract forbids. The one exception is a *defect*
/// (`WEM_ERR_INTERNAL`): it means this library broke an invariant, the state
/// it left behind is not something a caller may build on, so its output is
/// not delivered and the handle is terminal.
fn deliver_decode_step(state: &mut WemDecoder, step: DecodeStep) -> WemError {
    let defect = matches!(&step.outcome, Err(DecoderError::Internal(_)));
    if !defect {
        if let Some(header) = step.header.as_ref() {
            let code = unsafe {
                (state.header_cb)(
                    header.channels,
                    header.sample_rate,
                    header.total_frames,
                    header.setup_packet.as_ptr(),
                    header.setup_packet.len(),
                    state.user_data,
                )
            };
            if code != WemError::Ok {
                state.failed = true;
                return code;
            }
        }
        if !step.pcm.is_empty() {
            if step.channels == 0 {
                // PCM without a geometry cannot be delivered, and delivering
                // it wrongly would be worse than reporting the invariant.
                state.failed = true;
                return WemError::Internal;
            }
            let channels = step.channels as usize;
            let frames = step.pcm.len() / channels;
            let mut offset = 0usize;
            while offset < frames {
                let block = (frames - offset).min(PCM_BLOCK_FRAMES);
                let code = unsafe {
                    (state.pcm_cb)(
                        step.pcm[offset * channels..].as_ptr(),
                        block,
                        state.user_data,
                    )
                };
                if code != WemError::Ok {
                    state.failed = true;
                    return code;
                }
                offset += block;
            }
        }
    }
    match step.outcome {
        Ok(()) => WemError::Ok,
        Err(error) => {
            let code = WemError::from_decoder(&error);
            if code == WemError::Internal {
                state.failed = true;
            }
            code
        }
    }
}

/// Open a streaming decode session (include/wem.h `wem_decoder_new`).
///
/// Lifecycle: `wem_decoder_new` (Init) -> `wem_decoder_push`* ->
/// `wem_decoder_finish` (Finish) -> `wem_decoder_free`. There is no profile
/// argument: a WEM is self-describing, and the geometry arrives through
/// `header_cb` before any PCM. One-shot like `wem_session_new`: a caught panic
/// here hands out no handle, so the caller may call again.
///
/// Both callbacks are required, and a NULL for either is a malformed call
/// (include/wem.h section 5): the PCM cannot be interpreted without the
/// geometry, and the geometry is available nowhere else in the output.
///
/// `*out_decoder` is written on every exit: the handle on `WEM_OK`, NULL on
/// every failure (include/wem.h, "MEMORY OWNERSHIP"). The one call that
/// cannot write is a NULL `out_decoder` itself, which is rejected.
///
/// # Safety
///
/// - `header_cb` and `pcm_cb` must be live callbacks (or NULL: rejected);
/// - `out_decoder` must not be NULL;
/// - the returned decoder is owned by the calling thread (single-threaded
///   lifetime; never share it).
#[no_mangle]
pub unsafe extern "C" fn wem_decoder_new(
    header_cb: Option<WemHeaderFn>,
    pcm_cb: Option<WemPcmFn>,
    user_data: *mut c_void,
    out_decoder: *mut *mut WemDecoder,
) -> WemError {
    if out_decoder.is_null() {
        return WemError::StateError;
    }
    unsafe {
        *out_decoder = std::ptr::null_mut();
    }
    let (Some(header_cb), Some(pcm_cb)) = (header_cb, pcm_cb) else {
        return WemError::StateError;
    };
    let outcome = guard(PanicScope::OneShot, move || {
        Ok(WemDecoder {
            session: DecodeSession::new(),
            header_cb,
            pcm_cb,
            user_data,
            failed: false,
        })
    });
    match outcome.outcome {
        Ok(decoder) => {
            unsafe {
                *out_decoder = Box::into_raw(Box::new(decoder));
            }
            WemError::Ok
        }
        Err(code) => code,
    }
}

/// Push one chunk of WEM bytes (include/wem.h `wem_decoder_push`) and deliver
/// the header announcement (at most once per session) and the PCM the packets
/// in this chunk completed.
///
/// Chunk boundaries never affect the emitted samples; an empty chunk is a
/// no-op. Terminal on `WEM_ERR_INTERNAL`; any other rejection leaves the
/// decoder usable, with the bytes it could not read still pending, so the
/// next call reports the same rejection again rather than skipping them.
///
/// # Safety
///
/// - `decoder` must be NULL or a live handle owned by this thread (rejected
///   when NULL or terminal);
/// - `data` must hold at least `len` bytes for the duration of the call (a
///   NULL `data` with `len > 0` is rejected, not dereferenced);
/// - the callbacks must be callable from this thread.
#[no_mangle]
pub unsafe extern "C" fn wem_decoder_push(
    decoder: *mut WemDecoder,
    data: *const u8,
    len: usize,
) -> WemError {
    if decoder.is_null() {
        return WemError::StateError;
    }
    if len > 0 && data.is_null() {
        return WemError::StateError;
    }
    let bytes = if len > 0 {
        unsafe { std::slice::from_raw_parts(data, len) }
    } else {
        &[] as &[u8]
    };
    let guarded = {
        let state = unsafe { &mut *decoder };
        if state.failed {
            return WemError::StateError;
        }
        guard(
            PanicScope::Handle,
            AssertUnwindSafe(move || Ok(state.session.push_bytes(bytes))),
        )
    };
    match guarded.outcome {
        Ok(step) => deliver_decode_step(unsafe { &mut *decoder }, step),
        Err(code) => {
            if guarded.handle_dead {
                unsafe {
                    (*decoder).failed = true;
                }
            }
            code
        }
    }
}

/// Mark the end of the WEM bytes and complete the decode (include/wem.h
/// `wem_decoder_finish`), delivering the last frames through `pcm_cb`.
/// Terminal: after this call (regardless of outcome) the decoder must be
/// released, not used. On `WEM_OK` the session has delivered exactly the
/// container's `dw_total_pcm_frames` frames. There is no terminal summary and
/// no out-parameter: the frame count is the container's own declaration.
///
/// # Safety
///
/// - `decoder` must be NULL or a live handle owned by this thread;
/// - the callbacks must be callable from this thread.
#[no_mangle]
pub unsafe extern "C" fn wem_decoder_finish(decoder: *mut WemDecoder) -> WemError {
    if decoder.is_null() {
        return WemError::StateError;
    }
    let guarded = {
        let state = unsafe { &mut *decoder };
        if state.failed {
            return WemError::StateError;
        }
        guard(
            PanicScope::Handle,
            AssertUnwindSafe(move || Ok(state.session.finish())),
        )
    };
    let code = match guarded.outcome {
        Ok(step) => deliver_decode_step(unsafe { &mut *decoder }, step),
        Err(code) => code,
    };
    // Terminal whatever it returns, like the encoder's `finish`.
    unsafe {
        (*decoder).failed = true;
    }
    code
}

/// Release one decode session (include/wem.h `wem_decoder_free`); NULL is a
/// no-op, safe before or after `finish`, and safe on a decoder a defect has
/// marked dead.
///
/// # Safety
///
/// `decoder` must be NULL or a live handle owned by this thread; it must not
/// have been released.
#[no_mangle]
pub unsafe extern "C" fn wem_decoder_free(decoder: *mut WemDecoder) {
    if !decoder.is_null() {
        unsafe {
            drop(Box::from_raw(decoder));
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn error_codes_follow_the_wem_h_table() {
        assert_eq!(WemError::Ok as u32, 0);
        assert_eq!(WemError::ProfileNotFound as u32, 1);
        assert_eq!(WemError::StateError as u32, 2);
        assert_eq!(WemError::GeometryMismatch as u32, 3);
        assert_eq!(WemError::InputTooShort as u32, 4);
        assert_eq!(WemError::FormatUnsupported as u32, 5);
        assert_eq!(WemError::Internal as u32, 6);
        assert_eq!(WemError::InputMalformed as u32, 7);
    }

    /// Every `DecoderError` class maps to the code include/wem.h section 5
    /// documents for it. `WemError::from_decoder` matches without a `_` arm,
    /// so a new variant cannot be added without deciding its class; this test
    /// pins the decision for every variant this crate can construct.
    ///
    /// `DecoderError::Floor1` and the `Setup`/`Packet` variants that wrap a
    /// `wem-vorbis` type are not constructible here — `wem-vorbis` is not a
    /// dependency of this crate, and adding one is a manifest change this lane
    /// does not own. `Setup` is still covered, produced by the kernel below;
    /// `Floor1`'s class rests on the exhaustive match.
    #[test]
    fn decoder_errors_map_to_the_documented_classes() {
        use wem_container::error::ContainerError;
        use wem_core::error::InternalError;
        use wem_profiles::error::ProfileError;

        let cases = [
            (
                DecoderError::Container(ContainerError::NotRiff),
                WemError::InputMalformed,
            ),
            (
                DecoderError::SetupPadding {
                    end_bit: 1,
                    total_bits: 2,
                    pad_bits: 1,
                    pad_value: 1,
                },
                WemError::InputMalformed,
            ),
            (
                DecoderError::Truncated {
                    stream_offset: 4,
                    need: "a packet payload",
                },
                WemError::InputMalformed,
            ),
            (
                DecoderError::MissingSetup { data_size: 0 },
                WemError::InputMalformed,
            ),
            (
                DecoderError::BlockSizeMismatch {
                    container: [256, 2048],
                    profile: [256, 1024],
                },
                WemError::InputMalformed,
            ),
            (
                DecoderError::FrameCountMismatch {
                    declared: 10,
                    synthesized: 9,
                },
                WemError::InputMalformed,
            ),
            (
                DecoderError::ResidueBitstreamDefect {
                    index: 0,
                    submap: 0,
                    bit_position: 8,
                    packet_bits: 16,
                },
                WemError::InputMalformed,
            ),
            (
                DecoderError::NotWwiseVorbis { format_tag: 1 },
                WemError::FormatUnsupported,
            ),
            (
                DecoderError::ConfigurationUnsupported {
                    channels: 3,
                    sample_rate: 44_100,
                    source: ProfileError::SelectionGeometryNonPositive,
                },
                WemError::FormatUnsupported,
            ),
            (
                DecoderError::SetupNotCarried {
                    container_len: 1,
                    carried_len: 2,
                    first_difference: Some(0),
                },
                WemError::FormatUnsupported,
            ),
            (
                DecoderError::StateError {
                    message: "x".into(),
                },
                WemError::StateError,
            ),
            (
                DecoderError::Internal(InternalError::Invariant { message: "x" }),
                WemError::Internal,
            ),
        ];
        for (error, expected) in cases {
            assert_eq!(
                WemError::from_decoder(&error),
                expected,
                "{error} must map to {expected:?}"
            );
        }

        // The `Setup` class, produced by the kernel rather than constructed
        // here: a container whose setup packet is empty parses its framing and
        // then fails inside the setup packet.
        let encoder = Encoder::new(
            WwiseProfile::new(WwiseVersion::Wwise2013, 6, 44_100).expect("installed geometry"),
        )
        .expect("the profile resolves");
        let mut fields = *encoder.container_plan().fmt();
        fields.dw_seek_table_size = 0;
        let container = wem_container::riff::build_riff(
            &[
                (b"fmt " as &[u8], fields.pack().as_slice()),
                (b"data" as &[u8], [0u8, 0u8].as_slice()),
            ],
            wem_container::riff::Endian::Little,
            false,
        )
        .expect("the container assembles");
        let mut session = DecodeSession::new();
        let error = session
            .push_bytes(&container)
            .outcome
            .expect_err("an empty setup packet is refused");
        assert!(
            matches!(error, DecoderError::Setup { .. }),
            "expected a setup parse failure, got {error}"
        );
        assert_eq!(WemError::from_decoder(&error), WemError::InputMalformed);
    }

    #[test]
    fn version_codes_follow_the_wem_h_table() {
        assert_eq!(WemVersion::Wwise2013 as u32, 0);
    }

    /// The header is the interface, so the crate must not drift
    /// from it: the declared revision has to match, no declaration may
    /// reintroduce a profile name or a profile directory (ABI revision 2
    /// replaced both with the `WemProfile` selection), no entry may grow back
    /// a terminal summary out-parameter (ABI revision 3 removed
    /// `wem_session_finish`'s), and the two rules a caller can only learn from
    /// the header — what every exit writes through an out-parameter, and what
    /// `WEM_ERR_STATE_ERROR` covers — must still be stated there.
    #[test]
    fn header_matches_this_crate() {
        let header = std::fs::read_to_string(
            std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../include/wem.h"),
        )
        .expect("include/wem.h reads");

        assert!(
            header.contains(&format!("#define WEM_ABI_REVISION {ABI_REVISION}")),
            "include/wem.h must declare WEM_ABI_REVISION {ABI_REVISION}"
        );
        assert!(
            header.contains("} WemProfile;"),
            "WemProfile is in the header"
        );
        assert!(
            header.contains("WEM_WWISE_2013 = 0"),
            "version table is in the header"
        );

        // The two rules a C caller cannot derive from the prototypes and must
        // find in the header: what every exit writes through an out-parameter,
        // and what WEM_ERR_STATE_ERROR does and does not distinguish. Both are
        // implemented here, so the text is part of the interface this crate is
        // pinned to, not commentary free to drift away from the code.
        for (text, rule) in [
            (
                "OUT-PARAMETERS: WHAT EVERY EXIT WRITES",
                "the out-parameter rule (handle out-pointers written on every exit)",
            ),
            (
                "One code, three situations: WEM_ERR_STATE_ERROR",
                "the WEM_ERR_STATE_ERROR situations and their single recovery",
            ),
        ] {
            assert!(
                header.contains(text),
                "include/wem.h must state {rule}; it no longer contains {text:?}"
            );
        }

        // Check the declarations only: the header's evolution note and its
        // migration appendix name the removed arguments and the removed
        // terminal summary on purpose, so strip comments before looking.
        let declarations = strip_c_comments(&header);
        assert!(
            declarations.contains("wem_session_finish(WemSession *session)"),
            "include/wem.h must declare the revision 3 finish signature, \
             which takes the session alone"
        );
        // A declaration that creeps back in would be a revision nobody bumped:
        // revision 2 removed the profile name and the data directory,
        // revision 3 removed the terminal summary and its out-parameter.
        for removed in ["profile_name", "data_dir", "WemMeta", "out_meta"] {
            assert!(
                !declarations.contains(removed),
                "include/wem.h declares {removed:?} again"
            );
        }
    }

    /// Remove `/* ... */` and `// ...` comments so a check can look at the
    /// declarations alone.
    fn strip_c_comments(source: &str) -> String {
        let chars: Vec<char> = source.chars().collect();
        let mut out = String::with_capacity(source.len());
        let mut i = 0usize;
        while i < chars.len() {
            let at = |n: usize| chars.get(n).copied();
            if chars[i] == '/' && at(i + 1) == Some('*') {
                i += 2;
                while i < chars.len() && !(chars[i] == '*' && at(i + 1) == Some('/')) {
                    i += 1;
                }
                i += 2;
            } else if chars[i] == '/' && at(i + 1) == Some('/') {
                while i < chars.len() && chars[i] != '\n' {
                    i += 1;
                }
            } else {
                out.push(chars[i]);
                i += 1;
            }
        }
        out
    }

    #[test]
    fn profile_layout_matches_wem_h() {
        // WemVersion code + two i32 geometry fields — exactly 12 bytes.
        assert_eq!(std::mem::size_of::<WemProfile>(), 12);
        assert_eq!(
            std::mem::size_of::<WemProfile>(),
            3 * std::mem::size_of::<u32>()
        );
    }

    #[test]
    fn profile_selection_decodes_and_rejects_bad_geometry() {
        let supported = WemProfile {
            version: WemVersion::Wwise2013,
            channels: 2,
            sample_rate: 48_000,
        };
        let selection = supported
            .selection()
            .expect("known version, positive geometry");
        assert_eq!(selection.channels(), 2);
        assert_eq!(selection.sample_rate(), 48_000);

        for (channels, sample_rate) in [(0, 48_000), (2, 0), (-2, 48_000)] {
            let malformed = WemProfile {
                version: WemVersion::Wwise2013,
                channels,
                sample_rate,
            };
            assert_eq!(malformed.selection().err(), Some(WemError::StateError));
        }
    }

    #[test]
    fn null_profile_is_a_malformed_call() {
        // ABI surface: a NULL selection is rejected and the out-pointer is
        // cleared, never dereferenced. FFI requires the unsafe call; no
        // pointer is read.
        let mut encoder: *mut WemEncoder = std::ptr::null_mut();
        let code = unsafe { wem_encoder_new(std::ptr::null(), &mut encoder) };
        assert_eq!(code, WemError::StateError);
        assert!(encoder.is_null());
    }

    /// A write callback that accepts everything: no case here delivers bytes
    /// worth keeping.
    unsafe extern "C" fn discard_write(
        _data: *const u8,
        _len: usize,
        _user_data: *mut c_void,
    ) -> WemError {
        WemError::Ok
    }

    /// The panic rule of include/wem.h ("Panics") as the mapping the entries
    /// apply to every kernel call.
    ///
    /// The mapping is tested directly instead of by panicking inside a kernel
    /// call: forcing one would need a fault-injection switch in `src/`, and a
    /// switch that can abort an encode is not something to ship. The caught
    /// panic below is a real one, produced in the test.
    #[test]
    fn guarded_outcome_is_the_whole_panic_rule() {
        // A call that returns a value: nothing is terminal, either scope.
        for scope in [PanicScope::OneShot, PanicScope::Handle] {
            let guarded = guarded_outcome::<u32>(scope, Ok(Ok(7)));
            assert_eq!(guarded.outcome, Ok(7), "{scope:?}: the value passes");
            assert!(!guarded.handle_dead, "{scope:?}: success kills nothing");
        }

        // A rejection the kernel reported: its own stable code survives, and
        // no rejection other than the defect code is terminal.
        for (error, code) in [
            (
                EncoderError::GeometryMismatch {
                    message: "trailing partial frame".into(),
                },
                WemError::GeometryMismatch,
            ),
            (
                EncoderError::StateError {
                    message: "PCM channel count is not representable".into(),
                },
                WemError::StateError,
            ),
            (
                EncoderError::InputTooShort { want: 4096, got: 1 },
                WemError::InputTooShort,
            ),
            (
                EncoderError::ProfileNotFound {
                    requested: "2ch/44100Hz/2013".into(),
                },
                WemError::ProfileNotFound,
            ),
            (
                EncoderError::FormatUnsupported {
                    message: "not signed-16 PCM".into(),
                },
                WemError::FormatUnsupported,
            ),
        ] {
            let guarded = guarded_outcome::<u32>(PanicScope::Handle, Ok(Err(error)));
            assert_eq!(guarded.outcome, Err(code), "{code:?} must keep its code");
            assert!(
                !guarded.handle_dead,
                "{code:?} refuses one call; it must leave the handle usable"
            );
        }

        // The defect code — reported by the kernel or reached by a panic —
        // keeps the code INTERNAL, and only a handle-carrying entry dies on
        // it: a one-shot entry has nothing to kill and may be called again.
        for scope in [PanicScope::OneShot, PanicScope::Handle] {
            let terminal = scope == PanicScope::Handle;
            let reported = guarded_outcome::<u32>(
                scope,
                Ok(Err(EncoderError::Internal(
                    wem_core::InternalError::Invariant { message: "x" },
                ))),
            );
            assert_eq!(reported.outcome, Err(WemError::Internal));
            assert_eq!(
                reported.handle_dead, terminal,
                "{scope:?}: a reported kernel defect follows the same rule as a panic"
            );

            let panicked = guarded_outcome::<u32>(
                scope,
                std::panic::catch_unwind(|| -> Result<u32, EncoderError> {
                    panic!("injected kernel invariant violation")
                }),
            );
            assert_eq!(
                panicked.outcome,
                Err(WemError::Internal),
                "{scope:?}: a caught panic is WEM_ERR_INTERNAL"
            );
            assert_eq!(
                panicked.handle_dead, terminal,
                "{scope:?}: only a handle-carrying entry dies on a panic"
            );
        }
    }

    /// The other half of the rule: what a dead handle answers afterwards.
    #[test]
    fn a_dead_encoder_handle_rejects_every_later_call() {
        let profile = WemProfile {
            version: WemVersion::Wwise2013,
            channels: 6,
            sample_rate: 44_100,
        };
        let mut handle: *mut WemEncoder = std::ptr::null_mut();
        assert_eq!(
            unsafe { wem_encoder_new(&profile, &mut handle) },
            WemError::Ok,
            "the fixture selection resolves"
        );
        // One frame of silence: enough to reach the kernel, which refuses it
        // for its length. That refusal is what tells a live handle from a
        // dead one below.
        let pcm = [0i16; 6];
        let pcm = pcm.as_ptr().cast::<u8>();
        assert_eq!(
            unsafe {
                wem_encoder_encode(handle, pcm, 1, Some(discard_write), std::ptr::null_mut())
            },
            WemError::InputTooShort,
            "a live handle runs the call and reports the kernel's own code"
        );

        unsafe { (*handle).mark_dead() };

        assert_eq!(
            unsafe {
                wem_encoder_encode(handle, pcm, 1, Some(discard_write), std::ptr::null_mut())
            },
            WemError::StateError,
            "a dead handle is refused before the kernel sees the call"
        );
        unsafe {
            wem_encoder_free(handle);
        }
    }
}
