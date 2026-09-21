//! The installed 2ch/48000 selection end to end: the same committed reference
//! WEM the batch encoder and the streaming session must produce.
//!
//! The profile bundle that drives both paths is resolved with
//! `wem_profiles::bundle_for_selection` — a structured selection against the
//! compiled-in profile bundle, never a profile name or a profile tree.

use wem_core::encoder::Encoder;
use wem_core::stream::StreamSession;
use wem_core::usecases::wav::read_pcm16;
use wem_core::{WwiseProfile, WwiseVersion};

const TWO_CHANNEL_PROFILE_NAME: &str = "wwise2013-2ch-48000";
const TWO_CHANNEL_SETUP_SHA256: &str =
    "894a545ca48993bb0e5b768b1a367fd4475f806658b51bbcc88c8a6243849afc";

/// The installed Wwise 2013 2ch/48000 configuration.
fn two_channel_selection() -> WwiseProfile {
    WwiseProfile::new(WwiseVersion::Wwise2013, 2, 48_000).expect("2ch/48000 selection")
}

fn two_channel_dir() -> std::path::PathBuf {
    std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../..")
        .join("tests/data/2ch-reference")
}

#[test]
fn two_channel_selection_encodes_the_committed_two_channel_wem() {
    let encoder = Encoder::new(two_channel_selection()).expect("2ch selection resolves");
    assert_eq!(encoder.profile().name(), TWO_CHANNEL_PROFILE_NAME);
    assert_eq!(encoder.profile().setup_sha256(), TWO_CHANNEL_SETUP_SHA256);

    let wav = read_pcm16(&two_channel_dir().join("tone_high.wav")).expect("2ch WAV reads");
    let pcm = wav.to_pcm16().expect("2ch WAV converts to Pcm16");
    let result = encoder.encode_pcm(&pcm).expect("2ch encode runs");
    assert_eq!(
        result.data,
        std::fs::read(two_channel_dir().join("tone_high.wem")).unwrap()
    );
}

#[test]
fn two_channel_selection_streams_the_committed_two_channel_wem() {
    let wav = read_pcm16(&two_channel_dir().join("tone_high.wav")).expect("2ch WAV reads");
    let le_bytes = wav.interleaved_le_bytes();
    let bytes_per_frame = wav.channels() * 2;
    let mut session =
        StreamSession::for_selection(two_channel_selection()).expect("2ch session opens");

    // Two uneven chunks: chunk boundaries must not affect the bytes.
    let cut = 12_345 * bytes_per_frame;
    session.push_pcm_chunk(&le_bytes[..cut]).expect("chunk one");
    session.push_pcm_chunk(&le_bytes[cut..]).expect("chunk two");
    let result = session.finish().expect("2ch stream finishes");

    assert_eq!(
        result.data,
        std::fs::read(two_channel_dir().join("tone_high.wem")).unwrap()
    );
}
