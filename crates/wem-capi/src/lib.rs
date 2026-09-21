//! wem-capi — the C ABI core surface of the WEM encoder kernel.
//!
//! This crate is the single cross-language integration point: a thin,
//! stable C ABI over `wem-core`. The normative contract lives in
//! `include/wem.h` (lifecycle, reply framing, error codes, memory
//! ownership, integration rules); the signatures here mirror it 1:1.
//!
//! Design rules (crates/AGENTS.md, "C ABI contract"):
//! * Zero drift: validation, profile loading, checksums and encoding run
//!   only in the kernel (`wem_core::Encoder`, `wem_core::StreamSession`);
//!   this layer only converts C values to kernel types and kernel errors
//!   to `WemError`.
//! * No unwind across the FFI: every kernel call is wrapped in
//!   `catch_unwind`; a kernel panic becomes `WEM_ERR_INTERNAL`.
//! * Every entry validates its arguments (NULL checks, length checks)
//!   before touching kernel state.
//!
//! Thread-safety semantics (normative, mirrored in include/wem.h):
//! * [`WemEncoder`] is shareable: concurrent `wem_encoder_encode` /
//!   `wem_encode_pcm16_interleaved` calls may share one handle.
//! * [`WemSession`] has single-threaded ownership: never share it
//!   between threads (it is intentionally not `Send`).

use std::ffi::{c_void, CStr};
use std::path::PathBuf;

use wem_core::encoder::{Encoder, Pcm16};
use wem_core::error::EncoderError;
use wem_core::stream::StreamSession;
use wem_profiles::data::DataDir;

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
/// cross-language contract (see the capi_e2e stability test).
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
}

/// Terminal container summary (include/wem.h `WemMeta`): byte length and
/// lowercase-hex SHA-256 of the assembled WEM bytes (64 chars, no NUL).
#[repr(C)]
#[derive(Debug, Clone, Copy)]
pub struct WemMeta {
    /// Byte length of the assembled container.
    pub total_len: u64,
    /// SHA-256 of the container bytes, lowercase hex (exactly 64 bytes).
    pub sha256_hex: [u8; 64],
}

/// Shareable profile-resolved encoder (include/wem.h `WemEncoder`).
pub struct WemEncoder {
    encoder: Encoder,
}

/// Streaming encode session (include/wem.h `WemSession`):
/// single-threaded ownership; `failed` marks the terminal error state.
pub struct WemSession {
    session: StreamSession,
    write_cb: Option<WemWriteFn>,
    packet_cb: Option<WemPacketFn>,
    user_data: *mut c_void,
    /// Next reply-packet sequence number (seq 0 = setup packet).
    next_seq: u32,
    /// Terminal state set by `finish` (success or error).
    failed: bool,
}

/// Output block size for the write callback (bounded, no size limit on
/// the whole container).
const WRITE_BLOCK: usize = 65536;

fn load_encoder(name: &str, dir: Option<PathBuf>) -> Result<Encoder, EncoderError> {
    match dir {
        Some(dir) => Encoder::from_profile_in(&DataDir::from_profiles_dir(dir), name),
        None => Encoder::from_profile(name),
    }
}

fn load_stream(name: &str, dir: Option<PathBuf>) -> Result<StreamSession, EncoderError> {
    match dir {
        Some(dir) => StreamSession::for_profile_in(&DataDir::from_profiles_dir(dir), name),
        None => StreamSession::for_profile(name),
    }
}

/// Decode a NUL-terminated C string (None when NULL or non-UTF-8).
fn cstr_to_string(ptr: *const u8) -> Option<String> {
    if ptr.is_null() {
        return None;
    }
    let cstr = unsafe { CStr::from_ptr(ptr as *const i8) };
    cstr.to_str().ok().map(str::to_string)
}

/// A NUL-terminated optional string argument (NULL or empty -> None).
fn cstr_opt(ptr: *const u8) -> Option<PathBuf> {
    match cstr_to_string(ptr) {
        Some(value) if !value.is_empty() => Some(PathBuf::from(value)),
        _ => None,
    }
}

/// Wrap one kernel call so a panic cannot cross the FFI.
fn guarded<T>(
    work: impl FnOnce() -> Result<T, EncoderError> + std::panic::UnwindSafe,
) -> Result<T, WemError> {
    match std::panic::catch_unwind(work) {
        Ok(Ok(value)) => Ok(value),
        Ok(Err(error)) => Err(WemError::from_kernel(&error)),
        Err(_) => Err(WemError::Internal),
    }
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
fn encode_with(
    encoder: &Encoder,
    pcm: *const u8,
    frames: usize,
    write_cb: Option<WemWriteFn>,
    user_data: *mut c_void,
) -> Result<(), WemError> {
    if frames == 0 || pcm.is_null() || write_cb.is_none() {
        // The caller guarantees `frames` complete frames behind `pcm`;
        // NULL/zero arguments are malformed calls, never encodable input.
        return Err(WemError::StateError);
    }
    let channels = encoder.profile().channels() as usize;
    let byte_len = frames
        .checked_mul(channels * 2)
        .ok_or(WemError::StateError)?;
    let bytes = unsafe { std::slice::from_raw_parts(pcm, byte_len) };
    let pcm16 = Pcm16::from_interleaved_le(encoder.profile().sample_rate(), channels, bytes)
        .map_err(|error| WemError::from_kernel(&error))?;
    // CPU-bound kernel work behind the FFI boundary: catch any panic.
    let outcome = std::panic::catch_unwind(std::panic::AssertUnwindSafe(move || {
        encoder.encode_pcm(&pcm16)
    }));
    match outcome {
        Ok(Ok(result)) => emit_bytes(&result.data, write_cb, user_data),
        Ok(Err(error)) => Err(WemError::from_kernel(&error)),
        Err(_) => Err(WemError::Internal),
    }
}

// ---------------------------------------------------------------------------
// Encoder handle (shareable)
// ---------------------------------------------------------------------------

/// Resolve one installed profile into a shareable encoder handle
/// (include/wem.h `wem_encoder_new`).
///
/// `data_dir` selects an explicit profile tree without changing process
/// environment; NULL uses the profiles compiled into the library. On
/// `WEM_OK`, `*out_encoder` owns the handle; on error it is set to NULL.
///
/// # Safety
///
/// - `profile_name` must be NUL-terminated UTF-8 (or NULL: rejected, no
///   UB), `data_dir` the same;
/// - `out_encoder` must not be NULL.
#[no_mangle]
pub unsafe extern "C" fn wem_encoder_new(
    profile_name: *const u8,
    data_dir: *const u8,
    out_encoder: *mut *mut WemEncoder,
) -> WemError {
    if out_encoder.is_null() {
        return WemError::StateError;
    }
    let Some(name) = cstr_to_string(profile_name) else {
        return WemError::StateError;
    };
    let dir = cstr_opt(data_dir);
    let outcome = guarded(move || {
        Ok(WemEncoder {
            encoder: load_encoder(&name, dir)?,
        })
    });
    match outcome {
        Ok(encoder) => {
            unsafe {
                *out_encoder = Box::into_raw(Box::new(encoder));
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
/// NULL is a no-op.
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
    match encode_with(&handle.encoder, pcm, frames, write_cb, user_data) {
        Ok(()) => WemError::Ok,
        Err(code) => code,
    }
}

// ---------------------------------------------------------------------------
// One-shot convenience entry
// ---------------------------------------------------------------------------

/// One-shot encode: resolve the profile, encode, deliver the container
/// bytes (include/wem.h `wem_encode_pcm16_interleaved`).
///
/// `pcm` must hold `frames * channels` little-endian signed-16 samples
/// (interleaved, `channels` from the profile); `data_dir` NULL uses profiles
/// compiled into the library. Output goes through `write_cb` in bounded blocks —
/// callback-style output, so there is no container size limit.
///
/// # Safety
///
/// - `profile_name` / `data_dir` as in `wem_encoder_new`;
/// - `pcm` must hold at least `frames * channels * 2` bytes for the
///   duration of the call;
/// - `write_cb` must be a live callback (or NULL: rejected).
#[no_mangle]
pub unsafe extern "C" fn wem_encode_pcm16_interleaved(
    profile_name: *const u8,
    data_dir: *const u8,
    pcm: *const u8,
    frames: usize,
    write_cb: Option<WemWriteFn>,
    user_data: *mut c_void,
) -> WemError {
    let Some(name) = cstr_to_string(profile_name) else {
        return WemError::StateError;
    };
    let dir = cstr_opt(data_dir);
    let outcome = guarded(move || load_encoder(&name, dir));
    match outcome {
        Ok(encoder) => match encode_with(&encoder, pcm, frames, write_cb, user_data) {
            Ok(()) => WemError::Ok,
            Err(code) => code,
        },
        Err(code) => code,
    }
}

// ---------------------------------------------------------------------------
// Streaming session
// ---------------------------------------------------------------------------

/// Open a streaming session on one installed profile
/// (include/wem.h `wem_session_new`).
///
/// Lifecycle: `wem_session_new` (Init) -> `wem_session_push`* ->
/// `wem_session_finish` (Finish) -> `wem_session_free`. Emitted packets
/// are delivered through `packet_cb` (NULL discards them); the terminal
/// container bytes are delivered through `write_cb` at `finish`
/// (required). The session has single-threaded ownership. On `WEM_OK`,
/// `*out_session` owns the handle; on error it is set to NULL.
///
/// # Safety
///
/// - `profile_name` / `data_dir` as in `wem_encoder_new`;
/// - `write_cb` must be a live callback (or NULL: rejected); `packet_cb`
///   must be NULL or a live callback;
/// - `out_session` must not be NULL;
/// - the returned session is owned by the calling thread (single-threaded
///   lifetime; never share it).
#[no_mangle]
pub unsafe extern "C" fn wem_session_new(
    profile_name: *const u8,
    data_dir: *const u8,
    write_cb: Option<WemWriteFn>,
    packet_cb: Option<WemPacketFn>,
    user_data: *mut c_void,
    out_session: *mut *mut WemSession,
) -> WemError {
    if out_session.is_null() {
        return WemError::StateError;
    }
    let Some(name) = cstr_to_string(profile_name) else {
        return WemError::StateError;
    };
    if write_cb.is_none() {
        return WemError::StateError;
    }
    let write_cb = Some(write_cb.expect("validated non-null above"));
    let dir = cstr_opt(data_dir);
    let outcome = guarded(move || {
        load_stream(&name, dir).map(|session| WemSession {
            session,
            write_cb,
            packet_cb,
            user_data,
            next_seq: 0,
            failed: false,
        })
    });
    match outcome {
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
    let outcome = {
        let state = unsafe { &mut *session };
        if state.failed {
            return WemError::StateError;
        }
        guarded(std::panic::AssertUnwindSafe(move || {
            state.session.push_pcm_chunk(bytes)
        }))
    };
    match outcome {
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
            unsafe {
                (*session).failed = true;
            }
            code
        }
    }
}

/// Mark the end of the PCM stream, complete the encode, deliver the
/// container bytes through the write callback, and fill the terminal
/// summary (include/wem.h `wem_session_finish`). Terminal: after this
/// call (regardless of outcome) the session must be released, not used.
///
/// # Safety
///
/// - `session` must be NULL or a live handle owned by this thread;
/// - `out_meta` must not be NULL (it is written on `WEM_OK`);
/// - the write callback must be callable from this thread.
#[no_mangle]
pub unsafe extern "C" fn wem_session_finish(
    session: *mut WemSession,
    out_meta: *mut WemMeta,
) -> WemError {
    if session.is_null() || out_meta.is_null() {
        return WemError::StateError;
    }
    let outcome = {
        let state = unsafe { &mut *session };
        if state.failed {
            return WemError::StateError;
        }
        guarded(std::panic::AssertUnwindSafe(move || state.session.finish()))
    };
    match outcome {
        Ok(result) => {
            let state = unsafe { &mut *session };
            if let Err(code) = emit_bytes(&result.data, state.write_cb, state.user_data) {
                state.failed = true;
                return code;
            }
            let sha = result.sha256();
            unsafe {
                (*out_meta).total_len = result.data.len() as u64;
                (&mut (*out_meta).sha256_hex)[..sha.len()].copy_from_slice(sha.as_bytes());
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
/// NULL is a no-op, safe before or after `finish`.
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
    }

    #[test]
    fn meta_layout_matches_wem_h() {
        // u64 total_len + sha256_hex[64] — exactly the C struct.
        assert_eq!(std::mem::size_of::<WemMeta>(), 72);
        assert_eq!(
            std::mem::size_of::<u64>() + 64,
            std::mem::size_of::<WemMeta>()
        );
    }
}
