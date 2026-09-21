//! 28-frame packet parity against the stage-golden pipeline assets.
//!
//! For every representative frame in `tests/data/stage-golden/stages` the
//! packet path is re-run end to end:
//!
//!   f{NNN}.post.f32le.bin      → floor1_fit_wwise fitted curve (analysis.post)
//!   f{NNN}.raw_mdct.f32le.bin  → floor1_fit_wwise raw MDCT curve
//!   f{NNN}.side.f32le.bin      → pack_block_packet_details mdct rows
//!   index.json `mode`          → block mode
//!
//! The produced packet bytes must hash to the recorded `packet_sha256`, and
//! the intermediate floor-posts / residue-integer surfaces must hash to the
//! recorded `floor_posts_sha256` / `residue_q_after_sha256`
//! (Python `_hash_int_rows`: `0xff` byte for a None row, else `0x00` byte
//! plus i32le words).

use std::path::{Path, PathBuf};

use serde_json::Value;
use sha2::{Digest, Sha256};
use wem_profiles::{assemble_encoder_profile_resources, load_profile_bundle, DataDir};
use wem_vorbis::floor_fit::floor1_fit_wwise;
use wem_vorbis::packet_encoder::pack_block_packet_details;

const SCHEMA: &str = "wwise-wem.stage-golden.v1";

fn repo_root() -> PathBuf {
    // crates/wem-vorbis -> repo root (two levels up from the manifest dir).
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .expect("crates dir")
        .parent()
        .expect("repo root")
        .to_path_buf()
}

fn stages_dir() -> PathBuf {
    repo_root().join("tests/data/stage-golden/stages")
}

fn sha256_hex(payload: &[u8]) -> String {
    format_digest_hex(&Sha256::digest(payload))
}

/// Format an already-computed digest byte string as lowercase hex.
fn format_digest_hex(digest: &[u8]) -> String {
    const DIGITS: &[u8; 16] = b"0123456789abcdef";
    let mut out = String::new();
    for &b in digest {
        out.push(DIGITS[(b >> 4) as usize] as char);
        out.push(DIGITS[(b & 0xF) as usize] as char);
    }
    out
}

/// Read one stage dump; verify its bytes against the index entry.
fn read_dump(stages: &Path, entry: &Value, key: &str) -> Vec<u8> {
    let path = stages.join(entry["path"].as_str().unwrap());
    let bytes =
        std::fs::read(&path).unwrap_or_else(|e| panic!("read dump {}: {e}", path.display()));
    let digest = sha256_hex(&bytes);
    assert_eq!(
        digest,
        entry["sha256"].as_str().unwrap(),
        "dump {key} bytes differ from index.json"
    );
    assert_eq!(
        bytes.len(),
        entry["size"].as_u64().unwrap() as usize,
        "dump {key} size"
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

/// f32le dump → the (channel-major) rows for a named stage of one frame.
fn stage_rows(
    index: &Value,
    stages: &Path,
    frame_index: usize,
    stage: &str,
    channels: usize,
) -> Vec<Vec<f32>> {
    let key = format!("f{frame_index:03}.{stage}");
    let entry = index["dumps"]
        .get(&key)
        .unwrap_or_else(|| panic!("missing dump entry {key}"));
    let bytes = read_dump(stages, entry, &key);
    f32_rows(&bytes, channels)
}

/// Python `_hash_int_rows`: None row → `0xff`; else `0x00` + i32le words.
fn hash_int_rows(rows: &[Option<Vec<i64>>]) -> String {
    let mut hasher = Sha256::new();
    for row in rows {
        match row {
            None => hasher.update([0xFFu8]),
            Some(row) => {
                hasher.update([0x00u8]);
                for &value in row {
                    hasher.update((value as i32).to_le_bytes());
                }
            }
        }
    }
    format_digest_hex(hasher.finalize().as_slice())
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
            .expect("stage-golden index.json exists");
        serde_json::from_str(&raw).expect("index parses")
    };
    assert_eq!(index["schema"].as_str(), Some(SCHEMA));
    assert_eq!(index["profile"].as_str(), Some("wwise2013-6ch-44100"));
    assert_eq!(index["audio_packets"].as_u64(), Some(205));
}

#[test]
fn stage_frames_packet_parity_all_28_representatives() {
    let raw = std::fs::read_to_string(stages_dir().join("index.json"))
        .expect("stage-golden index.json exists");
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
        "stage-golden must carry 28 representative frames"
    );

    let stages = stages_dir();
    let res = {
        let data = DataDir::from_profiles_dir(
            std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
                .join("../..")
                .join("src/wwise_wem/data/profiles"),
        );
        let bundle = load_profile_bundle(&data, None, false).expect("installed profile loads");
        assemble_encoder_profile_resources(&bundle, None, None).expect("assembly succeeds")
    };
    // The setup packet must match the recorded setup hash.
    assert_eq!(
        sha256_hex(&res.setup_packet),
        index["container"]["setup_sha256"].as_str().unwrap(),
        "setup packet hash differs"
    );

    for &frame_index in &reps {
        let frame = frames
            .iter()
            .find(|f| f["index"].as_u64() == Some(frame_index as u64))
            .unwrap_or_else(|| panic!("frame {frame_index} missing from index"));
        let expected_packet_sha = frame["packet_sha256"].as_str().unwrap().to_string();
        let expected_packet_size = frame["packet_size"].as_u64().unwrap() as usize;

        let (packet, posts, quantized_residue) = pack_frame(&res, &index, &stages, frame, channels);

        assert_eq!(
            packet.len(),
            expected_packet_size,
            "frame {frame_index}: packet size differs (got {})",
            packet.len()
        );
        let actual_sha = sha256_hex(&packet);
        assert_eq!(
            actual_sha, expected_packet_sha,
            "frame {frame_index}: packet SHA-256 differs\n  expected: {expected_packet_sha}\n  actual:   {actual_sha}"
        );

        // floor_posts parity (Python `_hash_int_rows` encoding).
        let expected_posts_sha = frame["floor_posts_sha256"].as_str().unwrap();
        let posts_for_hash: Vec<Option<Vec<i64>>> = posts.clone();
        assert_eq!(
            hash_int_rows(&posts_for_hash),
            expected_posts_sha,
            "frame {frame_index}: floor_posts hash differs"
        );

        // residue_q parity (Python `_hash_int_rows` encoding, rows never None).
        let expected_residue_sha = frame["residue_q_after_sha256"].as_str().unwrap();
        let residue_for_hash: Vec<Option<Vec<i64>>> = quantized_residue
            .iter()
            .map(|row| Some(row.clone()))
            .collect();
        assert_eq!(
            hash_int_rows(&residue_for_hash),
            expected_residue_sha,
            "frame {frame_index}: residue_q hash differs"
        );
    }
}
