//! Stage-golden parity test: validates the Rust analysis kernel against the
//! byte-exact reference assets captured from the Python encoder.
//!
//! For every one of the 205 frames it checks the mode/previous/current/
//! following/window_center fields against `index.json`, and for the 28
//! representative frames it compares the eight float stages (window,
//! coefficients, raw_mdct, fft, remap, seed, post, side) byte-for-byte
//! against the frozen `f32le` dumps.

use std::path::Path;

use serde_json::Value;
use wem_analysis::model::PsyFrame;
use wem_analysis::session::AnalysisSession;

const FLOAT_STAGES: [&str; 8] = [
    "window",
    "coefficients",
    "raw_mdct",
    "fft",
    "remap",
    "seed",
    "post",
    "side",
];

fn repo_root() -> std::path::PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .expect("crates dir")
        .parent()
        .expect("repo root")
        .to_path_buf()
}

fn stage_dir() -> std::path::PathBuf {
    repo_root().join("tests/data/stage-golden/stages")
}

fn read_index() -> Value {
    let path = stage_dir().join("index.json");
    let text = std::fs::read_to_string(path).expect("index.json reads");
    serde_json::from_str(&text).expect("index.json parses")
}

fn int_field(v: &Value, field: &str) -> i64 {
    v.get(field)
        .and_then(Value::as_i64)
        .unwrap_or_else(|| panic!("frame field {field}"))
}

/// Read an uncompressed signed-16 PCM WAV into channel-major f64 values.
/// Mirrors Python `read_pcm16` (`value / 32768.0`).
#[allow(clippy::needless_range_loop)]
fn read_pcm16(path: &Path) -> (Vec<Vec<f64>>, i64, i64) {
    let bytes = std::fs::read(path).expect("input.wav reads");
    // RIFF....WAVE
    assert_eq!(&bytes[0..4], b"RIFF", "RIFF marker");
    assert_eq!(&bytes[8..12], b"WAVE", "WAVE marker");
    let mut off = 12usize;
    let mut channels = 0i64;
    let mut sample_rate = 0i64;
    let (data_off, data_size) = loop {
        let chunk_id = &bytes[off..off + 4];
        let chunk_size = u32::from_le_bytes(bytes[off + 4..off + 8].try_into().unwrap())
            as usize;
        off += 8;
        if chunk_id == b"fmt " {
            let audio_format =
                u16::from_le_bytes(bytes[off..off + 2].try_into().unwrap());
            assert_eq!(audio_format, 1, "PCM format");
            channels = i64::from(u16::from_le_bytes(bytes[off + 2..off + 4].try_into().unwrap()));
            sample_rate =
                i64::from(u32::from_le_bytes(bytes[off + 4..off + 8].try_into().unwrap()));
            let bits = u16::from_le_bytes(bytes[off + 14..off + 16].try_into().unwrap());
            assert_eq!(bits, 16, "16-bit PCM");
            off += chunk_size;
        } else if chunk_id == b"data" {
            break (off, chunk_size);
        } else {
            off += chunk_size;
        }
    };
    assert!(channels > 0 && sample_rate > 0, "valid WAV geometry");
    let frames = data_size / (channels as usize * 2);
    let mut chans: Vec<Vec<f64>> =
        (0..channels).map(|_| Vec::with_capacity(frames)).collect();
    for frame in 0..frames {
        for channel in 0..channels as usize {
            let i = data_off + frame * (channels as usize * 2) + channel * 2;
            let v = i16::from_le_bytes(bytes[i..i + 2].try_into().unwrap());
            chans[channel].push(v as f64 / 32768.0);
        }
    }
    (chans, channels, sample_rate)
}

fn pack_f32le(rows: &[Vec<f64>]) -> Vec<u8> {
    let mut out = Vec::new();
    for row in rows {
        for value in row {
            let bits = (*value as f32).to_bits().to_le_bytes();
            out.extend_from_slice(&bits);
        }
    }
    out
}

fn get_stage(psy: &PsyFrame, stage: &str, window_samples: &[Vec<f64>]) -> Vec<Vec<f64>> {
    match stage {
        "window" => window_samples.to_vec(),
        "coefficients" => psy.coefficients().clone(),
        "raw_mdct" => psy.raw_mdct().clone(),
        "fft" => psy.fft().clone(),
        "remap" => psy.remap.clone(),
        "seed" => psy.seed.clone(),
        "post" => psy.post.clone(),
        "side" => psy.side.clone(),
        other => panic!("unknown stage {other}"),
    }
}

#[test]
fn stage_parity_all_frames() {
    let index = read_index();
    let frames = index
        .get("frames")
        .and_then(Value::as_array)
        .expect("frames array");
    let audio_packets = index
        .get("audio_packets")
        .and_then(Value::as_i64)
        .expect("audio_packets");
    let representatives: Vec<i64> = index
        .get("representative_frames")
        .and_then(Value::as_array)
        .expect("representative_frames")
        .iter()
        .map(|v| v.as_i64().expect("rep int"))
        .collect();

    // Load PCM + profile.
    let (pcm, channels, sample_rate) =
        read_pcm16(&repo_root().join("tests/fixtures/input.wav"));
    assert_eq!(channels, 6, "6 channels");
    assert_eq!(sample_rate, 44100, "44.1 kHz");

    let data_dir = wem_profiles::DataDir::from_profiles_dir(
        repo_root().join("src/wwise_wem/data/profiles"),
    );
    let bundle = wem_profiles::load_profile_bundle(&data_dir, None, false)
        .expect("installed profile loads");
    let resources = wem_profiles::assemble_analysis_resources(&bundle, None)
        .expect("analysis resources assemble");

    // Create the session and select modes + windows.
    let mut session = AnalysisSession::new(channels, sample_rate, [256, 2048], resources)
        .expect("session constructs");
    let (modes, windows) = session
        .selected_windows(&pcm)
        .expect("modes + windows");

    assert_eq!(
        windows.len() as i64,
        audio_packets,
        "window count == audio_packets"
    );
    assert_eq!(frames.len() as i64, audio_packets, "frame count matches");

    // Validate every frame's scheduling fields.
    for (i, window) in windows.iter().enumerate() {
        let frame = &frames[i];
        assert_eq!(
            (
                int_field(frame, "mode"),
                int_field(frame, "previous"),
                int_field(frame, "current"),
                int_field(frame, "following"),
                int_field(frame, "window_center"),
            ),
            (
                modes[i],
                window.previous(),
                window.current(),
                window.following(),
                window.center,
            ),
            "frame {} scheduling fields",
            i
        );
    }

    // Run the analysis pipeline frame-by-frame, comparing the 8 float stages
    // for the representative frames.
    let mut rep_set = std::collections::HashSet::new();
    for r in &representatives {
        rep_set.insert(*r as usize);
    }
    let mut checked = 0usize;
    for (i, window) in windows.iter().enumerate() {
        let psy = session.analyze_window(window.clone(), None).expect("analyze frame");
        if rep_set.contains(&i) {
            for stage in FLOAT_STAGES {
                let rows = get_stage(&psy, stage, &window.samples);
                let packed = pack_f32le(&rows);
                let golden_name = format!("f{i:03}.{stage}.f32le.bin");
                let golden_path = stage_dir().join("frames").join(&golden_name);
                let golden = std::fs::read(&golden_path)
                    .unwrap_or_else(|_| panic!("golden dump reads: {golden_name}"));
                assert_eq!(
                    packed, golden,
                    "frame {} stage {stage} byte parity",
                    i
                );
            }
            checked += 1;
        }
    }
    assert_eq!(checked, representatives.len(), "all representatives checked");
}
