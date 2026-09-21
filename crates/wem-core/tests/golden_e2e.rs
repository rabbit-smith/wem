//! End-to-end golden parity test: the Rust kernel must reproduce the
//! reference WEM byte-for-byte (the P2-4 gate).
//!
//! Oracle: `tests/fixtures/reference.wem`, produced by the Python encoder
//! from `tests/fixtures/input.wav`.
//!
//! This test asserts file-level byte equality (`Vec<u8> ==`); the digest of
//! that artifact is never re-typed here — the bytes are the claim.

use wem_core::encoder::{Encoder, Pcm16};
use wem_core::error::EncoderError;
use wem_core::stream::StreamSession;
use wem_core::usecases::wav::read_pcm16;
use wem_core::{WwiseProfile, WwiseVersion};

mod common;

use common::{fixture_selection, fixtures_dir, read_fixture};

/// Read the fixture WAV and hand it to the encoder the way the CLI does.
fn encode_fixture() -> (wem_core::EncodeResult, Encoder) {
    let wav = read_pcm16(&fixtures_dir().join("input.wav")).expect("input.wav reads");
    assert_eq!(wav.channels(), 6);
    assert_eq!(wav.sample_rate(), 44100);
    assert_eq!(wav.frames(), 139398);
    let encoder = Encoder::new(fixture_selection()).expect("selection resolves");
    let pcm = wav.to_pcm16().expect("wav converts to Pcm16");
    let result = encoder.encode_pcm(&pcm).expect("encode runs");
    (result, encoder)
}

// ---------------------------------------------------------------------------
// 1. Golden byte parity (file-level, Vec<u8> equality)
// ---------------------------------------------------------------------------

#[test]
fn golden_encode_is_byte_identical_to_reference_wem() {
    let (result, _encoder) = encode_fixture();
    let reference = read_fixture("reference.wem");
    // The whole claim: the kernel's bytes are the reference container's bytes.
    assert_eq!(
        result.data, reference,
        "rust WEM differs from reference.wem at file level"
    );
    // Stats mirror the Python golden expectations.
    let stats = &result.stats;
    assert_eq!(stats.pcm_frames, 139398);
    assert_eq!(stats.channels, 6);
    assert_eq!(stats.audio_packets, 205);
    assert_eq!(stats.short_packets, 77);
    assert_eq!(stats.long_packets, 128);
    assert_eq!(stats.bytes, 108771);
    // The provenance label is the name-free selection description.
    assert_eq!(stats.metadata_source, "profile:6ch/44100Hz/2013");
}

// ---------------------------------------------------------------------------
// 2. StreamSession: chunk boundaries must not affect the output bytes
// ---------------------------------------------------------------------------

fn fixture_le_bytes() -> Vec<u8> {
    let wav = read_pcm16(&fixtures_dir().join("input.wav")).expect("input.wav reads");
    // The streaming cases slice the bytes repeatedly, so they take an owned
    // copy of the WAV's borrow.
    wav.interleaved_le_bytes().to_vec()
}

#[test]
fn stream_session_with_seven_uneven_chunks_matches_encode_pcm() {
    let bytes = fixture_le_bytes();
    let channels = 6usize;
    let bytes_per_frame = 2 * channels;
    let total_frames = bytes.len() / bytes_per_frame;
    // Seven unequal frame-count chunks summing to the full stream.
    let frame_boundaries: Vec<usize> =
        vec![0, 17000, 40500, 70600, 85600, 110100, 129100, total_frames];
    assert_eq!(frame_boundaries.len(), 8, "seven chunks");
    assert!(
        frame_boundaries
            .windows(2)
            .all(|w| w[0] < w[1] && w[1] <= total_frames),
        "uneven, in-range chunks"
    );
    assert!(
        frame_boundaries
            .windows(2)
            .map(|w| w[1] - w[0])
            .collect::<Vec<_>>()
            .windows(2)
            .all(|w| w[0] != w[1]),
        "all chunk sizes differ"
    );

    let mut session = StreamSession::for_selection(fixture_selection()).expect("session opens");
    for window in frame_boundaries.windows(2) {
        let start = window[0] * bytes_per_frame;
        let end = window[1] * bytes_per_frame;
        session
            .push_pcm_chunk(&bytes[start..end])
            .expect("chunk pushes");
    }
    let streamed = session.finish().expect("finish");
    let (oneshot, encoder) = encode_fixture();
    assert_eq!(
        streamed.data, oneshot.data,
        "streamed encode differs from encode_pcm"
    );
    assert_eq!(streamed.data, read_fixture("reference.wem"));
    assert_eq!(streamed.stats, oneshot.stats);
    // The container walk exposes the reply packet stream: the setup
    // packet (seq 0) followed by the audio packets in encoding order.
    let parts = wem_container::load_wem_parts_bytes(&streamed.data).expect("wem parts");
    assert!(parts.is_wwise_vorbis);
    let setup = encoder.profile().setup_packet().expect("setup reads");
    assert_eq!(parts.setup_packet, Some(setup.clone()));
    assert_eq!(parts.audio_packets.len(), 205);
    let mut reassembled = Vec::new();
    for packet in
        std::iter::once(setup.as_slice()).chain(parts.audio_packets.iter().map(|p| p.as_slice()))
    {
        reassembled.extend_from_slice(&(packet.len() as u16).to_le_bytes());
        reassembled.extend_from_slice(packet);
    }
    assert!(
        parts.data_raw.starts_with(&reassembled),
        "data chunk payload does not start with the reply packet stream"
    );
}

// ---------------------------------------------------------------------------
// 3. Error paths (same semantics as the Python encoder / v1 codes)
// ---------------------------------------------------------------------------

#[test]
fn input_too_short_rejects_4095_frames() {
    let wav = read_pcm16(&fixtures_dir().join("input.wav")).expect("input.wav reads");
    let frames = 4095usize;
    let samples_per_frame = wav.channels() * 2;
    let bytes = &wav.interleaved_le_bytes()[0..frames * samples_per_frame];
    let pcm = Pcm16::from_interleaved_le(44100, 6, bytes).expect("pcm parses");
    let encoder = Encoder::new(fixture_selection()).expect("selection resolves");
    match encoder.encode_pcm(&pcm) {
        Err(EncoderError::InputTooShort { want, got }) => {
            assert_eq!(want, 4096);
            assert_eq!(got, 4095);
        }
        Err(other) => panic!("expected InputTooShort, got {other:?}"),
        Ok(_) => panic!("4095 frames must be rejected"),
    }
}

#[test]
fn stream_session_input_too_short_rejects_4095_frames() {
    let wav = read_pcm16(&fixtures_dir().join("input.wav")).expect("input.wav reads");
    let bytes: Vec<u8> = wav.interleaved_le_bytes()[0..4095 * wav.channels() * 2].to_vec();
    let mut session = StreamSession::for_selection(fixture_selection()).expect("session opens");
    session.push_pcm_chunk(&bytes).expect("chunk pushes");
    match session.finish() {
        Err(EncoderError::InputTooShort { want, got }) => {
            assert_eq!(want, 4096);
            assert_eq!(got, 4095);
        }
        Err(other) => panic!("expected InputTooShort, got {other:?}"),
        Ok(_) => panic!("4095 frames must be rejected"),
    }
}

#[test]
fn wrong_profile_geometry_is_rejected() {
    let encoder = Encoder::new(fixture_selection()).expect("selection resolves");
    // 2 channels / 48 kHz: geometry differs from the 6ch/44100 profile.
    let pcm = Pcm16::new(48000, vec![vec![0i16; 5000], vec![0i16; 5000]]).expect("pcm builds");
    match encoder.encode_pcm(&pcm) {
        Err(EncoderError::GeometryMismatch { .. }) => {}
        Err(other) => panic!("expected GeometryMismatch, got {other:?}"),
        Ok(_) => panic!("wrong geometry must be rejected"),
    }
}

#[test]
fn unresolvable_selections_are_rejected() {
    // 2ch/44100 is not an installed configuration (the installed 2ch
    // profile is 48000 Hz): the selection must be rejected, never
    // satisfied by a neighbouring geometry.
    let selection = WwiseProfile::new(WwiseVersion::Wwise2013, 2, 44_100).expect("selection");
    match Encoder::new(selection) {
        Err(EncoderError::ProfileNotFound { requested }) => {
            assert_eq!(requested, "2ch/44100Hz/2013");
        }
        Err(other) => panic!("expected ProfileNotFound, got {other:?}"),
        Ok(_) => panic!("unresolvable selection must be rejected"),
    }
    match StreamSession::for_selection(selection) {
        Err(EncoderError::ProfileNotFound { requested }) => {
            assert_eq!(requested, "2ch/44100Hz/2013");
        }
        Err(other) => panic!("expected ProfileNotFound, got {other:?}"),
        Ok(_) => panic!("unresolvable selection must not open a session"),
    }
}

#[test]
fn stream_lifecycle_violations_are_state_errors() {
    // Chunk before Init.
    let mut early = StreamSession::new();
    match early.push_pcm_chunk(&[0u8; 12]) {
        Err(EncoderError::StateError { .. }) => {}
        Err(other) => panic!("chunk before Init: expected StateError, got {other:?}"),
        Ok(_) => panic!("chunk before Init must fail"),
    }

    // Finish before Init.
    match early.finish() {
        Err(EncoderError::StateError { .. }) => {}
        Err(other) => panic!("finish before Init: expected StateError, got {other:?}"),
        Ok(_) => panic!("finish before Init must fail"),
    }

    // Chunk after Finish.
    let mut session = StreamSession::for_selection(fixture_selection()).expect("session opens");
    session
        .push_pcm_chunk(&[0u8; 12 * 1000])
        .expect("chunk pushes");
    match session.finish() {
        Err(EncoderError::InputTooShort { .. }) => {}
        Err(other) => panic!("short finish: expected InputTooShort, got {other:?}"),
        Ok(_) => panic!("short stream must fail"),
    }
    match session.push_pcm_chunk(&[0u8; 12]) {
        Err(EncoderError::StateError { .. }) => {}
        Err(other) => panic!("chunk after Finish: expected StateError, got {other:?}"),
        Ok(_) => panic!("chunk after Finish must fail"),
    }
    // Terminal: a second Finish is also a state error.
    match session.finish() {
        Err(EncoderError::StateError { .. }) => {}
        Err(other) => panic!("second Finish: expected StateError, got {other:?}"),
        Ok(_) => panic!("second Finish must fail"),
    }
}

#[test]
fn chunk_partial_frame_is_a_geometry_mismatch() {
    let mut session = StreamSession::for_selection(fixture_selection()).expect("session opens");
    // 6 channels -> 12 bytes per frame; 13 leaves a trailing partial frame.
    match session.push_pcm_chunk(&[0u8; 13]) {
        Err(EncoderError::GeometryMismatch { .. }) => {}
        Err(other) => panic!("partial frame: expected GeometryMismatch, got {other:?}"),
        Ok(_) => panic!("partial frame must fail"),
    }
}

/// The result carries the container without the caller reaching into `data`:
/// `len`/`is_empty` describe it and `write_to` persists it, naming the path
/// in the error when the write fails.
#[test]
fn encode_result_length_and_write_to_round_trip() {
    let pcm =
        Pcm16::from_interleaved_le(44_100, 6, fixture_le_bytes()).expect("fixture PCM parses");
    let result = Encoder::new(fixture_selection())
        .expect("encoder builds")
        .encode_pcm(&pcm)
        .expect("encode succeeds");

    assert_eq!(result.len(), result.data.len());
    assert_eq!(
        result.len(),
        read_fixture("reference.wem").len(),
        "the container length is the committed reference's"
    );
    assert!(!result.is_empty());

    let path = std::env::temp_dir().join("wem-encode-result-write-to.wem");
    let _ = std::fs::remove_file(&path);
    result.write_to(&path).expect("write_to succeeds");
    assert_eq!(
        std::fs::read(&path).expect("written file reads"),
        result.data,
        "write_to must persist exactly the container bytes"
    );
    std::fs::remove_file(&path).expect("temporary output is removable");

    // A path that cannot be written reports the path it failed on.
    let unwritable = std::env::temp_dir().join("wem-does-not-exist-dir/out.wem");
    match result.write_to(&unwritable) {
        Err(EncoderError::Internal(wem_core::InternalError::Io { message })) => {
            assert!(
                message.contains("wem-does-not-exist-dir"),
                "the write error must name the path: {message}"
            );
        }
        other => panic!("expected an Io internal error, got {other:?}"),
    }
}
