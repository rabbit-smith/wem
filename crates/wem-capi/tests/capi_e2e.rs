//! C ABI end-to-end contract tests (wem-capi).
//!
//! Drive the FFI surface exactly the way an external language would
//! (through the rlib symbols, `unsafe` and all) and prove:
//! * one-shot encode == golden WEM bytes (fixture PCM);
//! * streaming with seven uneven chunks == the same bytes, seq-ordered
//!   packets, correct terminal meta;
//! * encoder-handle path matches the one-shot convenience entry;
//! * one handle shared by several threads encodes concurrently and every
//!   thread reproduces the golden bytes (the include/wem.h shareability
//!   promise);
//! * NULL arguments, unsatisfiable selections, and lifecycle violations
//!   return the expected stable codes;
//! * an out-of-table `WemVersion` code is a revision mismatch
//!   (FORMAT_UNSUPPORTED), never a silent fallback;
//! * the error-code table and the `WemProfile` / `WemMeta` layouts stay in
//!   sync with include/wem.h.

use std::ffi::c_void;
use std::path::PathBuf;
use std::sync::Barrier;
use std::thread;

use sha2::{Digest, Sha256};
use wem_capi::{WemEncoder, WemError, WemMeta, WemProfile, WemVersion};
use wem_core::usecases::wav::read_pcm16;

const GOLDEN_SHA256: &str = "17851d26c6210b85e498ae0452d2562d7b9e2c3e9e795c459656b9c9d8d35247";

/// The fixture's encoder configuration: Wwise 2013.2, 6ch @ 44.1kHz.
fn fixture_profile() -> WemProfile {
    WemProfile {
        version: WemVersion::Wwise2013,
        channels: 6,
        sample_rate: 44_100,
    }
}

/// The other installed configuration: Wwise 2013.2, 2ch @ 48kHz.
fn stereo_profile() -> WemProfile {
    WemProfile {
        version: WemVersion::Wwise2013,
        channels: 2,
        sample_rate: 48_000,
    }
}

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

/// One `WemEncoder *` that may cross a thread boundary.
///
/// include/wem.h states normatively: *"WemEncoder is shareable: concurrent
/// encodes may use one handle on different threads"*, and declares the handle
/// as `/* Profile-resolved shareable encoder (concurrent encodes OK). */`.
/// This test drives the raw symbol exactly as a C client would, so the only
/// value that reaches the threads is the raw pointer and there is no Rust type
/// that could carry the promise — the wrapper carries it instead.
///
/// `unsafe` is the point of this file: an FFI test must exercise the C contract
/// on C terms (raw pointers, `unsafe extern "C"` callbacks), and the Rust type
/// system cannot express a claim the C header makes about a pointer it hands
/// out. `wem-capi` backs the same promise at compile time with the
/// `Send + Sync` assertion next to `WemEncoder`.
#[derive(Clone, Copy)]
struct SharedHandle(*const WemEncoder);

impl SharedHandle {
    /// The raw handle one `wem_encoder_encode` call needs. Taking `self` by
    /// value keeps the whole wrapper — and with it the `Send`/`Sync` claim
    /// below — in the closure capture, instead of the bare `!Send` raw pointer
    /// the field holds.
    fn raw(self) -> *const WemEncoder {
        self.0
    }
}

// SAFETY: the header promise quoted above — the handle is shareable across
// threads. The pointer is only ever borrowed by `wem_encoder_encode`
// (`*const WemEncoder` -> `&WemEncoder`) and is freed after every thread has
// joined, so no thread can observe a dropped encoder.
unsafe impl Send for SharedHandle {}
// SAFETY: `&SharedHandle` only ever yields the raw pointer, and concurrent
// encodes through it are exactly what the header documents as safe.
unsafe impl Sync for SharedHandle {}

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
    // The helper hands owned bytes to its callers, so the WAV's borrow is
    // copied here.
    (
        wav.interleaved_le_bytes().to_vec(),
        wav.frames(),
        wav.channels(),
    )
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
    let mut sinks = Sinks::new();
    let ud = sinks.user_data();

    let code = unsafe {
        wem_capi::wem_encode_pcm16_interleaved(
            &fixture_profile(),
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

    let mut handle = std::ptr::null_mut();
    let code = unsafe { wem_capi::wem_encoder_new(&fixture_profile(), &mut handle) };
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
// 2. One handle, several threads (the header's shareability promise)
// ---------------------------------------------------------------------------

/// include/wem.h: *"WemEncoder is shareable: concurrent encodes may use one
/// handle on different threads."* This is that sentence executed: one handle,
/// four threads, each encoding the fixture PCM into its own sink, and each
/// reproducing the golden container byte for byte.
#[test]
fn concurrent_encodes_share_one_handle() {
    const THREADS: usize = 4;

    let (pcm, frames, _channels) = read_fixture_pcm();
    let reference = reference_wem();

    let mut handle = std::ptr::null_mut();
    let code = unsafe { wem_capi::wem_encoder_new(&fixture_profile(), &mut handle) };
    assert_eq!(code, WemError::Ok, "encoder_new rejected");
    assert!(!handle.is_null());
    let shared = SharedHandle(handle.cast_const());
    // Every thread waits here before calling in, so the encodes genuinely
    // overlap instead of merely being permitted to.
    let barrier = Barrier::new(THREADS);

    let outputs: Vec<Vec<u8>> = thread::scope(|scope| {
        let mut joins = Vec::with_capacity(THREADS);
        for index in 0..THREADS {
            let barrier = &barrier;
            let pcm = pcm.as_slice();
            joins.push(scope.spawn(move || {
                // Each thread passes its own sink: a callback that received a
                // NULL `user_data` (the bug this guards against) makes
                // `sink_write` return WEM_ERR_INTERNAL, so the encode returns
                // that code and the assertion below fails loudly — no thread
                // can quietly write into another thread's buffer.
                let mut sinks = Sinks::new();
                let ud = sinks.user_data();
                barrier.wait();
                let code = unsafe {
                    wem_capi::wem_encoder_encode(
                        shared.raw(),
                        pcm.as_ptr(),
                        frames,
                        Some(sink_write),
                        ud,
                    )
                };
                assert_eq!(code, WemError::Ok, "thread {index} encode rejected");
                (index, sinks.out)
            }));
        }
        joins
            .into_iter()
            .map(|join| {
                let (index, out) = join.join().expect("an encode thread panicked");
                assert_eq!(
                    sha256_hex(&out),
                    GOLDEN_SHA256,
                    "thread {index} sha256 differs from the golden"
                );
                assert_eq!(
                    out, reference,
                    "thread {index} bytes differ from reference.wem"
                );
                out
            })
            .collect()
    });
    assert_eq!(outputs.len(), THREADS);

    unsafe {
        wem_capi::wem_encoder_free(handle);
    }
}

// ---------------------------------------------------------------------------
// 3. Streaming path
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

    let mut sinks = Sinks::new();
    let ud = sinks.user_data();

    let mut session = std::ptr::null_mut();
    let code = unsafe {
        wem_capi::wem_session_new(
            &fixture_profile(),
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
    let mut sinks = Sinks::new();
    let ud = sinks.user_data();

    let mut session = std::ptr::null_mut();
    let code = unsafe {
        wem_capi::wem_session_new(&fixture_profile(), Some(sink_write), None, ud, &mut session)
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
// 4. Argument validation (NULL / malformed calls)
// ---------------------------------------------------------------------------

#[test]
fn null_arguments_reject_with_state_error() {
    let (pcm, frames, _channels) = read_fixture_pcm();
    let mut sinks = Sinks::new();
    let ud = sinks.user_data();

    let mut handle = std::ptr::null_mut();
    assert_eq!(
        unsafe { wem_capi::wem_encoder_new(std::ptr::null(), &mut handle) },
        WemError::StateError,
        "NULL selection must reject"
    );
    assert_eq!(
        unsafe { wem_capi::wem_encoder_new(&fixture_profile(), std::ptr::null_mut()) },
        WemError::StateError,
        "NULL out handle must reject"
    );
    assert_eq!(
        unsafe {
            wem_capi::wem_encode_pcm16_interleaved(
                std::ptr::null(),
                pcm.as_ptr(),
                frames,
                Some(sink_write),
                ud,
            )
        },
        WemError::StateError,
        "NULL selection must reject"
    );
    assert_eq!(
        unsafe {
            wem_capi::wem_encode_pcm16_interleaved(
                &fixture_profile(),
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
                &fixture_profile(),
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
                &fixture_profile(),
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
        unsafe { wem_capi::wem_session_new(&fixture_profile(), None, None, ud, &mut session) },
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
// 5. Profile selection semantics
// ---------------------------------------------------------------------------

#[test]
fn both_installed_selections_resolve() {
    for profile in [fixture_profile(), stereo_profile()] {
        let mut handle = std::ptr::null_mut();
        assert_eq!(
            unsafe { wem_capi::wem_encoder_new(&profile, &mut handle) },
            WemError::Ok,
            "{}ch/{}Hz/2013 must resolve",
            profile.channels,
            profile.sample_rate
        );
        assert!(!handle.is_null());
        unsafe {
            wem_capi::wem_encoder_free(handle);
        }
    }
}

#[test]
fn unsatisfiable_selection_rejects_with_profile_not_found() {
    let (pcm, frames, _channels) = read_fixture_pcm();
    let mut sinks = Sinks::new();
    let ud = sinks.user_data();

    // Geometries no installed configuration provides: an uninstalled rate on
    // an installed channel count, and an uninstalled channel count.
    let unsupported = [
        WemProfile {
            version: WemVersion::Wwise2013,
            channels: 6,
            sample_rate: 48_000,
        },
        WemProfile {
            version: WemVersion::Wwise2013,
            channels: 3,
            sample_rate: 44_100,
        },
        WemProfile {
            version: WemVersion::Wwise2013,
            channels: 2,
            sample_rate: 44_100,
        },
    ];

    for profile in unsupported {
        let mut handle = std::ptr::null_mut();
        assert_eq!(
            unsafe { wem_capi::wem_encoder_new(&profile, &mut handle) },
            WemError::ProfileNotFound,
            "{}ch/{}Hz must not resolve",
            profile.channels,
            profile.sample_rate
        );
        assert!(
            handle.is_null(),
            "a failed encoder_new must leave a NULL handle"
        );

        assert_eq!(
            unsafe {
                wem_capi::wem_encode_pcm16_interleaved(
                    &profile,
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
                wem_capi::wem_session_new(&profile, Some(sink_write), None, ud, &mut session)
            },
            WemError::ProfileNotFound
        );
        assert!(
            session.is_null(),
            "a failed session_new must leave a NULL handle"
        );
    }
}

#[test]
fn malformed_selection_geometry_rejects_with_state_error() {
    for (channels, sample_rate) in [(0, 44_100), (6, 0), (-6, 44_100)] {
        let profile = WemProfile {
            version: WemVersion::Wwise2013,
            channels,
            sample_rate,
        };
        let mut handle = std::ptr::null_mut();
        assert_eq!(
            unsafe { wem_capi::wem_encoder_new(&profile, &mut handle) },
            WemError::StateError,
            "{channels}ch/{sample_rate}Hz is malformed, not merely uninstalled"
        );
        assert!(handle.is_null());
    }
}

/// A selection struct as a client built against a *newer* header would pass
/// it: same layout, a version code this revision does not implement.
#[repr(C)]
struct ForeignSelection {
    version: u32,
    channels: i32,
    sample_rate: i32,
}

#[test]
fn out_of_table_version_code_rejects_with_format_unsupported() {
    let foreign = ForeignSelection {
        version: 7,
        channels: 6,
        sample_rate: 44_100,
    };
    let mut handle = std::ptr::null_mut();
    let code = unsafe {
        wem_capi::wem_encoder_new(
            std::ptr::from_ref(&foreign).cast::<WemProfile>(),
            &mut handle,
        )
    };
    assert_eq!(
        code,
        WemError::FormatUnsupported,
        "an unimplemented WemVersion code must not fall back to a default"
    );
    assert!(handle.is_null());
}

// ---------------------------------------------------------------------------
// 6. Lifecycle violations
// ---------------------------------------------------------------------------

#[test]
fn lifecycle_violations_reject_with_state_error() {
    let (pcm, _frames, _channels) = read_fixture_pcm();
    let mut sinks = Sinks::new();
    let ud = sinks.user_data();

    let mut session = std::ptr::null_mut();
    assert_eq!(
        unsafe {
            wem_capi::wem_session_new(&fixture_profile(), Some(sink_write), None, ud, &mut session)
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
    let mut sinks = Sinks::new();
    let ud = sinks.user_data();

    let mut session = std::ptr::null_mut();
    assert_eq!(
        unsafe {
            wem_capi::wem_session_new(&fixture_profile(), Some(sink_write), None, ud, &mut session)
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
