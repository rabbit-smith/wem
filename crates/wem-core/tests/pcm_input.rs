//! PCM input contract (wem-core).
//!
//! Two things are pinned here:
//! * the rejection conditions of the byte-backed constructors
//!   (`from_channel_major_le` / `from_interleaved_le`);
//! * the shape equivalence: the same PCM handed in as channel-major i16 rows,
//!   as channel-major little-endian bytes, or as interleaved little-endian
//!   bytes must produce the identical float rows — and therefore the identical
//!   container bytes. The row shape is the reference: it is the form every
//!   caller used before the byte-backed shapes existed.

use std::path::{Path, PathBuf};

use wem_core::encoder::{Encoder, Pcm16};
use wem_core::error::EncoderError;
use wem_core::usecases::wav::read_pcm16;
use wem_core::{WwiseProfile, WwiseVersion};

/// The repository fixtures directory (repo_root/tests/fixtures).
fn fixtures_dir() -> PathBuf {
    // CARGO_MANIFEST_DIR = <root>/crates/wem-core
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../..")
        .join("tests/fixtures")
        .canonicalize()
        .expect("fixtures directory resolves")
}

/// The fixture profile selection: the installed Wwise 2013 6ch/44100
/// configuration.
fn fixture_selection() -> WwiseProfile {
    WwiseProfile::new(WwiseVersion::Wwise2013, 6, 44_100).expect("fixture selection")
}

/// Transpose interleaved little-endian s16 bytes into channel-major bytes
/// (byte level only: no `i16` participates, so the kernel's own decode is what
/// is under test).
fn channel_major_from_interleaved(interleaved: &[u8], channels: usize) -> Vec<u8> {
    let frames = interleaved.len() / (channels * 2);
    let mut out = vec![0u8; interleaved.len()];
    for frame in 0..frames {
        for channel in 0..channels {
            let src = (frame * channels + channel) * 2;
            let dst = (channel * frames + frame) * 2;
            out[dst] = interleaved[src];
            out[dst + 1] = interleaved[src + 1];
        }
    }
    out
}

#[test]
fn interleaved_pcm_rejects_unrepresentable_channel_count() {
    let error = Pcm16::from_interleaved_le(48_000, usize::MAX, &[0, 0])
        .expect_err("channel-byte multiplication must be checked");
    assert!(matches!(error, EncoderError::StateError { .. }));

    let error = Pcm16::from_interleaved_le(48_000, usize::MAX / 2, &[])
        .expect_err("empty PCM must be rejected before channel allocation");
    assert!(matches!(error, EncoderError::StateError { .. }));

    let error = Pcm16::from_channel_major_le(48_000, usize::MAX, &[0, 0])
        .expect_err("channel-byte multiplication must be checked here too");
    assert!(matches!(error, EncoderError::StateError { .. }));
}

#[test]
fn interleaved_pcm_rejects_zero_channels_and_partial_frames() {
    assert!(matches!(
        Pcm16::from_interleaved_le(48_000, 0, &[]),
        Err(EncoderError::StateError { .. })
    ));
    assert!(matches!(
        Pcm16::from_interleaved_le(48_000, 2, &[0, 0, 0]),
        Err(EncoderError::GeometryMismatch { .. })
    ));
}

/// The new shape carries exactly the validation `new` /
/// `from_interleaved_le` carry: a positive rate, at least one channel, at
/// least one frame, and a byte length that is an exact multiple of the frame
/// width (a trailing partial frame is GEOMETRY_MISMATCH).
#[test]
fn channel_major_pcm_rejects_malformed_geometry() {
    // Zero channels.
    assert!(matches!(
        Pcm16::from_channel_major_le(48_000, 0, &[]),
        Err(EncoderError::StateError { .. })
    ));
    assert!(matches!(
        Pcm16::from_channel_major_le(48_000, 0, &[0, 0]),
        Err(EncoderError::StateError { .. })
    ));
    // Zero frames (empty byte buffer).
    assert!(matches!(
        Pcm16::from_channel_major_le(48_000, 2, &[]),
        Err(EncoderError::StateError { .. })
    ));
    // 6 bytes into 2-channel frames of 4 bytes: a trailing partial frame.
    assert!(matches!(
        Pcm16::from_channel_major_le(48_000, 2, &[0, 0, 0, 0, 0, 0]),
        Err(EncoderError::GeometryMismatch { .. })
    ));
    // Non-positive rates are rejected, exactly as `new` rejects them.
    for sample_rate in [0, -44_100] {
        assert!(matches!(
            Pcm16::from_channel_major_le(sample_rate, 2, &[0, 0, 0, 0]),
            Err(EncoderError::StateError { .. })
        ));
        assert!(matches!(
            Pcm16::from_interleaved_le(sample_rate, 2, &[0, 0, 0, 0]),
            Err(EncoderError::StateError { .. })
        ));
    }
}

/// Small, non-square, every sample distinct (both signed-16 extremes
/// included): a transposition or an off-by-one offset would land a different
/// value at some `(channel, frame)` — and the expected rows spell out the
/// normalization statement the kernel must use.
#[test]
fn the_three_pcm_shapes_share_one_float_row_view() {
    let channels = 3usize;
    let frames = 5usize;
    let sample_rate = 44_100i64;

    let mut values: Vec<Vec<i16>> = (0..channels)
        .map(|channel| {
            (0..frames)
                .map(|frame| ((channel * frames + frame) as i16) * 997 - 2_000)
                .collect()
        })
        .collect();
    values[0][0] = i16::MIN;
    values[channels - 1][frames - 1] = i16::MAX;

    let expected: Vec<Vec<f64>> = values
        .iter()
        .map(|row| row.iter().map(|value| *value as f64 / 32768.0).collect())
        .collect();
    let channel_major: Vec<u8> = values
        .iter()
        .flatten()
        .flat_map(|value| value.to_le_bytes())
        .collect();
    let mut interleaved = Vec::with_capacity(channels * frames * 2);
    for frame in 0..frames {
        for row in &values {
            interleaved.extend_from_slice(&row[frame].to_le_bytes());
        }
    }

    let row_pcm = Pcm16::new(sample_rate, values).expect("rows build");
    let channel_pcm =
        Pcm16::from_channel_major_le(sample_rate, channels, &channel_major).expect("bytes build");
    let interleaved_pcm =
        Pcm16::from_interleaved_le(sample_rate, channels, &interleaved).expect("bytes build");

    // Geometry is reported identically by every shape.
    for pcm in [&row_pcm, &channel_pcm, &interleaved_pcm] {
        assert_eq!(pcm.sample_rate(), sample_rate);
        assert_eq!(pcm.channel_count(), channels);
        assert_eq!(pcm.frame_count(), frames as i64);
    }

    // The whole point: the float rows are identical, sample for sample.
    for (name, pcm) in [
        ("channel-major bytes", &channel_pcm),
        ("interleaved bytes", &interleaved_pcm),
    ] {
        assert_eq!(
            pcm.to_float_rows(),
            expected,
            "{name} float rows differ from the row-major reference"
        );
    }
    assert_eq!(row_pcm.to_float_rows(), expected);

    // Equality is shape-independent: same rate, geometry and samples.
    assert_eq!(row_pcm, channel_pcm);
    assert_eq!(row_pcm, interleaved_pcm);
    assert_ne!(
        row_pcm,
        Pcm16::new(sample_rate, vec![vec![0i16; frames]; channels]).expect("zeros build")
    );
}

/// The fixture PCM in all three shapes: one encode each, byte-identical to the
/// golden container (the row shape's bytes are the reference).
#[test]
fn the_three_pcm_shapes_encode_to_identical_containers() {
    let wav = read_pcm16(&fixtures_dir().join("input.wav")).expect("input.wav reads");
    assert_eq!(wav.channels(), 6);
    assert_eq!(wav.sample_rate(), 44_100);
    let interleaved = wav.interleaved_le_bytes();
    let channel_major = channel_major_from_interleaved(&interleaved, wav.channels());

    let rows: Vec<Vec<i16>> = (0..wav.channels())
        .map(|channel| {
            (0..wav.frames())
                .map(|frame| {
                    let offset = (frame * wav.channels() + channel) * 2;
                    i16::from_le_bytes([interleaved[offset], interleaved[offset + 1]])
                })
                .collect()
        })
        .collect();

    let row_pcm = Pcm16::new(44_100, rows).expect("rows build");
    let channel_pcm = Pcm16::from_channel_major_le(44_100, wav.channels(), &channel_major)
        .expect("channel-major bytes build");
    let interleaved_pcm = Pcm16::from_interleaved_le(44_100, wav.channels(), &interleaved)
        .expect("interleaved bytes build");
    assert_eq!(row_pcm, channel_pcm);
    assert_eq!(row_pcm, interleaved_pcm);

    let encoder = Encoder::new(fixture_selection()).expect("selection resolves");
    let reference = row_pcm.to_float_rows();
    let mut containers = Vec::new();
    for (name, pcm) in [
        ("row-major", &row_pcm),
        ("channel-major bytes", &channel_pcm),
        ("interleaved bytes", &interleaved_pcm),
    ] {
        assert_eq!(
            pcm.to_float_rows(),
            reference,
            "{name} float rows differ from the row-major reference"
        );
        let result = encoder.encode_pcm(pcm).expect("encode runs");
        containers.push((name, result.data));
    }

    let golden = std::fs::read(fixtures_dir().join("reference.wem")).expect("reference.wem reads");
    for (name, bytes) in &containers {
        assert_eq!(
            bytes, &golden,
            "{name} container differs from reference.wem at file level"
        );
    }
    assert_eq!(containers[0].1, containers[1].1);
    assert_eq!(containers[0].1, containers[2].1);
}

/// A geometry that disagrees with the profile is rejected whichever shape the
/// PCM arrived in (the shape affects storage, never validation).
#[test]
fn wrong_profile_geometry_rejects_every_pcm_shape() {
    let encoder = Encoder::new(fixture_selection()).expect("selection resolves");
    // 2 channels / 48 kHz: geometry differs from the 6ch/44100 profile.
    let frames = 5_000usize;
    let channel_major = vec![0u8; frames * 2 * 2];
    let mut interleaved = vec![0u8; frames * 2 * 2];
    for frame in 0..frames {
        // Distinct per (channel, frame): a silent buffer would not show that
        // the geometry check runs before any sample is read.
        interleaved[frame * 4] = (frame % 251) as u8;
        interleaved[frame * 4 + 2] = (frame % 253) as u8;
    }

    let shapes = [
        (
            "rows",
            Pcm16::new(48_000, vec![vec![0i16; frames], vec![1i16; frames]]).expect("rows build"),
        ),
        (
            "channel-major bytes",
            Pcm16::from_channel_major_le(48_000, 2, &channel_major).expect("bytes build"),
        ),
        (
            "interleaved bytes",
            Pcm16::from_interleaved_le(48_000, 2, &interleaved).expect("bytes build"),
        ),
    ];
    for (name, pcm) in shapes {
        match encoder.encode_pcm(&pcm) {
            Err(EncoderError::GeometryMismatch { .. }) => {}
            Err(other) => panic!("{name}: expected GeometryMismatch, got {other:?}"),
            Ok(_) => panic!("{name}: wrong geometry must be rejected"),
        }
    }
}
