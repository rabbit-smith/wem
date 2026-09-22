//! Live per-frame parity: the native kernel against the pure-Python oracle.
//!
//! Every one of the 205 audio frames of `tests/fixtures/input.wav` is produced
//! twice at test time and the two results are compared value by value: the
//! scheduler's mode and transition codes, the window center, the eight float
//! analysis stages, the floor posts, the quantized residue rows, and the packed
//! audio packet. The header the oracle prints is compared too — the resolved
//! profile label and the carrier's setup packet — so the two halves of the
//! container identity that no per-frame row carries are established here as
//! well.
//!
//! Nothing is recorded, and no side is compared against a stored expectation.
//! The kernel runs in this process — `AnalysisSession` plus
//! `wem_core::pack::pack_analysis_frame`, the same calls `Encoder::encode_pcm`
//! makes — and the oracle runs in a subprocess
//! (`tests/parity/oracle_frame_values.py`), one JSON record per frame on
//! stdout. This suite decodes each record as it arrives and, on a divergence,
//! names the frame, stage, channel, and bin and prints both words.
//!
//! Why this direction: the kernel's per-frame values (analysis stages, floor
//! posts, residue rows) are crate surfaces the PyO3 binding does not expose, so
//! a Python-side comparison could never see more than packet bytes. The kernel
//! crates are directly reachable from a Rust integration test, which mirrors
//! `crates/wem-container/tests/container_codec.rs` the other way round: that
//! one drives the oracle in a subprocess and compares in Rust, this one does
//! the same with the oracle as the subprocess.

use std::io::{BufRead, BufReader, Lines};
use std::path::{Path, PathBuf};
use std::process::{Child, ChildStdout, Command, Stdio};

use serde_json::Value;
use wem_analysis::model::PsyFrame;
use wem_analysis::session::AnalysisSession;
use wem_core::pack::pack_analysis_frame;
use wem_core::usecases::wav;
use wem_core::Encoder;

mod common;

use common::{fixture_selection, fixtures_dir, repo_root};

/// The float analysis stages the oracle emits and this suite compares, in
/// pipeline order (the names a divergence is reported under).
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

/// The fixture's audio packet count (the frame count the comparison covers).
const AUDIO_PACKETS: usize = 205;

// ---------------------------------------------------------------------------
// Oracle subprocess
// ---------------------------------------------------------------------------

fn python_interpreter() -> PathBuf {
    // The development venv first so a local run uses the interpreter the
    // Makefile does; the plain name keeps CI (system python) working.
    let venv = repo_root().join(".venv/bin/python");
    if venv.is_file() {
        return venv;
    }
    PathBuf::from(std::env::var("PYTHON").unwrap_or_else(|_| "python3".into()))
}

fn python_path() -> String {
    let root = repo_root();
    #[cfg(unix)]
    let separator = ":";
    #[cfg(not(unix))]
    let separator = ";";
    format!(
        "{}{separator}{}",
        root.join("src").display(),
        root.join("reference").display()
    )
}

/// The oracle's per-frame value stream: one JSON record per line, header first.
struct OracleStream {
    child: Child,
    lines: Lines<BufReader<ChildStdout>>,
}

impl OracleStream {
    fn spawn(wav: &Path) -> Self {
        let mut child = Command::new(python_interpreter())
            .current_dir(repo_root())
            .env("PYTHONPATH", python_path())
            .env("PYTHONDONTWRITEBYTECODE", "1")
            .args(["-m", "tests.parity.oracle_frame_values", "--wav"])
            .arg(wav)
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            // The oracle's stderr carries only a failure report (a traceback or
            // its own count assertion); it goes straight to the test log.
            .stderr(Stdio::inherit())
            .spawn()
            .expect("python interpreter runs the oracle value stream");
        let stdout = child.stdout.take().expect("oracle stdout is piped");
        Self {
            child,
            lines: BufReader::new(stdout).lines(),
        }
    }

    fn next_record(&mut self, what: &str) -> Value {
        let line = self
            .lines
            .next()
            .unwrap_or_else(|| panic!("oracle stream ended before {what}"))
            .expect("oracle record reads");
        serde_json::from_str(&line).unwrap_or_else(|error| panic!("oracle {what} parses: {error}"))
    }

    /// Assert the stream is exhausted and the oracle exited cleanly.
    fn finish(mut self) {
        if let Some(extra) = self.lines.next() {
            let extra = extra.expect("trailing oracle record reads");
            let record: Value = serde_json::from_str(&extra).expect("trailing record parses");
            panic!(
                "oracle emitted more frames than the kernel: extra record index {}",
                record.get("index").unwrap_or(&Value::Null)
            );
        }
        let status = self.child.wait().expect("oracle exits");
        assert!(status.success(), "oracle exited with {status}");
    }
}

// ---------------------------------------------------------------------------
// Word-level comparison
// ---------------------------------------------------------------------------

fn hex_digit(byte: u8, context: &str) -> u32 {
    match byte {
        b'0'..=b'9' => u32::from(byte - b'0'),
        b'a'..=b'f' => u32::from(byte - b'a') + 10,
        other => panic!("{context}: '{other}' is not a lowercase hex digit"),
    }
}

/// Encode bytes as lowercase hexadecimal (the oracle stream's byte convention).
fn hex_bytes(payload: &[u8]) -> String {
    const DIGITS: &[u8; 16] = b"0123456789abcdef";
    let mut out = String::with_capacity(payload.len() * 2);
    for &byte in payload {
        out.push(DIGITS[(byte >> 4) as usize] as char);
        out.push(DIGITS[(byte & 0xF) as usize] as char);
    }
    out
}

/// Decode one hexadecimal row (8 digits per 32-bit word) into words.
fn hex_words(row: &str, context: &str) -> Vec<u32> {
    assert!(
        row.len().is_multiple_of(8),
        "{context}: row has {} digits, not a whole number of 32-bit words",
        row.len()
    );
    row.as_bytes()
        .chunks_exact(8)
        .map(|chunk| {
            chunk
                .iter()
                .fold(0u32, |word, byte| (word << 4) | hex_digit(*byte, context))
        })
        .collect()
}

fn oracle_row<'a>(record: &'a Value, field: &str, frame: usize) -> &'a Value {
    record
        .get(field)
        .unwrap_or_else(|| panic!("frame {frame}: oracle record has no '{field}'"))
}

fn oracle_int(record: &Value, field: &str, frame: usize) -> i64 {
    oracle_row(record, field, frame)
        .as_i64()
        .unwrap_or_else(|| panic!("frame {frame}: oracle '{field}' is not an integer"))
}

/// Compare one float stage row by row, word by word.
fn compare_float_stage(frame: usize, stage: &str, kernel: &[Vec<f64>], oracle: &Value) {
    let rows = oracle
        .as_array()
        .unwrap_or_else(|| panic!("frame {frame} stage {stage}: oracle rows are not an array"));
    assert_eq!(
        kernel.len(),
        rows.len(),
        "frame {frame} stage {stage}: channel count"
    );
    for (channel, (row, expected)) in kernel.iter().zip(rows).enumerate() {
        let context = format!("frame {frame} stage {stage} channel {channel}");
        let hex = expected
            .as_str()
            .unwrap_or_else(|| panic!("{context}: oracle row is not a string"));
        let words = hex_words(hex, &context);
        assert_eq!(row.len(), words.len(), "{context}: bin count");
        for (bin, value) in row.iter().enumerate() {
            let kernel_word = (*value as f32).to_bits();
            let oracle_word = words[bin];
            if kernel_word != oracle_word {
                panic!(
                    "{context} bin {bin}: kernel 0x{kernel_word:08x} ({}) \
                     != oracle 0x{oracle_word:08x} ({})",
                    f32::from_bits(kernel_word),
                    f32::from_bits(oracle_word),
                );
            }
        }
    }
}

/// Compare one integer row list (floor posts, quantized residue) word by word.
fn compare_int_rows(frame: usize, what: &str, kernel: &[Vec<i64>], oracle: &[Value]) {
    assert_eq!(
        kernel.len(),
        oracle.len(),
        "frame {frame} {what}: row count"
    );
    for (channel, (row, expected)) in kernel.iter().zip(oracle).enumerate() {
        let context = format!("frame {frame} {what} channel {channel}");
        let hex = expected
            .as_str()
            .unwrap_or_else(|| panic!("{context}: oracle row is not a string"));
        let words = hex_words(hex, &context);
        assert_eq!(row.len(), words.len(), "{context}: row length");
        for (index, value) in row.iter().enumerate() {
            let kernel_word = i32::try_from(*value)
                .unwrap_or_else(|_| panic!("{context} index {index}: {value} is not int32"))
                as u32;
            if kernel_word != words[index] {
                panic!(
                    "{context} index {index}: kernel {value} (0x{kernel_word:08x}) \
                     != oracle {} (0x{:08x})",
                    words[index] as i32, words[index],
                );
            }
        }
    }
}

fn compare_posts(frame: usize, kernel: &[Option<Vec<i64>>], oracle: &[Value]) {
    assert_eq!(
        kernel.len(),
        oracle.len(),
        "frame {frame} floor posts: channel count"
    );
    for (channel, (row, expected)) in kernel.iter().zip(oracle).enumerate() {
        match row {
            None => assert!(
                expected.is_null(),
                "frame {frame} floor posts channel {channel}: kernel has no posts, oracle has some"
            ),
            Some(row) => {
                let context = format!("frame {frame} floor posts channel {channel}");
                let hex = expected.as_str().unwrap_or_else(|| {
                    panic!("{context}: kernel has posts, oracle row is {expected}")
                });
                let words = hex_words(hex, &context);
                assert_eq!(row.len(), words.len(), "{context}: row length");
                for (index, value) in row.iter().enumerate() {
                    let kernel_word = i32::try_from(*value)
                        .unwrap_or_else(|_| panic!("{context} index {index}: {value} is not int32"))
                        as u32;
                    if kernel_word != words[index] {
                        panic!(
                            "{context} index {index}: kernel {value} (0x{kernel_word:08x}) \
                             != oracle {} (0x{:08x})",
                            words[index] as i32, words[index],
                        );
                    }
                }
            }
        }
    }
}

fn compare_packet(frame: usize, kernel: &[u8], expected_hex: &str) {
    assert!(
        expected_hex.len().is_multiple_of(2),
        "frame {frame}: oracle packet hex is not a whole number of bytes"
    );
    let expected: Vec<u8> = expected_hex
        .as_bytes()
        .chunks_exact(2)
        .map(|pair| ((hex_digit(pair[0], "packet") << 4) | hex_digit(pair[1], "packet")) as u8)
        .collect();
    assert_eq!(
        kernel.len(),
        expected.len(),
        "frame {frame}: packet size: kernel {} bytes, oracle {} bytes",
        kernel.len(),
        expected.len()
    );
    if let Some(at) = kernel
        .iter()
        .zip(&expected)
        .position(|(kernel_byte, oracle_byte)| kernel_byte != oracle_byte)
    {
        panic!(
            "frame {frame}: packet differs at byte {at} of {}: \
             kernel 0x{:02x} != oracle 0x{:02x}",
            kernel.len(),
            kernel[at],
            expected[at]
        );
    }
}

fn float_stage<'a>(psy: &'a PsyFrame, stage: &str, window: &'a [Vec<f64>]) -> &'a [Vec<f64>] {
    match stage {
        "window" => window,
        "coefficients" => psy.coefficients(),
        "raw_mdct" => psy.raw_mdct(),
        "fft" => psy.fft(),
        "remap" => &psy.remap,
        "seed" => &psy.seed,
        "post" => &psy.post,
        "side" => &psy.side,
        other => panic!("unknown float stage {other}"),
    }
}

// ---------------------------------------------------------------------------
// The comparison
// ---------------------------------------------------------------------------

#[test]
fn every_frame_matches_the_python_oracle() {
    let wav_path = fixtures_dir().join("input.wav");
    let wav = wav::read_pcm16(&wav_path).expect("fixture parses as signed-16 PCM");
    let pcm = wav.to_pcm16().expect("fixture geometry");
    let encoder = Encoder::new(fixture_selection()).expect("fixture selection resolves");

    // The kernel's per-frame pipeline: the calls `Encoder::encode_pcm` makes,
    // stopped one frame at a time so each frame's values are visible.
    let mut session = AnalysisSession::new(
        pcm.channel_count() as i64,
        pcm.sample_rate(),
        encoder.profile().block_sizes(),
        encoder.analysis_resources().clone(),
    )
    .expect("analysis session constructs");
    let pcm_rows = pcm.to_float_rows();
    let conditioned = session.condition_pcm(&pcm_rows).expect("PCM conditions");
    let (modes, windows) = session
        .selected_windows(&conditioned)
        .expect("mode/window plan selects");
    assert_eq!(modes.len(), windows.len(), "one mode per window");
    assert_eq!(
        windows.len(),
        AUDIO_PACKETS,
        "the fixture's frame count (asserted on both sides)"
    );

    let mut oracle = OracleStream::spawn(&wav_path);
    let header = oracle.next_record("the header");
    assert_eq!(
        header["audio_packets"].as_u64(),
        Some(AUDIO_PACKETS as u64),
        "oracle frame count"
    );
    assert_eq!(
        header["channels"].as_u64(),
        Some(pcm.channel_count() as u64),
        "oracle channel count"
    );
    assert_eq!(
        header["sample_rate"].as_i64(),
        Some(pcm.sample_rate()),
        "oracle sample rate"
    );
    assert_eq!(
        header["pcm_frames"].as_i64(),
        Some(pcm.frame_count()),
        "oracle PCM frame count"
    );
    // The oracle names the source it read; the kernel read the same path, and
    // every frame below is compared word for word, so a different input on
    // either side cannot pass unnoticed.
    let expected_source = wav_path
        .strip_prefix(repo_root())
        .expect("the fixture lives under the repository root");
    assert_eq!(
        header["wav"].as_str(),
        Some(expected_source.to_string_lossy().as_ref()),
        "both sides read the same source path"
    );
    assert_eq!(
        header["stages"].as_array().map(Vec::len),
        Some(FLOAT_STAGES.len()),
        "oracle emits every compared stage"
    );
    // The resolved profile and its setup packet: the kernel's carrier surface
    // against the oracle's assembled resources, live. These are the two halves
    // of the container header that no per-frame row carries.
    let resolved = wem_profiles::resolve_wem_profile_selection(fixture_selection())
        .expect("the fixture selection resolves in the kernel");
    assert_eq!(
        header["profile"].as_str(),
        Some(resolved.label().as_str()),
        "both sides resolve the same installed profile"
    );
    let compiled = wem_profiles::compiled_profile_for_selection(fixture_selection())
        .expect("the installed 6ch profile resolves");
    let resources = wem_profiles::assemble_encoder_profile_resources(&compiled, None, None)
        .expect("analysis resources assemble");
    assert_eq!(
        header["setup_packet"].as_str(),
        Some(hex_bytes(&resources.setup_packet).as_str()),
        "the kernel carrier's setup packet is the oracle's"
    );

    for (index, window) in windows.iter().enumerate() {
        let analysis = session
            .analyze_window(window.clone(), None)
            .expect("frame analyzes");
        let encoded = pack_analysis_frame(
            encoder.setup(),
            encoder.codebooks(),
            &analysis,
            pcm.channel_count() as u32,
        )
        .expect("frame packs");

        let record = oracle.next_record(&format!("frame {index}"));
        assert_eq!(
            oracle_int(&record, "index", index),
            index as i64,
            "oracle frame order"
        );
        assert_eq!(
            oracle_int(&record, "mode", index),
            modes[index],
            "frame {index}: mode"
        );
        let transition = oracle_row(&record, "transition", index)
            .as_array()
            .unwrap_or_else(|| panic!("frame {index}: transition is not an array"))
            .iter()
            .map(|value| value.as_i64().expect("transition code is an integer"))
            .collect::<Vec<i64>>();
        assert_eq!(
            transition,
            vec![window.previous(), window.current(), window.following()],
            "frame {index}: transition (previous, current, following)"
        );
        assert_eq!(
            oracle_int(&record, "window_center", index),
            window.center,
            "frame {index}: window center"
        );

        let stages = oracle_row(&record, "stages", index)
            .as_object()
            .unwrap_or_else(|| panic!("frame {index}: oracle emitted no stage rows"));
        assert_eq!(
            stages.len(),
            FLOAT_STAGES.len(),
            "frame {index}: stage count"
        );
        for stage in FLOAT_STAGES {
            let rows = stages
                .get(stage)
                .unwrap_or_else(|| panic!("frame {index}: oracle emitted no '{stage}' rows"));
            compare_float_stage(
                index,
                stage,
                float_stage(&analysis, stage, &window.samples),
                rows,
            );
        }

        let posts = oracle_row(&record, "posts", index)
            .as_array()
            .unwrap_or_else(|| panic!("frame {index}: posts are not an array"));
        compare_posts(index, &encoded.posts, posts);

        let residue = oracle_row(&record, "residue_q", index)
            .as_array()
            .unwrap_or_else(|| panic!("frame {index}: residue rows are not an array"));
        compare_int_rows(
            index,
            "quantized residue",
            &encoded.quantized_residue,
            residue,
        );

        let packet = oracle_row(&record, "packet", index)
            .as_str()
            .unwrap_or_else(|| panic!("frame {index}: packet is not a hex string"));
        compare_packet(index, &encoded.packet, packet);
    }

    oracle.finish();
}
