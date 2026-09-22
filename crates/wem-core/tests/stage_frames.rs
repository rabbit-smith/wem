//! 28-frame packet parity against the stage-records pipeline assets.
//!
//! For every representative frame in `tests/data/stage-records/stages` the
//! packet path is re-run end to end:
//!
//!   f{NNN}.post.f32le.bin      → floor1_fit_wwise fitted curve (analysis.post)
//!   f{NNN}.raw_mdct.f32le.bin  → floor1_fit_wwise raw MDCT curve
//!   f{NNN}.side.f32le.bin      → pack_block_packet_details mdct rows
//!   index.json `mode`          → block mode
//!
//! The produced packet bytes must equal the recorded `f{NNN}.packet.u8.bin`,
//! the floor posts must equal the recorded `f{NNN}.floor_posts.u16le.bin` and
//! the residue integers must equal the recorded `f{NNN}.residue_q.i32le.bin`,
//! byte for byte. The comparison is against the dumps themselves, so nothing
//! stands between the values and the assertion.

use std::path::{Path, PathBuf};

use serde_json::Value;
use wem_profiles::{assemble_encoder_profile_resources, compiled_profile_for_selection};
use wem_vorbis::floor_fit::floor1_fit_wwise;
use wem_vorbis::packet_encoder::pack_block_packet_details;

mod common;

use common::{fixture_selection, read_fixture, repo_root};

const SCHEMA: &str = "wwise-wem.stage-records.v1";

/// Encoder resources of the installed Wwise 2013 6ch/44100 configuration,
/// resolved from a structured selection against the compiled profile carrier
/// (never a profile name or a profile tree).
fn encoder_resources() -> wem_profiles::EncoderProfileResources {
    let compiled = compiled_profile_for_selection(fixture_selection())
        .expect("installed 6ch profile resolves");
    assemble_encoder_profile_resources(&compiled, None, None).expect("assembly succeeds")
}

fn stages_dir() -> PathBuf {
    repo_root().join("tests/data/stage-records/stages")
}

/// Read one stage dump, checking it against the index entry's recorded size.
fn read_dump(stages: &Path, entry: &Value, key: &str) -> Vec<u8> {
    let path = stages.join(entry["path"].as_str().unwrap());
    let bytes =
        std::fs::read(&path).unwrap_or_else(|e| panic!("read dump {}: {e}", path.display()));
    assert_eq!(
        bytes.len(),
        entry["size"].as_u64().unwrap() as usize,
        "dump {key} size differs from index.json"
    );
    bytes
}

/// f32le dump → channel-major rows.
fn f32_rows(bytes: &[u8], channels: usize) -> Vec<Vec<f32>> {
    let total = bytes.len() / 4;
    assert_eq!(
        total % channels,
        0,
        "f32 dump length {total}/4 not a multiple of {channels} channels"
    );
    let per_channel = total / channels;
    (0..channels)
        .map(|ch| {
            (0..per_channel)
                .map(|i| {
                    let off = (ch * per_channel + i) * 4;
                    f32::from_bits(u32::from_le_bytes(bytes[off..off + 4].try_into().unwrap()))
                })
                .collect()
        })
        .collect()
}

/// One named stage dump of one frame, with its index entry's byte length.
fn stage_dump(index: &Value, stages: &Path, frame_index: usize, stage: &str) -> Vec<u8> {
    let key = format!("f{frame_index:03}.{stage}");
    let entry = index["dumps"]
        .get(&key)
        .unwrap_or_else(|| panic!("missing dump entry {key}"));
    read_dump(stages, entry, &key)
}

/// f32le dump → the (channel-major) rows for a named stage of one frame.
fn stage_rows(
    index: &Value,
    stages: &Path,
    frame_index: usize,
    stage: &str,
    channels: usize,
) -> Vec<Vec<f32>> {
    f32_rows(&stage_dump(index, stages, frame_index, stage), channels)
}

/// Where two byte strings part company: the first differing byte and both
/// lengths, or that only the lengths differ.
fn difference(left: &[u8], right: &[u8]) -> String {
    let shared = left.len().min(right.len());
    match (0..shared).find(|&index| left[index] != right[index]) {
        Some(index) => format!(
            "first difference at byte {index}: recorded 0x{:02x}, produced 0x{:02x} \
             (lengths {} vs {})",
            left[index],
            right[index],
            left.len(),
            right.len()
        ),
        None if left.len() == right.len() => "identical".to_string(),
        None => format!(
            "lengths differ: recorded {} bytes, produced {} bytes",
            left.len(),
            right.len()
        ),
    }
}

/// Python `pack_posts`: u16le words, `0xffff` for an unused channel.
fn pack_posts(posts: &[Option<Vec<i64>>]) -> Vec<u8> {
    let mut out = Vec::new();
    for row in posts {
        match row {
            None => out.extend_from_slice(&0xFFFFu16.to_le_bytes()),
            Some(row) => {
                for &value in row {
                    out.extend_from_slice(&(value as u16).to_le_bytes());
                }
            }
        }
    }
    out
}

/// Python `pack_int_rows`: packed i32le words, channel-major.
fn pack_int_rows(rows: &[Vec<i64>]) -> Vec<u8> {
    let mut out = Vec::new();
    for row in rows {
        for &value in row {
            out.extend_from_slice(&(value as i32).to_le_bytes());
        }
    }
    out
}

/// Pack one frame the way `pack_analysis_frame` does (oracle input shapes),
/// returning (packet, posts_per_channel, quantized_residue).
#[allow(clippy::type_complexity)] // 3-tuple mirrors the Python pipeline result
fn pack_frame(
    res: &wem_profiles::EncoderProfileResources,
    index: &Value,
    stages: &Path,
    frame: &Value,
    channels: u32,
) -> (Vec<u8>, Vec<Option<Vec<i64>>>, Vec<Vec<i64>>) {
    let frame_index = frame["index"].as_u64().expect("frame index") as usize;
    let mode = frame["mode"].as_u64().expect("frame mode") as u32;

    let post = stage_rows(index, stages, frame_index, "post", channels as usize);
    let raw_mdct = stage_rows(index, stages, frame_index, "raw_mdct", channels as usize);
    let side = stage_rows(index, stages, frame_index, "side", channels as usize);

    // posts per channel: floor1_fit_wwise(post[ch], raw_mdct[ch], floor, n)
    let mapping = &res.setup.maps[res.setup.modes[mode as usize].mapping as usize];
    let mut posts: Vec<Option<Vec<i64>>> = Vec::with_capacity(channels as usize);
    for ch in 0..channels {
        let submap = if mapping.submaps > 1 {
            mapping.chmux[ch as usize]
        } else {
            0
        };
        let floor = &res.setup.floors[mapping.floors[submap as usize] as usize];
        let fit = floor1_fit_wwise(
            &post[ch as usize],
            &raw_mdct[ch as usize],
            floor,
            Some(raw_mdct[ch as usize].len()),
        )
        .unwrap_or_else(|e| panic!("frame {frame_index}: floor fit failed: {e}"));
        posts.push(fit);
    }

    let result = pack_block_packet_details(
        &res.setup,
        &res.codebooks,
        channels,
        mode,
        &posts,
        &side,
        None,
        true,
        true,
    )
    .unwrap_or_else(|e| panic!("frame {frame_index}: packet packing failed: {e}"));

    (result.packet, posts, result.quantized_residue)
}

#[test]
fn index_schema_is_current() {
    let index: Value = {
        let raw = std::fs::read_to_string(stages_dir().join("index.json"))
            .expect("stage-records index.json exists");
        serde_json::from_str(&raw).expect("index parses")
    };
    assert_eq!(index["schema"].as_str(), Some(SCHEMA));
    // The recorded profile label is the label the fixture selection resolves
    // to: derived from the identity, never typed into the test.
    let resolved = wem_profiles::resolve_wem_profile_selection(fixture_selection())
        .expect("installed 6ch profile resolves");
    assert_eq!(index["profile"].as_str(), Some(resolved.label().as_str()));
    assert_eq!(index["audio_packets"].as_u64(), Some(205));
}

#[test]
fn stage_frames_packet_parity_all_28_representatives() {
    let raw = std::fs::read_to_string(stages_dir().join("index.json"))
        .expect("stage-records index.json exists");
    let index: Value = serde_json::from_str(&raw).expect("index parses");
    let channels = index["channels"].as_u64().unwrap() as u32;
    let frames: Vec<&Value> = index["frames"].as_array().unwrap().iter().collect();
    let reps: Vec<usize> = index["representative_frames"]
        .as_array()
        .unwrap()
        .iter()
        .map(|v| v.as_u64().unwrap() as usize)
        .collect();
    assert_eq!(
        reps.len(),
        28,
        "stage-records must carry 28 representative frames"
    );

    let stages = stages_dir();
    let res = encoder_resources();
    // The setup packet must be the one the committed reference container
    // carries, compared as bytes (include/wem.h frames it as a u16 LE length
    // followed by the packet bytes).
    let reference = read_fixture("reference.wem");
    let setup = &res.setup_packet;
    let length = u16::try_from(setup.len()).expect("a reply packet length is a u16");
    let framed: Vec<u8> = length
        .to_le_bytes()
        .into_iter()
        .chain(setup.iter().copied())
        .collect();
    assert!(
        reference
            .windows(framed.len())
            .any(|window| window == framed),
        "the committed reference container must frame the profile's setup packet"
    );

    for &frame_index in &reps {
        let frame = frames
            .iter()
            .find(|f| f["index"].as_u64() == Some(frame_index as u64))
            .unwrap_or_else(|| panic!("frame {frame_index} missing from index"));
        let expected_packet_size = frame["packet_size"].as_u64().unwrap() as usize;

        let (packet, posts, quantized_residue) = pack_frame(&res, &index, &stages, frame, channels);

        assert_eq!(
            packet.len(),
            expected_packet_size,
            "frame {frame_index}: packet size differs (got {})",
            packet.len()
        );
        let recorded_packet = stage_dump(&index, &stages, frame_index, "packet");
        assert_eq!(
            difference(&recorded_packet, &packet),
            "identical",
            "frame {frame_index}: packet bytes differ"
        );

        // floor posts (Python `pack_posts` encoding).
        let recorded_posts = stage_dump(&index, &stages, frame_index, "floor_posts");
        assert_eq!(
            difference(&recorded_posts, &pack_posts(&posts)),
            "identical",
            "frame {frame_index}: floor_posts bytes differ"
        );

        // residue_q (Python `pack_int_rows` encoding, rows never None).
        let recorded_residue = stage_dump(&index, &stages, frame_index, "residue_q");
        assert_eq!(
            difference(&recorded_residue, &pack_int_rows(&quantized_residue)),
            "identical",
            "frame {frame_index}: residue_q bytes differ"
        );
    }
}
