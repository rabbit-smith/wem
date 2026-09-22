//! Adversarial input at the C ABI boundary: every case is a typed rejection.
//!
//! `WEM_ERR_INTERNAL` is the defect code (include/wem.h, "Panics"): it says
//! this library broke an invariant, so no input a caller can hand this surface
//! may produce it — and none may panic. The cases below are the input-derived
//! paths a hostile or merely confused caller reaches: a truncated RIFF header,
//! a `fmt ` chunk of size 0 or 1, a zero-frame stream, absurd channel counts
//! and geometries, a non-finite quality factor, PCM whose shape cannot be
//! represented at all.
//!
//! Every case names itself in its assertion message, so a failure says which
//! input was accepted, panicked, or was reported as a defect.
//!
//! Two of the listed shapes have no C entry point of their own and are driven
//! where they do enter the kernel, which is what keeps the claim honest:
//!
//! * a WAV is parsed by `wem_core::usecases::wav::parse_pcm16` — the wasm
//!   shell's `wem_parse_wav` and the CLI's input path;
//! * a quality factor is bound by
//!   `wem_core::encoder::Encoder::new_with_quality` — PyO3's `quality=`.
//!
//! The float-domain forms of non-finite and out-of-range PCM exist only in the
//! Python shell (a `float64` memoryview, out-of-range `int`s), which drives
//! them in its own case; on this surface PCM is raw `int16` bytes, where every
//! bit pattern is a sample and the adversarial shapes are lengths and channel
//! counts instead.

use std::ffi::c_void;

use wem_capi::{
    wem_encode_pcm16_interleaved, wem_encoder_new, wem_session_new, WemEncoder, WemError,
    WemProfile, WemSession, WemVersion,
};
use wem_core::encoder::{Encoder, Pcm16};
use wem_core::error::EncoderError;
use wem_core::stream::StreamSession;
use wem_core::usecases::wav::parse_pcm16;
use wem_core::{WwiseProfile, WwiseVersion};

// ---------------------------------------------------------------------------
// Case drivers: a panic is never an answer, so it is caught and named.
// ---------------------------------------------------------------------------

/// Run one case. The library must return a value: a panic is an invariant
/// violation, not a rejection of bad input, and the message says which input
/// caused it.
fn drive<T>(case: &str, work: impl FnOnce() -> T) -> T {
    match std::panic::catch_unwind(std::panic::AssertUnwindSafe(work)) {
        Ok(value) => value,
        Err(_) => panic!(
            "adversarial case {case}: the library panicked; a panic is an invariant \
             violation, never an answer to input"
        ),
    }
}

/// The ABI call must refuse the case with the documented code.
fn assert_abi(case: &str, expected: WemError, call: impl FnOnce() -> WemError) {
    let code = drive(case, call);
    assert_eq!(
        code, expected,
        "adversarial case {case}: expected the documented {expected:?}, got {code:?}"
    );
}

/// The kernel call must refuse the case, and the error class is returned for
/// the caller to pin.
fn rejected<T>(case: &str, call: impl FnOnce() -> Result<T, EncoderError>) -> EncoderError {
    match drive(case, call) {
        Ok(_) => panic!("adversarial case {case}: the kernel accepted input it must refuse"),
        Err(error) => error,
    }
}

/// The stable class name of one kernel error (the include/wem.h table).
fn class_of(error: &EncoderError) -> &'static str {
    match error {
        EncoderError::ProfileNotFound { .. } => "PROFILE_NOT_FOUND",
        EncoderError::StateError { .. } => "STATE_ERROR",
        EncoderError::GeometryMismatch { .. } => "GEOMETRY_MISMATCH",
        EncoderError::InputTooShort { .. } => "INPUT_TOO_SHORT",
        EncoderError::FormatUnsupported { .. } => "FORMAT_UNSUPPORTED",
        EncoderError::Internal(_) => "INTERNAL",
    }
}

/// The case was refused as caller input, not reported as a defect.
fn assert_refused(case: &str, error: &EncoderError, expected: &str) {
    assert_ne!(
        class_of(error),
        "INTERNAL",
        "adversarial case {case}: reported as a defect ({error}); INTERNAL means this \
         library broke an invariant, not that the caller passed something bad"
    );
    assert_eq!(
        class_of(error),
        expected,
        "adversarial case {case}: wrong rejection class for {error}"
    );
}

// ---------------------------------------------------------------------------
// Adversarial RIFF/WAVE construction
// ---------------------------------------------------------------------------

/// A RIFF/WAVE file whose `WAVE` form is `chunks` verbatim, with the RIFF size
/// field computed from them.
fn wav(chunks: &[u8]) -> Vec<u8> {
    let mut out = Vec::with_capacity(chunks.len() + 12);
    out.extend_from_slice(b"RIFF");
    out.extend_from_slice(&(chunks.len() as u32 + 4).to_le_bytes());
    out.extend_from_slice(b"WAVE");
    out.extend_from_slice(chunks);
    out
}

/// One chunk with an explicit size field, so a case can declare a size its
/// payload does not have.
fn chunk(id: &[u8; 4], declared_size: u32, payload: &[u8]) -> Vec<u8> {
    let mut out = id.to_vec();
    out.extend_from_slice(&declared_size.to_le_bytes());
    out.extend_from_slice(payload);
    if declared_size % 2 == 1 {
        out.push(0); // word alignment
    }
    out
}

/// The 16-byte PCM `fmt ` payload the parser requires. The byte-rate field is
/// sink data for the parser, so it saturates instead of overflowing on the
/// absurd geometries built below (a test that panics building its own input
/// would prove nothing about the library).
fn fmt_payload(channels: u16, sample_rate: u32, bits_per_sample: u16) -> Vec<u8> {
    let mut out = Vec::with_capacity(16);
    out.extend_from_slice(&1u16.to_le_bytes()); // PCM
    out.extend_from_slice(&channels.to_le_bytes());
    out.extend_from_slice(&sample_rate.to_le_bytes());
    out.extend_from_slice(
        &sample_rate
            .saturating_mul(u32::from(channels))
            .saturating_mul(2)
            .to_le_bytes(),
    );
    out.extend_from_slice(&channels.saturating_mul(2).to_le_bytes());
    out.extend_from_slice(&bits_per_sample.to_le_bytes());
    out
}

/// A well-formed signed-16 PCM WAV: `frames` frames of silence.
fn silence_wav(channels: u16, sample_rate: u32, frames: usize) -> Vec<u8> {
    let bytes = frames * usize::from(channels) * 2;
    let mut chunks = chunk(b"fmt ", 16, &fmt_payload(channels, sample_rate, 16));
    chunks.extend_from_slice(&chunk(b"data", bytes as u32, &vec![0u8; bytes]));
    wav(&chunks)
}

// ---------------------------------------------------------------------------
// Cases
// ---------------------------------------------------------------------------

/// A RIFF header that stops short of its own form: the parser must refuse the
/// file, never index past what it received.
#[test]
fn truncated_riff_headers_are_rejected_not_panics() {
    // Every prefix of a valid file that does not yet carry a whole header.
    let complete = silence_wav(1, 44_100, 1);
    for length in 0..12 {
        let case = format!("RIFF header truncated to {length} bytes");
        let error = rejected(&case, || parse_pcm16(&complete[..length]));
        assert_refused(&case, &error, "FORMAT_UNSUPPORTED");
    }

    // A header that lies about its own form: a wrong RIFF size field is
    // accepted (the chunk walk is the authority), a wrong form is not.
    let mut wrong_form = complete.clone();
    wrong_form[8..12].copy_from_slice(b"AVI ");
    let case = "RIFF header declaring a form other than WAVE";
    let error = rejected(case, || parse_pcm16(&wrong_form));
    assert_refused(case, &error, "FORMAT_UNSUPPORTED");

    // Cut inside the first chunk, and inside the data chunk.
    for cut in [12, 20, 28] {
        let case = format!("valid WAV truncated at {cut} bytes");
        let error = rejected(&case, || parse_pcm16(&complete[..cut]));
        assert_refused(&case, &error, "FORMAT_UNSUPPORTED");
    }
}

/// A `fmt ` chunk whose declared size is 0 or 1: the size field is part of the
/// input, so it is validated like the payload.
#[test]
fn fmt_chunks_of_size_zero_and_one_are_rejected_not_panics() {
    for declared in [0u32, 1] {
        let case = format!("fmt chunk declaring size {declared}");
        let mut chunks = chunk(b"fmt ", declared, &[0u8; 1][..declared as usize]);
        chunks.extend_from_slice(&chunk(b"data", 2, &[0, 0]));
        let error = rejected(&case, || parse_pcm16(&wav(&chunks)));
        assert_refused(&case, &error, "FORMAT_UNSUPPORTED");
    }

    // A size field no file can hold: larger than any buffer the parser reads,
    // and large enough that a 32-bit `offset + size` would wrap.
    for declared in [u32::MAX, u32::MAX - 15, 0x8000_0000] {
        let case = format!("fmt chunk declaring size {declared}");
        let mut chunks = chunk(b"fmt ", declared, &fmt_payload(1, 44_100, 16));
        chunks.extend_from_slice(&chunk(b"data", 2, &[0, 0]));
        let error = rejected(&case, || parse_pcm16(&wav(&chunks)));
        assert_refused(&case, &error, "FORMAT_UNSUPPORTED");
    }

    // A `fmt ` chunk that promises 16 bytes and delivers fewer.
    for delivered in 0..16usize {
        let case = format!("fmt chunk promising 16 bytes, delivering {delivered}");
        let mut chunks = chunk(b"fmt ", 16, &fmt_payload(1, 44_100, 16)[..delivered]);
        chunks.extend_from_slice(&chunk(b"data", 2, &[0, 0]));
        let error = rejected(&case, || parse_pcm16(&wav(&chunks)));
        assert_refused(&case, &error, "FORMAT_UNSUPPORTED");
    }
}

/// Zero frames: an empty stream is refused wherever it arrives, and never
/// reaches the analysis pipeline.
#[test]
fn zero_frame_streams_are_rejected_not_panics() {
    // A data chunk with no samples at all.
    let mut chunks = chunk(b"fmt ", 16, &fmt_payload(1, 44_100, 16));
    chunks.extend_from_slice(&chunk(b"data", 0, &[]));
    let case = "WAV with an empty data chunk";
    let error = rejected(case, || parse_pcm16(&wav(&chunks)));
    assert_refused(case, &error, "FORMAT_UNSUPPORTED");

    // The ABI: zero frames is a malformed call, not encodable input.
    let pcm = [0i16; 64];
    let profile = profile(6, 44_100);
    assert_abi(
        "one-shot encode of zero frames",
        WemError::StateError,
        || unsafe {
            wem_encode_pcm16_interleaved(
                &profile,
                pcm.as_ptr().cast::<u8>(),
                0,
                Some(discard_write),
                std::ptr::null_mut(),
            )
        },
    );

    // The kernel's streaming session: zero frames accumulated, then Finish.
    let case = "streaming session finished with zero frames";
    let error = rejected(case, || {
        StreamSession::for_selection(selection(6, 44_100))?.finish()
    });
    assert_refused(case, &error, "INPUT_TOO_SHORT");

    // The kernel's encoder: a stream below the 4096-frame minimum.
    let case = "one-shot encode of 4095 frames";
    let error = rejected(case, || {
        let encoder = Encoder::new(selection(6, 44_100))?;
        let pcm = Pcm16::from_interleaved_le(44_100, 6, vec![0u8; 4095 * 6 * 2])?;
        encoder.encode_pcm(&pcm)
    });
    assert_refused(case, &error, "INPUT_TOO_SHORT");
}

/// Absurd channel counts and geometries: non-positive ones are malformed
/// arguments, positive ones that no compiled configuration satisfies are
/// unsatisfiable selections — never a defect, and never an allocation sized by
/// the caller's number.
#[test]
fn absurd_geometries_are_rejected_not_panics() {
    for (channels, sample_rate, expected) in [
        (0, 44_100, WemError::StateError),
        (-1, 44_100, WemError::StateError),
        (i32::MIN, 44_100, WemError::StateError),
        (6, 0, WemError::StateError),
        (6, -44_100, WemError::StateError),
        (6, i32::MIN, WemError::StateError),
        (i32::MAX, 44_100, WemError::ProfileNotFound),
        (i32::MAX, i32::MAX, WemError::ProfileNotFound),
        (6, i32::MAX, WemError::ProfileNotFound),
    ] {
        let case = format!("{channels}ch/{sample_rate}Hz selection");
        let profile = profile(channels, sample_rate);
        let mut handle: *mut WemEncoder = std::ptr::null_mut();
        assert_abi(&case, expected, || unsafe {
            wem_encoder_new(&profile, &mut handle)
        });
        assert!(
            handle.is_null(),
            "adversarial case {case}: a refused selection must hand out no handle"
        );

        let mut session: *mut WemSession = std::ptr::null_mut();
        assert_abi(&case, expected, || unsafe {
            wem_session_new(
                &profile,
                Some(discard_write),
                None,
                std::ptr::null_mut(),
                &mut session,
            )
        });
        assert!(
            session.is_null(),
            "adversarial case {case}: a refused selection must hand out no session"
        );
    }

    // A selection the ABI resolves, driven with a frame count whose byte
    // length cannot be represented: the multiplication is checked, never
    // wrapped into a short slice.
    let pcm = [0i16; 64];
    let profile = profile(6, 44_100);
    assert_abi(
        "one-shot encode of usize::MAX frames",
        WemError::StateError,
        || unsafe {
            wem_encode_pcm16_interleaved(
                &profile,
                pcm.as_ptr().cast::<u8>(),
                usize::MAX,
                Some(discard_write),
                std::ptr::null_mut(),
            )
        },
    );

    // The kernel's PCM constructors: a channel count that cannot be
    // multiplied into a byte length, and a channel count from a WAV header
    // that no data chunk can fill.
    let case = "interleaved PCM claiming usize::MAX channels";
    let error = rejected(case, || {
        Pcm16::from_interleaved_le(44_100, usize::MAX, [0, 0])
    });
    assert_refused(case, &error, "STATE_ERROR");

    let case = "channel-major PCM claiming usize::MAX channels";
    let error = rejected(case, || {
        Pcm16::from_channel_major_le(44_100, usize::MAX, vec![0, 0])
    });
    assert_refused(case, &error, "STATE_ERROR");

    let mut chunks = chunk(b"fmt ", 16, &fmt_payload(0, 44_100, 16));
    chunks.extend_from_slice(&chunk(b"data", 2, &[0, 0]));
    let case = "WAV declaring zero channels";
    let error = rejected(case, || parse_pcm16(&wav(&chunks)));
    assert_refused(case, &error, "FORMAT_UNSUPPORTED");

    let mut chunks = chunk(b"fmt ", 16, &fmt_payload(u16::MAX, 44_100, 16));
    chunks.extend_from_slice(&chunk(b"data", 2, &[0, 0]));
    let case = "WAV declaring 65535 channels with a two-byte data chunk";
    let error = rejected(case, || parse_pcm16(&wav(&chunks)));
    assert_refused(case, &error, "FORMAT_UNSUPPORTED");
}

/// A quality factor is caller input: NaN and the infinities are refused by the
/// profile resolution, never carried into the analysis resources.
#[test]
fn non_finite_quality_is_rejected_not_a_panic() {
    for quality in [f64::NAN, f64::INFINITY, f64::NEG_INFINITY] {
        let case = format!("quality {quality}");
        let error = rejected(&case, || {
            Encoder::new_with_quality(selection(6, 44_100), Some(quality)).map(|_| ())
        });
        assert_refused(&case, &error, "STATE_ERROR");

        let error = rejected(&case, || {
            StreamSession::for_selection_quality(selection(6, 44_100), Some(quality)).map(|_| ())
        });
        assert_refused(&case, &error, "STATE_ERROR");
    }
}

/// PCM shapes that cannot be represented: a byte length that is not a whole
/// number of frames, a channel count no allocation can serve, a chunk that
/// carries half a sample.
#[test]
fn unrepresentable_pcm_shapes_are_rejected_not_panics() {
    // Interleaved bytes that stop inside a frame.
    for bytes in [1usize, 11, 13, 6 * 2 * 4096 - 1] {
        let case = format!("interleaved PCM of {bytes} bytes for 6 channels");
        let error = rejected(&case, || {
            Pcm16::from_interleaved_le(44_100, 6, vec![0u8; bytes]).map(|_| ())
        });
        assert_refused(&case, &error, "GEOMETRY_MISMATCH");
    }

    // A stream chunk that carries a trailing partial frame reaches the ABI
    // through the session, where the same rule holds.
    let case = "session chunk with a trailing partial frame";
    let mut session: *mut WemSession = std::ptr::null_mut();
    let profile = profile(6, 44_100);
    let mut sink: Vec<u8> = Vec::new();
    let user_data = std::ptr::from_mut(&mut sink).cast::<c_void>();
    assert_eq!(
        drive(case, || unsafe {
            wem_session_new(&profile, Some(discard_write), None, user_data, &mut session)
        }),
        WemError::Ok,
        "adversarial case {case}: the fixture selection must open"
    );
    let odd = [0u8; 13];
    assert_abi(case, WemError::GeometryMismatch, || unsafe {
        wem_capi::wem_session_push(session, odd.as_ptr(), odd.len())
    });

    // The rejected chunk was a refusal, not a defect: the session is still the
    // live session it was, and the stream it carries is unaffected. A push of
    // whole frames still succeeds (the byte suites own what it produces).
    let whole = [0u8; 12 * 16];
    assert_abi(
        &format!("{case}: the session after a refused chunk"),
        WemError::Ok,
        || unsafe { wem_capi::wem_session_push(session, whole.as_ptr(), whole.len()) },
    );
    unsafe {
        wem_capi::wem_session_free(session);
    }
}

// ---------------------------------------------------------------------------
// Fixtures (no digest constants: the byte suites own the bytes)
// ---------------------------------------------------------------------------

/// A write callback that accepts everything: no case here produces container
/// bytes worth keeping.
unsafe extern "C" fn discard_write(
    _data: *const u8,
    _len: usize,
    _user_data: *mut c_void,
) -> WemError {
    WemError::Ok
}

/// One ABI profile selection.
fn profile(channels: i32, sample_rate: i32) -> WemProfile {
    WemProfile {
        version: WemVersion::Wwise2013,
        channels,
        sample_rate,
    }
}

/// The same selection in the kernel's own type.
fn selection(channels: i64, sample_rate: i64) -> WwiseProfile {
    WwiseProfile::new(WwiseVersion::Wwise2013, channels, sample_rate)
        .expect("the case's geometry is a valid selection")
}
