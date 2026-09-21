//! The installed 2ch/48000 selection end to end: the same committed reference
//! WEM the batch encoder and the streaming session must produce.
//!
//! The profile bundle that drives both paths is resolved with
//! `wem_profiles::bundle_for_selection` — a structured selection against the
//! compiled-in profile bundle, never a profile name or a profile tree. The
//! profile name and the setup digest are properties read off that tree; the
//! bytes compared below are the claim.

use wem_core::encoder::Encoder;
use wem_core::stream::StreamSession;
use wem_core::usecases::wav::read_pcm16;
use wem_profiles::resolve_wem_profile_selection;

mod common;

use common::{two_channel_dir, two_channel_selection};

#[test]
fn two_channel_selection_encodes_the_committed_two_channel_wem() {
    let encoder = Encoder::new(two_channel_selection()).expect("2ch selection resolves");
    // The profile name is a property read off the tree the selection
    // resolves to, never a literal: the encoder and the free resolver must
    // name the same profile.
    let resolved = resolve_wem_profile_selection(two_channel_selection()).expect("2ch resolves");
    assert_eq!(encoder.profile().name(), resolved.name());

    let wav = read_pcm16(&two_channel_dir().join("tone_high.wav")).expect("2ch WAV reads");
    let pcm = wav.to_pcm16().expect("2ch WAV converts to Pcm16");
    let result = encoder.encode_pcm(&pcm).expect("2ch encode runs");
    // Byte equality covers the profile identity and its setup packet too:
    // the seq-0 packet of this container *is* the 2ch/48k setup packet.
    assert_eq!(
        result.data,
        std::fs::read(two_channel_dir().join("tone_high.wem")).expect("tone_high.wem reads")
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
        std::fs::read(two_channel_dir().join("tone_high.wem")).expect("tone_high.wem reads")
    );
}
