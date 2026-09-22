//! The decode entries of the C ABI (include/wem.h section 5), driven as a C
//! client drives them: raw pointers, `unsafe extern "C"` callbacks, and the
//! codes the header's error table pins.
//!
//! What is pinned here is the *surface*: the header announcement fires exactly
//! once before any PCM, the PCM arrives interleaved at the container's channel
//! count and totals `dw_total_pcm_frames` frames, chunk boundaries never move
//! a sample, the out-pointer is written on every exit, the lifecycle is the
//! encoder's, and every rejection carries a code from the documented table —
//! never `WEM_ERR_INTERNAL`, which would mean this library broke an invariant
//! in response to input.
//!
//! The kernel-level evidence (the real WEM against its source, the round trip,
//! the reference-decoder comparison) lives in
//! `crates/wem-core/tests/decode_reference_wem.rs`; this file repeats one
//! decode through the ABI so the surface is not merely a wrapper nobody drove.

use std::ffi::c_void;
use std::path::Path;

use wem_capi::{
    wem_decoder_finish, wem_decoder_free, wem_decoder_new, wem_decoder_push, WemDecoder, WemError,
};

mod common;

use common::reference_wem;

// ---------------------------------------------------------------------------
// Test sinks (what a C client would do with its callbacks)
// ---------------------------------------------------------------------------

/// What a decode client collects: the header announcement and every PCM block.
struct DecodeSink {
    headers: Vec<(u32, u32, u64, Vec<u8>)>,
    pcm: Vec<f32>,
    blocks: Vec<usize>,
    /// Inserted between the callbacks so ordering is observable: the index of
    /// the first PCM block, counted in callback calls.
    calls: Vec<&'static str>,
    /// When set, the next call of that callback returns this code instead of
    /// `WEM_OK` (the abort path of section 5).
    fail_header: Option<WemError>,
    fail_pcm: Option<WemError>,
}

impl DecodeSink {
    fn new() -> Box<Self> {
        Box::new(Self {
            headers: Vec::new(),
            pcm: Vec::new(),
            blocks: Vec::new(),
            calls: Vec::new(),
            fail_header: None,
            fail_pcm: None,
        })
    }

    fn user_data(&mut self) -> *mut c_void {
        std::ptr::from_mut(self) as *mut c_void
    }

    fn frames(&self, channels: usize) -> usize {
        self.pcm.len() / channels
    }
}

unsafe extern "C" fn sink_header(
    channels: u32,
    sample_rate: u32,
    total_frames: u64,
    setup: *const u8,
    setup_len: usize,
    user_data: *mut c_void,
) -> WemError {
    if user_data.is_null() {
        return WemError::Internal;
    }
    let sink = unsafe { &mut *(user_data as *mut DecodeSink) };
    sink.calls.push("header");
    if let Some(code) = sink.fail_header.take() {
        return code;
    }
    let bytes = unsafe { std::slice::from_raw_parts(setup, setup_len) };
    sink.headers
        .push((channels, sample_rate, total_frames, bytes.to_vec()));
    WemError::Ok
}

unsafe extern "C" fn sink_pcm(
    interleaved: *const f32,
    frames: usize,
    user_data: *mut c_void,
) -> WemError {
    if user_data.is_null() {
        return WemError::Internal;
    }
    let sink = unsafe { &mut *(user_data as *mut DecodeSink) };
    sink.calls.push("pcm");
    if let Some(code) = sink.fail_pcm.take() {
        return code;
    }
    let samples = unsafe { std::slice::from_raw_parts(interleaved, frames * 6) };
    sink.pcm.extend_from_slice(samples);
    sink.blocks.push(frames);
    WemError::Ok
}

/// Open a decoder on one sink.
fn open(sink: &mut DecodeSink) -> *mut WemDecoder {
    let mut decoder: *mut WemDecoder = std::ptr::null_mut();
    let code = unsafe {
        wem_decoder_new(
            Some(sink_header),
            Some(sink_pcm),
            sink.user_data(),
            &mut decoder,
        )
    };
    assert_eq!(code, WemError::Ok);
    assert!(!decoder.is_null());
    decoder
}

/// Push every chunk of `wem`, then finish; returns the codes observed.
fn drive(decoder: *mut WemDecoder, wem: &[u8], chunk: usize) -> (Vec<WemError>, WemError) {
    let mut codes = Vec::new();
    for bytes in wem.chunks(chunk.max(1)) {
        codes.push(unsafe { wem_decoder_push(decoder, bytes.as_ptr(), bytes.len()) });
    }
    let finished = unsafe { wem_decoder_finish(decoder) };
    (codes, finished)
}

// ---------------------------------------------------------------------------
// The path that matters: the real WEM through the ABI
// ---------------------------------------------------------------------------

/// A real paired-build WEM decodes through the C ABI to exactly the frames the
/// container declares, with the geometry announced once before any PCM.
#[test]
fn the_real_wem_decodes_through_the_abi() {
    let wem = reference_wem();
    let mut sink = DecodeSink::new();
    let decoder = open(&mut sink);
    let (codes, finished) = drive(decoder, &wem, 8192);
    unsafe { wem_decoder_free(decoder) };

    assert!(codes.iter().all(|code| *code == WemError::Ok), "{codes:?}");
    assert_eq!(finished, WemError::Ok);

    assert_eq!(
        sink.headers.len(),
        1,
        "the header is announced exactly once"
    );
    let (channels, sample_rate, total_frames, setup) = &sink.headers[0];
    assert_eq!(*channels, 6);
    assert_eq!(*sample_rate, 44_100);
    assert_eq!(
        *total_frames, 139_398,
        "the header announces the frame count the container declares, before any PCM"
    );
    assert_eq!(setup.len(), 201);
    assert_eq!(
        sink.calls.first(),
        Some(&"header"),
        "the header precedes every PCM call"
    );
    assert_eq!(
        sink.calls.iter().filter(|call| **call == "header").count(),
        1
    );

    assert_eq!(sink.frames(6), 139_398, "dw_total_pcm_frames");
    assert_eq!(sink.pcm.len() % 6, 0);
    assert!(sink.blocks.iter().all(|frames| *frames <= 1024));
    assert!(sink.blocks.iter().all(|frames| *frames > 0));
    assert!(
        sink.pcm.iter().all(|sample| sample.abs() <= 4.0),
        "the PCM is at ±1.0 full scale, not raw integers"
    );
}

/// Chunk boundaries never affect the emitted samples, through the ABI.
#[test]
fn chunking_never_moves_a_sample_through_the_abi() {
    let wem = reference_wem();
    let mut whole = DecodeSink::new();
    let decoder = open(&mut whole);
    let (_, finished) = drive(decoder, &wem, wem.len());
    unsafe { wem_decoder_free(decoder) };
    assert_eq!(finished, WemError::Ok);

    for chunk in [1usize, 7, 203, 4096] {
        let mut sink = DecodeSink::new();
        let decoder = open(&mut sink);
        let (_, finished) = drive(decoder, &wem, chunk);
        unsafe { wem_decoder_free(decoder) };
        assert_eq!(finished, WemError::Ok, "chunk {chunk}");
        assert_eq!(sink.pcm, whole.pcm, "chunk {chunk} changed the samples");
        assert_eq!(sink.headers.len(), 1, "chunk {chunk}");
    }
}

/// An empty push is a no-op: nothing is announced, nothing is delivered, and
/// the call succeeds.
#[test]
fn an_empty_push_is_a_no_op() {
    let mut sink = DecodeSink::new();
    let decoder = open(&mut sink);
    let code = unsafe { wem_decoder_push(decoder, std::ptr::null(), 0) };
    assert_eq!(code, WemError::Ok);
    assert!(sink.headers.is_empty());
    assert!(sink.pcm.is_empty());
    assert!(sink.calls.is_empty());
    unsafe { wem_decoder_free(decoder) };
}

// ---------------------------------------------------------------------------
// Out-parameters, lifecycle and malformed calls (include/wem.h sections 1, 3)
// ---------------------------------------------------------------------------

/// `wem_decoder_new` writes its out-pointer on every exit that can reach it.
#[test]
fn the_out_pointer_is_written_on_every_exit() {
    // NULL out-pointer: the one call that cannot write.
    let code = unsafe {
        wem_decoder_new(
            Some(sink_header),
            Some(sink_pcm),
            std::ptr::null_mut(),
            std::ptr::null_mut(),
        )
    };
    assert_eq!(code, WemError::StateError);

    // A missing callback is a malformed call, and the out-pointer is cleared.
    let mut decoder: *mut WemDecoder = std::ptr::null_mut();
    let code = unsafe { wem_decoder_new(None, Some(sink_pcm), std::ptr::null_mut(), &mut decoder) };
    assert_eq!(code, WemError::StateError);
    assert!(decoder.is_null(), "a failed Init hands out no handle");
    let mut decoder: *mut WemDecoder = std::ptr::null_mut();
    let code =
        unsafe { wem_decoder_new(Some(sink_header), None, std::ptr::null_mut(), &mut decoder) };
    assert_eq!(code, WemError::StateError);
    assert!(decoder.is_null());

    // A *successful* Init writes the handle, and NULL is a no-op to free.
    let mut sink = DecodeSink::new();
    let decoder = open(&mut sink);
    unsafe { wem_decoder_free(decoder) };
    unsafe { wem_decoder_free(std::ptr::null_mut()) };
}

/// Terminal calls: after `finish` (whatever it returned) and after a defect,
/// every later call is `WEM_ERR_STATE_ERROR`, and NULL is always rejected.
#[test]
fn finish_is_terminal_and_null_handles_are_rejected() {
    let wem = reference_wem();
    let mut sink = DecodeSink::new();
    let decoder = open(&mut sink);
    let (_, finished) = drive(decoder, &wem, wem.len());
    assert_eq!(finished, WemError::Ok);
    assert_eq!(
        unsafe { wem_decoder_finish(decoder) },
        WemError::StateError,
        "Finish is terminal"
    );
    assert_eq!(
        unsafe { wem_decoder_push(decoder, wem.as_ptr(), wem.len()) },
        WemError::StateError,
        "a finished decoder takes no more bytes"
    );
    unsafe { wem_decoder_free(decoder) };

    assert_eq!(
        unsafe { wem_decoder_push(std::ptr::null_mut(), wem.as_ptr(), wem.len()) },
        WemError::StateError
    );
    assert_eq!(
        unsafe { wem_decoder_finish(std::ptr::null_mut()) },
        WemError::StateError
    );
    // A NULL data pointer with a non-zero length is rejected, not dereferenced.
    let mut sink = DecodeSink::new();
    let decoder = open(&mut sink);
    assert_eq!(
        unsafe { wem_decoder_push(decoder, std::ptr::null(), 4) },
        WemError::StateError
    );
    unsafe { wem_decoder_free(decoder) };
}

/// A callback that returns non-`WEM_OK` aborts the decode with that code and
/// leaves the handle terminal.
#[test]
fn a_failing_callback_aborts_the_decode() {
    let wem = reference_wem();
    let mut sink = DecodeSink::new();
    sink.fail_pcm = Some(WemError::StateError);
    let decoder = open(&mut sink);
    let code = unsafe { wem_decoder_push(decoder, wem.as_ptr(), wem.len()) };
    assert_eq!(code, WemError::StateError, "the callback's own code");
    assert_eq!(
        unsafe { wem_decoder_finish(decoder) },
        WemError::StateError,
        "an aborted decode is terminal"
    );
    unsafe { wem_decoder_free(decoder) };

    // The same for the header callback, which fires before any PCM.
    let mut sink = DecodeSink::new();
    sink.fail_header = Some(WemError::FormatUnsupported);
    let decoder = open(&mut sink);
    let code = unsafe { wem_decoder_push(decoder, wem.as_ptr(), wem.len()) };
    assert_eq!(code, WemError::FormatUnsupported);
    assert!(sink.pcm.is_empty(), "no PCM follows a refused header");
    unsafe { wem_decoder_free(decoder) };
}

// ---------------------------------------------------------------------------
// The rejection classes (include/wem.h section 5)
// ---------------------------------------------------------------------------

/// Every input-derived rejection is a typed code from the table — never
/// `WEM_ERR_INTERNAL`, which is the defect code — and the handle stays usable
/// afterwards, with the same rejection reported again rather than skipped.
#[test]
fn malformed_input_is_a_typed_rejection_not_a_defect() {
    let wem = reference_wem();
    let mut cases: Vec<(&str, Vec<u8>, WemError)> = Vec::new();
    cases.push((
        "not a RIFF container",
        b"this is not a wem at all".to_vec(),
        WemError::InputMalformed,
    ));
    cases.push((
        "truncated inside the packet stream",
        wem[..wem.len() - 1].to_vec(),
        WemError::InputMalformed,
    ));
    cases.push(("empty input", Vec::new(), WemError::InputMalformed));

    // A RIFF container whose fmt chunk is not the Wwise Vorbis tag: parses
    // cleanly, names a kind this revision does not decode.
    let mut other = Vec::new();
    other.extend_from_slice(b"RIFF");
    other.extend_from_slice(&(4u32 + 8 + 66).to_le_bytes());
    other.extend_from_slice(b"WAVE");
    other.extend_from_slice(b"fmt ");
    other.extend_from_slice(&66u32.to_le_bytes());
    other.extend_from_slice(&[0u8; 66]);
    cases.push(("a non-Wwise fmt tag", other, WemError::FormatUnsupported));

    // A Wwise Vorbis container whose geometry this build does not carry.
    let mut unknown = wem.clone();
    let fmt_at = unknown
        .windows(4)
        .position(|window| window == b"fmt ")
        .expect("the fmt chunk is in the fixture");
    let payload = fmt_at + 8;
    unknown[payload + 2..payload + 4].copy_from_slice(&3u16.to_le_bytes());
    cases.push((
        "an uninstalled geometry",
        unknown,
        WemError::FormatUnsupported,
    ));

    // A container whose setup packet is not the one this build carries.
    let mut foreign = wem.clone();
    let data_at = foreign
        .windows(4)
        .position(|window| window == b"data")
        .expect("the data chunk is in the fixture");
    foreign[data_at + 11] ^= 0x01;
    cases.push((
        "a foreign setup packet",
        foreign,
        WemError::FormatUnsupported,
    ));

    for (case, bytes, expected) in cases {
        let mut sink = DecodeSink::new();
        let decoder = open(&mut sink);
        let code = unsafe { wem_decoder_push(decoder, bytes.as_ptr(), bytes.len()) };
        if code == WemError::Ok {
            // The shortfall may only be visible at Finish (a container that
            // ends inside a packet).
            let finished = unsafe { wem_decoder_finish(decoder) };
            assert_eq!(finished, expected, "case {case}: expected {expected:?}");
        } else {
            assert_eq!(code, expected, "case {case}: expected {expected:?}");
            assert_ne!(code, WemError::Internal, "case {case} is not a defect");
            // A rejection leaves the handle usable: the same bytes report the
            // same rejection again instead of being skipped.
            let again = unsafe { wem_decoder_push(decoder, bytes.as_ptr(), bytes.len()) };
            assert_eq!(again, expected, "case {case}: the rejection is repeatable");
            // Finish re-reports the pending rejection (the bytes it could not
            // read are still there) and is terminal whatever it returns.
            assert_eq!(
                unsafe { wem_decoder_finish(decoder) },
                expected,
                "case {case}: Finish reports the decode's own outcome"
            );
            assert_eq!(
                unsafe { wem_decoder_finish(decoder) },
                WemError::StateError,
                "case {case}: Finish consumes the handle whatever it reports"
            );
        }
        unsafe { wem_decoder_free(decoder) };
    }
}

/// The whole real WEM delivered one byte at a time is the same decode as one
/// push, and it still ends with a successful Finish.
#[test]
fn one_byte_chunks_reach_the_same_finish() {
    let wem = reference_wem();
    let mut sink = DecodeSink::new();
    let decoder = open(&mut sink);
    let (codes, finished) = drive(decoder, &wem, 1);
    unsafe { wem_decoder_free(decoder) };
    assert!(codes.iter().all(|code| *code == WemError::Ok));
    assert_eq!(finished, WemError::Ok);
    assert_eq!(sink.frames(6), 139_398);
}

/// The fixture path helper exists so this file never restates a repository
/// path; `reference.wem` is the committed paired-build artifact.
#[test]
fn the_fixture_is_the_committed_artifact() {
    let wem = reference_wem();
    assert!(wem.starts_with(b"RIFF"));
    assert_eq!(
        wem.len(),
        std::fs::metadata(
            Path::new(env!("CARGO_MANIFEST_DIR")).join("../../tests/fixtures/reference.wem")
        )
        .expect("the fixture exists")
        .len() as usize
    );
}
