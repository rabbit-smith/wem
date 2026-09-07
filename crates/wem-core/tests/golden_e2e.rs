//! End-to-end golden parity test: the Rust kernel must reproduce the
//! reference WEM byte-for-byte (the P2-4 gate).
//!
//! Oracle: `tests/fixtures/reference.wem`, produced by the Python encoder
//! from `tests/fixtures/input.wav`
//! (SHA-256 17851d26c6210b85e498ae0452d2562d7b9e2c3e9e795c459656b9c9d8d35247).
//!
//! This test asserts file-level byte equality (Vec<u8> ==), not just
//! matching hashes.

use std::path::{Path, PathBuf};

use wem_core::encoder::{Encoder, Pcm16};
use wem_core::error::EncoderError;
use wem_core::stream::{ProfileRef, StreamSession};
use wem_core::usecases::wav::read_pcm16;

const PROFILE_NAME: &str = "wwise2013-6ch-44100";
const REFERENCE_SHA256: &str = "17851d26c6210b85e498ae0452d2562d7b9e2c3e9e795c459656b9c9d8d35247";

/// The repository fixtures directory (repo_root/tests/fixtures).
fn fixtures_dir() -> PathBuf {
    // CARGO_MANIFEST_DIR = <root>/crates/wem-core
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../..")
        .join("tests/fixtures")
        .canonicalize()
        .expect("fixtures directory resolves")
}

fn read_fixture(name: &str) -> Vec<u8> {
    std::fs::read(fixtures_dir().join(name)).expect("fixture file reads")
}

/// Read the fixture WAV and hand it to the encoder the way the CLI does.
fn encode_fixture() -> (wem_core::EncodeResult, Encoder) {
    let wav = read_pcm16(&fixtures_dir().join("input.wav")).expect("input.wav reads");
    assert_eq!(wav.channels(), 6);
    assert_eq!(wav.sample_rate(), 44100);
    assert_eq!(wav.frames(), 139398);
    let encoder = Encoder::from_profile(PROFILE_NAME).expect("profile loads");
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
    // Sanity: the reference fixture itself is the expected golden WEM.
    let digest = {
        use sha2::{Digest, Sha256};
        wem_profiles::resources::hex(Sha256::digest(&reference))
    };
    assert_eq!(digest, REFERENCE_SHA256, "reference.wem fixture drifted");
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
    assert_eq!(stats.metadata_source, "profile:wwise2013-6ch-44100");
    assert_eq!(result.sha256(), REFERENCE_SHA256);
}

// ---------------------------------------------------------------------------
// 2. StreamSession: chunk boundaries must not affect the output bytes
// ---------------------------------------------------------------------------

fn fixture_le_bytes() -> Vec<u8> {
    let wav = read_pcm16(&fixtures_dir().join("input.wav")).expect("input.wav reads");
    wav.interleaved_le_bytes()
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

    let reference_sha = "3ef56cbd6e6a66a5474005db05912624487faa555fb2cdfed130f606b322e4e3";
    let ref_ = ProfileRef::with_name(reference_sha, PROFILE_NAME);
    let mut session = StreamSession::for_profile_ref(&ref_).expect("init");
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
    // The container walk exposes the wwise.v1 packet stream: the setup
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
        "data chunk payload does not start with the wwise.v1 packet stream"
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
    let bytes: Vec<u8> = wav.interleaved_le_bytes()[0..frames * samples_per_frame].to_vec();
    let pcm = Pcm16::from_interleaved_le(44100, 6, &bytes).expect("pcm parses");
    let encoder = Encoder::from_profile(PROFILE_NAME).expect("profile loads");
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
    let ref_ = ProfileRef {
        setup_sha256: "3ef56cbd6e6a66a5474005db05912624487faa555fb2cdfed130f606b322e4e3"
            .to_string(),
        name: None,
    };
    let mut session = StreamSession::for_profile_ref(&ref_).expect("init");
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
    let encoder = Encoder::from_profile(PROFILE_NAME).expect("profile loads");
    // 2 channels / 48 kHz: geometry differs from the 6ch/44100 profile.
    let pcm = Pcm16::new(48000, vec![vec![0i16; 5000], vec![0i16; 5000]]).expect("pcm builds");
    match encoder.encode_pcm(&pcm) {
        Err(EncoderError::GeometryMismatch { .. }) => {}
        Err(other) => panic!("expected GeometryMismatch, got {other:?}"),
        Ok(_) => panic!("wrong geometry must be rejected"),
    }
}

#[test]
fn unknown_profile_names_are_rejected() {
    match Encoder::from_profile("does-not-exist") {
        Err(EncoderError::ProfileNotFound { requested }) => {
            assert_eq!(requested, "does-not-exist");
        }
        Err(other) => panic!("expected ProfileNotFound, got {other:?}"),
        Ok(_) => panic!("unknown profile must be rejected"),
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

    // Second Init is a state error.
    let ref_ = ProfileRef {
        setup_sha256: "3ef56cbd6e6a66a5474005db05912624487faa555fb2cdfed130f606b322e4e3"
            .to_string(),
        name: None,
    };
    let mut session = StreamSession::for_profile_ref(&ref_).expect("init");
    match session.init_profile(&ref_) {
        Err(EncoderError::StateError { .. }) => {}
        Err(other) => panic!("second Init: expected StateError, got {other:?}"),
        Ok(_) => panic!("second Init must fail"),
    }

    // Soft name cross-check failure.
    let wrong_name = ProfileRef::with_name(
        "3ef56cbd6e6a66a5474005db05912624487faa555fb2cdfed130f606b322e4e3",
        "another-profile",
    );
    match StreamSession::for_profile_ref(&wrong_name) {
        Err(EncoderError::StateError { .. }) => {}
        Err(other) => panic!("name mismatch: expected StateError, got {other:?}"),
        Ok(_) => panic!("name mismatch must fail"),
    }

    // No profile matches a bogus digest.
    let bogus = ProfileRef {
        setup_sha256: "0000000000000000000000000000000000000000000000000000000000000000"
            .to_string(),
        name: None,
    };
    match StreamSession::for_profile_ref(&bogus) {
        Err(EncoderError::ProfileNotFound { .. }) => {}
        Err(other) => panic!("bogus digest: expected ProfileNotFound, got {other:?}"),
        Ok(_) => panic!("bogus digest must fail"),
    }

    // Chunk after Finish.
    let ref_ = ProfileRef {
        setup_sha256: "3ef56cbd6e6a66a5474005db05912624487faa555fb2cdfed130f606b322e4e3"
            .to_string(),
        name: None,
    };
    let mut session = StreamSession::for_profile_ref(&ref_).expect("init");
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
    let ref_ = ProfileRef {
        setup_sha256: "3ef56cbd6e6a66a5474005db05912624487faa555fb2cdfed130f606b322e4e3"
            .to_string(),
        name: None,
    };
    let mut session = StreamSession::for_profile_ref(&ref_).expect("init");
    // 6 channels -> 12 bytes per frame; 13 leaves a trailing partial frame.
    match session.push_pcm_chunk(&[0u8; 13]) {
        Err(EncoderError::GeometryMismatch { .. }) => {}
        Err(other) => panic!("partial frame: expected GeometryMismatch, got {other:?}"),
        Ok(_) => panic!("partial frame must fail"),
    }
}
