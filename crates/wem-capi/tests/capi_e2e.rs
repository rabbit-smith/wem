//! C ABI end-to-end contract tests (wem-capi).
//!
//! Drive the FFI surface exactly the way an external language would
//! (through the rlib symbols, `unsafe` and all) and prove:
//! * one-shot encode == golden WEM bytes (fixture PCM);
//! * streaming with seven uneven chunks == the same bytes, seq-ordered
//!   packets, correct terminal meta;
//! * encoder-handle path matches the one-shot convenience entry;
//! * NULL arguments, unknown profiles, and lifecycle violations return
//!   the expected stable codes;
//! * the error-code table and the `WemMeta` layout stay in sync with
//!   include/wem.h.

use std::ffi::c_void;
use std::path::PathBuf;

use sha2::{Digest, Sha256};
use wem_capi::{WemError, WemMeta};
use wem_core::usecases::wav::read_pcm16;

const PROFILE_NAME: &str = "wwise2013-6ch-44100";
const GOLDEN_SHA256: &str = "17851d26c6210b85e498ae0452d2562d7b9e2c3e9e795c459656b9c9d8d35247";

// ---------------------------------------------------------------------------
// Test sinks (what a C client would do with its callbacks).
//
// Each test owns its sink (passed as `user_data`), so parallel test
// threads never share a buffer.
// ---------------------------------------------------------------------------

struct Sinks {
    out: Vec<u8>,
    packets: Vec<(u32, Vec<u8>)>,
}

impl Sinks {
    fn new() -> Box<Self> {
        Box::new(Self {
            out: Vec::new(),
            packets: Vec::new(),
        })
    }

    /// The `user_data` pointer this sink presents to the C callbacks.
    fn user_data(&mut self) -> *mut c_void {
        std::ptr::from_mut(self) as *mut c_void
    }
}

unsafe extern "C" fn sink_write(data: *const u8, len: usize, user_data: *mut c_void) -> WemError {
    if user_data.is_null() {
        return WemError::Internal;
    }
    let sinks = unsafe { &mut *(user_data as *mut Sinks) };
    let chunk = unsafe { std::slice::from_raw_parts(data, len) };
    sinks.out.extend_from_slice(chunk);
    WemError::Ok
}

unsafe extern "C" fn sink_packet(
    seq: u32,
    data: *const u8,
    len: usize,
    user_data: *mut c_void,
) -> WemError {
    if user_data.is_null() {
        return WemError::Internal;
    }
    let sinks = unsafe { &mut *(user_data as *mut Sinks) };
    let chunk = unsafe { std::slice::from_raw_parts(data, len) };
    sinks.packets.push((seq, chunk.to_vec()));
    WemError::Ok
}

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

fn fixtures_dir() -> PathBuf {
    std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../..")
        .join("tests/fixtures")
        .canonicalize()
        .expect("fixtures directory resolves")
}

fn reference_wem() -> Vec<u8> {
    std::fs::read(fixtures_dir().join("reference.wem")).expect("reference.wem reads")
}

fn read_fixture_pcm() -> (Vec<u8>, usize, usize) {
    let wav = read_pcm16(&fixtures_dir().join("input.wav")).expect("input.wav reads");
    (wav.interleaved_le_bytes(), wav.frames(), wav.channels())
}

fn cstr(value: &str) -> std::ffi::CString {
    std::ffi::CString::new(value).expect("no NUL in test strings")
}

/// The C-ABI string argument form (char bytes as u8).
fn p(cstring: &std::ffi::CString) -> *const u8 {
    cstring.as_ptr() as *const u8
}

fn sha256_hex(bytes: &[u8]) -> String {
    Sha256::digest(bytes)
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect()
}

/// The setup packet inside one assembled WEM container (seq 0).
fn setup_packet_of(wem_bytes: &[u8]) -> Vec<u8> {
    let parts = wem_container::load_wem_parts_bytes(wem_bytes).expect("wem parts load");
    parts.setup_packet.expect("setup packet present").to_vec()
}

// ---------------------------------------------------------------------------
// 1. One-shot path
// ---------------------------------------------------------------------------

#[test]
fn one_shot_fixture_rebuilds_golden() {
    let (pcm, frames, _channels) = read_fixture_pcm();
    let profile = cstr(PROFILE_NAME);
    let mut sinks = Sinks::new();
    let ud = sinks.user_data();

    let code = unsafe {
        wem_capi::wem_encode_pcm16_interleaved(
            p(&profile),
            std::ptr::null(),
            pcm.as_ptr(),
            frames,
            Some(sink_write),
            ud,
        )
    };
    assert_eq!(code, WemError::Ok, "one-shot encode rejected");

    let out = &sinks.out;
    assert_eq!(
        sha256_hex(out),
        GOLDEN_SHA256,
        "one-shot C ABI output sha256 differs from the golden"
    );
    assert_eq!(
        out,
        reference_wem().as_slice(),
        "one-shot C ABI output differs from reference.wem"
    );
}

#[test]
fn encoder_handle_matches_one_shot_and_reuses() {
    let (pcm, frames, _channels) = read_fixture_pcm();
    let profile = cstr(PROFILE_NAME);

    let mut handle = std::ptr::null_mut();
    let code = unsafe { wem_capi::wem_encoder_new(p(&profile), std::ptr::null(), &mut handle) };
    assert_eq!(code, WemError::Ok, "encoder_new rejected");
    assert!(!handle.is_null());

    // Two independent encodes through the shared handle: byte-identical.
    for run in 0..2 {
        let mut sinks = Sinks::new();
        let ud = sinks.user_data();
        let code = unsafe {
            wem_capi::wem_encoder_encode(handle, pcm.as_ptr(), frames, Some(sink_write), ud)
        };
        assert_eq!(code, WemError::Ok, "encoder_encode {run} rejected");
        assert_eq!(
            sinks.out,
            reference_wem(),
            "encoder handle run {run} differs"
        );
    }
    unsafe {
        wem_capi::wem_encoder_free(handle);
        // NULL release is a documented no-op.
        wem_capi::wem_encoder_free(std::ptr::null_mut());
    }
}

// ---------------------------------------------------------------------------
// 2. Streaming path
// ---------------------------------------------------------------------------

#[test]
fn streaming_seven_uneven_chunks_match_one_shot() {
    let (pcm, _frames, channels) = read_fixture_pcm();

    // Seven uneven, frame-aligned chunks: the last takes the remainder.
    let head_frames: [usize; 6] = [137, 211, 97, 331, 211, 53];
    let mut bounds = vec![0usize];
    for frame_count in head_frames {
        bounds.push(bounds.last().copied().unwrap() + frame_count * channels * 2);
    }
    bounds.push(pcm.len());
    assert!(bounds[6] < pcm.len(), "the seventh chunk must be non-empty");

    let profile = cstr(PROFILE_NAME);
    let mut sinks = Sinks::new();
    let ud = sinks.user_data();

    let mut session = std::ptr::null_mut();
    let code = unsafe {
        wem_capi::wem_session_new(
            p(&profile),
            std::ptr::null(),
            Some(sink_write),
            Some(sink_packet),
            ud,
            &mut session,
        )
    };
    assert_eq!(code, WemError::Ok, "session_new rejected");
    assert!(!session.is_null());

    for (index, pair) in bounds.windows(2).enumerate() {
        let (start, end) = (pair[0], pair[1]);
        let code =
            unsafe { wem_capi::wem_session_push(session, pcm[start..end].as_ptr(), end - start) };
        assert_eq!(code, WemError::Ok, "push {index} rejected");
    }

    let mut meta: WemMeta = unsafe { std::mem::zeroed() };
    let code = unsafe { wem_capi::wem_session_finish(session, &mut meta) };
    assert_eq!(code, WemError::Ok, "finish rejected");
    unsafe {
        wem_capi::wem_session_free(session);
    }

    // Container bytes: the golden.
    let out = &sinks.out;
    assert_eq!(sha256_hex(out), GOLDEN_SHA256, "streaming sha256 differs");
    assert_eq!(
        out,
        reference_wem().as_slice(),
        "streaming bytes differ from reference.wem"
    );

    // Terminal meta: length + lowercase-hex digest of the container.
    assert_eq!(meta.total_len as usize, out.len());
    assert_eq!(
        std::str::from_utf8(&meta.sha256_hex).expect("hex is ascii"),
        GOLDEN_SHA256
    );

    // Reply stream: seq starts at 0 (the setup packet) and increases by
    // one per emitted packet; the setup packet equals the container's.
    let packets = &sinks.packets;
    assert!(!packets.is_empty(), "no packets delivered");
    for (index, (seq, _data)) in packets.iter().enumerate() {
        assert_eq!(*seq, index as u32, "seq must be contiguous from 0");
    }
    assert_eq!(
        packets[0].1,
        setup_packet_of(out),
        "seq 0 must be the setup packet"
    );
}

#[test]
fn streaming_null_packet_cb_still_streams_bytes() {
    let (pcm, _frames, _channels) = read_fixture_pcm();
    let profile = cstr(PROFILE_NAME);
    let mut sinks = Sinks::new();
    let ud = sinks.user_data();

    let mut session = std::ptr::null_mut();
    let code = unsafe {
        wem_capi::wem_session_new(
            p(&profile),
            std::ptr::null(),
            Some(sink_write),
            None,
            ud,
            &mut session,
        )
    };
    assert_eq!(code, WemError::Ok);
    let code = unsafe { wem_capi::wem_session_push(session, pcm.as_ptr(), pcm.len()) };
    assert_eq!(code, WemError::Ok);
    let mut meta: WemMeta = unsafe { std::mem::zeroed() };
    let code = unsafe { wem_capi::wem_session_finish(session, &mut meta) };
    assert_eq!(code, WemError::Ok);
    unsafe {
        wem_capi::wem_session_free(session);
    }
    let out = &sinks.out;
    assert_eq!(out, reference_wem().as_slice());
    assert_eq!(meta.total_len as usize, out.len());
}

// ---------------------------------------------------------------------------
// 3. Argument validation (NULL / malformed calls)
// ---------------------------------------------------------------------------

#[test]
fn null_arguments_reject_with_state_error() {
    let (pcm, frames, _channels) = read_fixture_pcm();
    let profile = cstr(PROFILE_NAME);
    let mut sinks = Sinks::new();
    let ud = sinks.user_data();

    let mut handle = std::ptr::null_mut();
    assert_eq!(
        unsafe { wem_capi::wem_encoder_new(std::ptr::null(), std::ptr::null(), &mut handle) },
        WemError::StateError,
        "NULL profile name must reject"
    );
    assert_eq!(
        unsafe { wem_capi::wem_encoder_new(p(&profile), std::ptr::null(), std::ptr::null_mut()) },
        WemError::StateError,
        "NULL out handle must reject"
    );
    assert_eq!(
        unsafe {
            wem_capi::wem_encode_pcm16_interleaved(
                std::ptr::null(),
                std::ptr::null(),
                pcm.as_ptr(),
                frames,
                Some(sink_write),
                ud,
            )
        },
        WemError::StateError,
        "NULL profile name must reject"
    );
    assert_eq!(
        unsafe {
            wem_capi::wem_encode_pcm16_interleaved(
                p(&profile),
                std::ptr::null(),
                std::ptr::null(),
                frames,
                Some(sink_write),
                ud,
            )
        },
        WemError::StateError,
        "NULL pcm must reject"
    );
    assert_eq!(
        unsafe {
            wem_capi::wem_encode_pcm16_interleaved(
                p(&profile),
                std::ptr::null(),
                pcm.as_ptr(),
                frames,
                None,
                ud,
            )
        },
        WemError::StateError,
        "NULL write callback must reject"
    );

    let mut session = std::ptr::null_mut();
    assert_eq!(
        unsafe {
            wem_capi::wem_session_new(
                p(&profile),
                std::ptr::null(),
                Some(sink_write),
                None,
                ud,
                std::ptr::null_mut(),
            )
        },
        WemError::StateError,
        "NULL out session must reject"
    );
    assert_eq!(
        unsafe {
            wem_capi::wem_session_new(p(&profile), std::ptr::null(), None, None, ud, &mut session)
        },
        WemError::StateError,
        "NULL write callback must reject"
    );
    assert_eq!(
        unsafe { wem_capi::wem_session_push(std::ptr::null_mut(), pcm.as_ptr(), 2) },
        WemError::StateError,
        "NULL session push must reject"
    );
    assert_eq!(
        unsafe { wem_capi::wem_session_finish(std::ptr::null_mut(), &mut std::mem::zeroed()) },
        WemError::StateError,
        "NULL session finish must reject"
    );

    // NULL releases are documented no-ops (no crash, no UB).
    unsafe {
        wem_capi::wem_encoder_free(std::ptr::null_mut());
        wem_capi::wem_session_free(std::ptr::null_mut());
    }
}

// ---------------------------------------------------------------------------
// 4. Profile selection semantics
// ---------------------------------------------------------------------------

#[test]
fn unknown_profile_rejects_with_profile_not_found() {
    let (pcm, frames, _channels) = read_fixture_pcm();
    let missing = cstr("no-such-profile-xyz");
    let mut sinks = Sinks::new();
    let ud = sinks.user_data();

    let mut handle = std::ptr::null_mut();
    assert_eq!(
        unsafe { wem_capi::wem_encoder_new(p(&missing), std::ptr::null(), &mut handle) },
        WemError::ProfileNotFound,
        "unknown profile name must map to PROFILE_NOT_FOUND"
    );
    assert!(
        handle.is_null(),
        "failed encoder_new must leave a NULL handle"
    );

    assert_eq!(
        unsafe {
            wem_capi::wem_encode_pcm16_interleaved(
                p(&missing),
                std::ptr::null(),
                pcm.as_ptr(),
                frames,
                Some(sink_write),
                ud,
            )
        },
        WemError::ProfileNotFound
    );

    let mut session = std::ptr::null_mut();
    assert_eq!(
        unsafe {
            wem_capi::wem_session_new(
                p(&missing),
                std::ptr::null(),
                Some(sink_write),
                None,
                ud,
                &mut session,
            )
        },
        WemError::ProfileNotFound
    );
    assert!(
        session.is_null(),
        "failed session_new must leave a NULL handle"
    );
}

#[test]
fn explicit_data_dir_selects_profile_tree() {
    let profiles_dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../..")
        .join("src/wwise_wem/data/profiles");
    let dir_cstr = cstr(&profiles_dir.to_string_lossy());
    let profile = cstr(PROFILE_NAME);
    let mut handle = std::ptr::null_mut();
    assert_eq!(
        unsafe { wem_capi::wem_encoder_new(p(&profile), p(&dir_cstr), &mut handle) },
        WemError::Ok,
        "explicit data_dir must resolve the installed profile"
    );
    unsafe {
        wem_capi::wem_encoder_free(handle);
    }
}

// ---------------------------------------------------------------------------
// 5. Lifecycle violations
// ---------------------------------------------------------------------------

#[test]
fn lifecycle_violations_reject_with_state_error() {
    let (pcm, _frames, _channels) = read_fixture_pcm();
    let profile = cstr(PROFILE_NAME);
    let mut sinks = Sinks::new();
    let ud = sinks.user_data();

    let mut session = std::ptr::null_mut();
    assert_eq!(
        unsafe {
            wem_capi::wem_session_new(
                p(&profile),
                std::ptr::null(),
                Some(sink_write),
                None,
                ud,
                &mut session,
            )
        },
        WemError::Ok
    );
    // Full stream in one chunk, then the terminal calls.
    assert_eq!(
        unsafe { wem_capi::wem_session_push(session, pcm.as_ptr(), pcm.len()) },
        WemError::Ok,
        "push rejected"
    );

    // A second finish (after a successful one) is a lifecycle violation.
    let mut meta: WemMeta = unsafe { std::mem::zeroed() };
    assert_eq!(
        unsafe { wem_capi::wem_session_finish(session, &mut meta) },
        WemError::Ok,
        "finish on a full stream"
    );
    assert_eq!(
        unsafe { wem_capi::wem_session_push(session, pcm.as_ptr(), 2) },
        WemError::StateError,
        "push after finish must reject"
    );
    assert_eq!(
        unsafe { wem_capi::wem_session_finish(session, &mut meta) },
        WemError::StateError,
        "double finish must reject"
    );
    unsafe {
        wem_capi::wem_session_free(session);
    }
}

#[test]
fn short_stream_rejects_with_input_too_short() {
    let (pcm, frames, channels) = read_fixture_pcm();
    assert!(frames > 4096, "fixture is longer than the minimum");
    let short_frames = 100usize;
    let short_len = short_frames * channels * 2;
    let profile = cstr(PROFILE_NAME);
    let mut sinks = Sinks::new();
    let ud = sinks.user_data();

    let mut session = std::ptr::null_mut();
    assert_eq!(
        unsafe {
            wem_capi::wem_session_new(
                p(&profile),
                std::ptr::null(),
                Some(sink_write),
                None,
                ud,
                &mut session,
            )
        },
        WemError::Ok
    );
    assert_eq!(
        unsafe { wem_capi::wem_session_push(session, pcm.as_ptr(), short_len) },
        WemError::Ok,
        "pushing a short stream is legal until finish"
    );
    let mut meta: WemMeta = unsafe { std::mem::zeroed() };
    assert_eq!(
        unsafe { wem_capi::wem_session_finish(session, &mut meta) },
        WemError::InputTooShort,
        "a stream under 4096 frames must reject at finish"
    );
    unsafe {
        wem_capi::wem_session_free(session);
    }
}
