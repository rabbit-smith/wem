//! StreamSession contract tests (wwise.v1 streaming lifecycle).
//!
//! Three gates live here:
//!
//! 1. **Chunking invariance** — `StreamSession` output bytes equal
//!    `Encoder::encode_pcm` for any chunk splitting (six+ strategies),
//!    and the packets emitted through `push_pcm_chunk` replay exactly
//!    the WEM container's packet sequence.
//! 2. **Bounded input memory** — 300 s / 600 s synthetic 6 ch/44100
//!    streams stay under the 150 MB peak-RSS ceiling, and the memory
//!    growth beyond the emitted output is duration-independent
//!    (the input PCM is never accumulated).
//! 3. **Performance regression** — `encode_pcm`'s release-mode median
//!    stays within the 150 ms gate documented for the fixture encode.

use std::time::Instant;

use sha2::{Digest, Sha256};
use wem_container::load_wem_parts_bytes;
use wem_core::encoder::Encoder;
use wem_core::stream::{ProfileRef, StreamSession};
use wem_core::usecases::wav::read_pcm16;

const PROFILE_NAME: &str = "wwise2013-6ch-44100";
const SETUP_SHA256: &str = "3ef56cbd6e6a66a5474005db05912624487faa555fb2cdfed130f606b322e4e3";

/// Serializes the memory-heavy tests: they share the process's RSS, so a
/// concurrent encode would inflate each other's readings. The chunking
/// test, the memory gate and the performance gate all hold this lock.
static MEMORY_GUARD: std::sync::Mutex<()> = std::sync::Mutex::new(());

fn fixtures_dir() -> std::path::PathBuf {
    std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../..")
        .join("tests/fixtures")
        .canonicalize()
        .expect("fixtures directory resolves")
}

fn read_profile_bytes_bundle() -> (Vec<u8>, Vec<(String, Vec<u8>)>) {
    let dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../..")
        .join("src/wwise_wem/data/profiles")
        .canonicalize()
        .expect("profiles directory resolves");
    let index = std::fs::read(dir.join("index.json")).expect("index.json reads");
    let mut files: Vec<(String, Vec<u8>)> = Vec::new();
    fn walk(base: &std::path::Path, cur: &std::path::Path, out: &mut Vec<(String, Vec<u8>)>) {
        for entry in std::fs::read_dir(cur).expect("directory reads") {
            let entry = entry.expect("directory entry");
            let path = entry.path();
            if path.is_dir() {
                walk(base, &path, out);
            } else if path.file_name() != Some("index.json".as_ref()) {
                let rel = path.strip_prefix(base).expect("path under profiles dir");
                out.push((
                    rel.to_string_lossy().into_owned(),
                    std::fs::read(&path).expect("resource file reads"),
                ));
            }
        }
    }
    walk(&dir, &dir, &mut files);
    (index, files)
}

// ---------------------------------------------------------------------------
// 1. Chunking invariance
// ---------------------------------------------------------------------------

/// Drive one chunking strategy end-to-end and return the emitted packet
/// sequence plus the final encode result.
fn run_session_with_chunks(
    le_bytes: &[u8],
    cuts: &[usize],
) -> (Vec<Vec<u8>>, wem_core::EncodeResult) {
    let (index, files) = read_profile_bytes_bundle();
    let ref_ = ProfileRef::with_name(SETUP_SHA256, PROFILE_NAME);
    let mut session =
        StreamSession::for_profile_ref_bytes(&ref_, &index, files).expect("bytes init");
    let mut emitted: Vec<Vec<u8>> = Vec::new();
    let mut offset = 0usize;
    for &cut in cuts {
        let end = offset + cut;
        let packets = session
            .push_pcm_chunk(&le_bytes[offset..end])
            .expect("chunk ok");
        for packet in packets {
            emitted.push(packet.data);
        }
        offset = end;
    }
    let result = session.finish().expect("finish ok");
    (emitted, result)
}

/// Six+ chunk splits over the fixture stream (the cuts must tile the
/// whole byte string; `tiles` describes each strategy).
fn chunk_strategies(len: usize, channels: i64) -> Vec<(String, Vec<usize>)> {
    let bytes_per_frame = channels as usize * 2;
    let frames = len / bytes_per_frame;
    let full_frames_bytes = frames * bytes_per_frame;

    // 1. One single chunk.
    let one = vec![full_frames_bytes];
    // 2. Two uneven chunks (1/3 | 2/3, frame-aligned).
    let third = (frames / 3) * bytes_per_frame;
    let two = vec![third, full_frames_bytes - third];
    // 3. Five near-equal chunks (the last absorbs the rounding remainder).
    let base = frames / 5;
    let five: Vec<usize> = (0..5)
        .map(|i| {
            let count = if i < 4 { base } else { frames - base * 4 };
            count * bytes_per_frame
        })
        .collect();
    // 4. One frame per chunk (the adversarial fragmentation extreme).
    let tiny: Vec<usize> = vec![bytes_per_frame; frames];
    // 5. Irregular chunks (7, 13 frames, then an empty no-op chunk, 999
    //    frames, then the rest).
    let irregular: Vec<usize> = vec![
        7 * bytes_per_frame,
        13 * bytes_per_frame,
        0,
        999 * bytes_per_frame,
        full_frames_bytes - (7 + 13 + 999) * bytes_per_frame,
    ];
    // 6. 4096-frame chunks, remainder at the end.
    let mut block: Vec<usize> = Vec::new();
    let mut remaining = full_frames_bytes;
    while remaining > 0 {
        let take = remaining.min(4096 * bytes_per_frame);
        block.push(take);
        remaining -= take;
    }

    let strategies = vec![
        ("single chunk".to_string(), one),
        ("two uneven chunks".to_string(), two),
        ("five near-equal chunks".to_string(), five),
        ("one frame per chunk".to_string(), tiny),
        ("irregular chunks + empty".to_string(), irregular),
        ("4096-frame blocks".to_string(), block),
    ];
    // Sanity: every strategy tiles the byte string exactly.
    for (name, cuts) in &strategies {
        let sum: usize = cuts.iter().sum();
        assert_eq!(
            sum, full_frames_bytes,
            "strategy '{name}' must tile the stream"
        );
    }
    strategies
}

#[test]
fn stream_bytes_match_encode_pcm_across_chunking_strategies() {
    let _guard = MEMORY_GUARD.lock().expect("memory guard is uncontended");

    let wav = read_pcm16(&fixtures_dir().join("input.wav")).expect("input.wav reads");
    let le_bytes: Vec<u8> = wav.interleaved_le_bytes();
    let channels = wav.channels();

    let encoder = Encoder::from_profile(PROFILE_NAME).expect("fs encoder builds");
    let pcm = wav.to_pcm16().expect("wav converts to Pcm16");
    let reference = encoder.encode_pcm(&pcm).expect("encode runs");

    for (name, cuts) in chunk_strategies(le_bytes.len(), channels as i64) {
        let (emitted, result) = run_session_with_chunks(&le_bytes, &cuts);
        assert_eq!(
            result.data,
            reference.data,
            "strategy '{name}' ({} chunks) diverges from encode_pcm",
            cuts.len()
        );
        assert_eq!(
            result.stats, reference.stats,
            "strategy '{name}' stats diverge from encode_pcm"
        );

        // The emitted packet sequence must replay the WEM packet layout as a
        // strict prefix: setup first, then audio packets in encoding order;
        // the delayed tail (frame centers within the look-ahead window of the
        // endpoint) is withheld until `finish` and appears only in the
        // container.
        let parts = load_wem_parts_bytes(&result.data).expect("wem parses");
        let mut expected: Vec<Vec<u8>> = Vec::new();
        if let Some(setup) = &parts.setup_packet {
            expected.push(setup.clone());
        }
        expected.extend(parts.audio_packets.iter().cloned());
        assert!(
            !emitted.is_empty() && expected.len() >= emitted.len(),
            "strategy '{name}': emitted {} packets must be a prefix of the WEM's {}",
            emitted.len(),
            expected.len()
        );
        assert_eq!(
            emitted,
            expected[..emitted.len()].to_vec(),
            "strategy '{name}': emitted packets must equal the WEM packet sequence prefix"
        );
        // The delayed tail (the only packets withheld for `finish`) is
        // bounded: at most 24 short frames plus the setup packet gap.
        assert!(
            expected.len() - emitted.len() <= 25,
            "strategy '{name}': {delayed} packets deferred to finish exceeds the documented tail bound",
            delayed = expected.len() - emitted.len()
        );
        // Streaming must not defer everything: the first push completes
        // frames (the fixture stream far exceeds the 4096-sample start
        // threshold on its first chunk in every strategy).
        assert!(
            emitted.len() > 1,
            "strategy '{name}': no audio packet emitted during streaming"
        );
    }
}

// ---------------------------------------------------------------------------
// 2. Bounded input memory (300 s / 600 s synthetic streams)
// ---------------------------------------------------------------------------

/// Deterministic, slowly-varying synthetic PCM (two integer triangle
/// waves per channel; structured enough to compress like real audio, so
/// the test exercises the input-memory bound instead of the compressor's
/// worst case). The encoder kernel never sees a transcendental — this is
/// test-only source data.
fn synthetic_chunk(channels: usize, frames: usize, start_frame: u64) -> Vec<u8> {
    let mut out = Vec::with_capacity(frames * channels * 2);
    for frame in 0..frames {
        let i = start_frame + frame as u64;
        for channel in 0..channels {
            let c = channel as u64;
            let slow = tri(i * 3, 49152, c * 7919u64);
            let fast = tri(i * 5, 16384, c * 104729u64);
            let value = (slow * 9000.0 + fast * 2200.0) as i16;
            out.extend_from_slice(&value.to_le_bytes());
        }
    }
    out
}

/// One integer triangle wave in [-1, 1].
fn tri(i: u64, period: u64, seed: u64) -> f64 {
    let r = (i + seed) % period;
    let p = period as f64;
    if r < period / 2 {
        (r as f64 * 4.0) / p - 1.0
    } else {
        3.0 - (r as f64 * 4.0) / p
    }
}

/// Peak RSS probe via `ps -o rss= -p <pid>` (macOS and Linux; the value
/// is in kilobytes on both).
fn process_rss_bytes() -> Option<usize> {
    use std::process::Command;
    let pid = std::process::id().to_string();
    let output = Command::new("ps")
        .args(["-o", "rss=", "-p", &pid])
        .output()
        .ok()?;
    if !output.status.success() {
        return None;
    }
    let kb = output
        .stdout
        .split(|b| *b == b' ' || *b == b'\n' || *b == b'\t')
        .find_map(|field| {
            std::str::from_utf8(field)
                .ok()
                .and_then(|text| text.parse::<usize>().ok())
        })?;
    Some(kb * 1024)
}

const RSS_CEILING_BYTES: usize = 150 * 1024 * 1024;
const DURATION_SECONDS: [i64; 2] = [300, 600];
const SAMPLE_RATE: i64 = 44100;
const CHANNELS: usize = 6;

/// One duration phase of the memory gate: drive `duration` seconds in
/// 10 s chunks, track peak RSS and the emitted output bytes, and finish.
fn run_duration_phase(duration: i64) {
    let (index, files) = read_profile_bytes_bundle();
    let ref_ = ProfileRef::with_name(SETUP_SHA256, PROFILE_NAME);
    let mut session =
        StreamSession::for_profile_ref_bytes(&ref_, &index, files).expect("bytes init");

    let mut rng_frame_offset = 0u64;
    let baseline_rss = process_rss_bytes().expect("rss probe works");
    let mut peak_rss = baseline_rss;
    let mut emitted_bytes = 0usize;

    let mut seconds_done = 0;
    while seconds_done < duration {
        let chunk_seconds = 10i64.min(duration - seconds_done);
        let frames = (SAMPLE_RATE * chunk_seconds) as usize;
        let bytes = synthetic_chunk(CHANNELS, frames, rng_frame_offset);
        rng_frame_offset += frames as u64;
        let packets = session.push_pcm_chunk(&bytes).expect("chunk ok");
        for packet in packets {
            emitted_bytes += packet.data.len();
        }
        if let Some(rss) = process_rss_bytes() {
            peak_rss = peak_rss.max(rss);
        }
        seconds_done += chunk_seconds;
    }

    // The ceiling: the whole run (push phase, where input retention
    // lives) must stay under 150 MB of RSS.
    assert!(
        peak_rss < RSS_CEILING_BYTES,
        "{duration}s stream peaked at {peak_rss} bytes RSS (> {RSS_CEILING_BYTES}); \
         the session must not accumulate the input PCM"
    );

    // Duration-independence sanity: memory beyond the emitted output
    // (i.e. retained input state) is the bounded ring + tails + mode
    // metadata — assert it stays small. The output itself is the only
    // legitimately linear component, measured via `emitted_bytes`.
    let excess = peak_rss
        .saturating_sub(emitted_bytes)
        .saturating_sub(baseline_rss);
    assert!(
        excess < 64 * 1024 * 1024,
        "{duration}s stream retained {excess} bytes beyond baseline+output; \
         input-side state must be duration-independent"
    );

    // Complete the lifecycle (container assembly happens on Finish).
    let result = session.finish().expect("finish ok");
    assert_eq!(
        result.stats.pcm_frames,
        duration * SAMPLE_RATE,
        "frame accounting must match the driven duration"
    );
}

/// The giant-chunk phase: one 60 s push (~33 MB of i16 PCM) must stay
/// bounded through the internal segmentation, and segmentation must not
/// change the bytes.
fn run_giant_chunk_phase() {
    let duration = 60i64;
    let frames = (SAMPLE_RATE * duration) as usize;

    let (index, files) = read_profile_bytes_bundle();
    let ref_ = ProfileRef::with_name(SETUP_SHA256, PROFILE_NAME);
    let mut session =
        StreamSession::for_profile_ref_bytes(&ref_, &index, files.clone()).expect("bytes init");

    // Baseline before the caller's PCM buffer exists: the ceiling is about
    // the session's retention, not the caller's input payload.
    let baseline_rss = process_rss_bytes().expect("rss probe works");
    let pcm_bytes = synthetic_chunk(CHANNELS, frames, 0);
    let _packets = session.push_pcm_chunk(&pcm_bytes).expect("giant chunk ok");
    let peak_rss = process_rss_bytes().expect("rss probe works");
    assert!(
        peak_rss < RSS_CEILING_BYTES,
        "single {duration}s chunk peaked at {peak_rss} bytes RSS (baseline {baseline_rss}); \
         internal segmentation must keep the conversion bounded"
    );

    // Segmentation must not change the bytes: the same PCM in three
    // chunks encodes identically.
    let mut chunked =
        StreamSession::for_profile_ref_bytes(&ref_, &index, files).expect("bytes init");
    let third = pcm_bytes.len() / 3;
    for slice in [0..third, third..2 * third, 2 * third..pcm_bytes.len()] {
        chunked.push_pcm_chunk(&pcm_bytes[slice]).expect("chunk ok");
    }
    let giant = session.finish().expect("giant finish");
    let split = chunked.finish().expect("split finish");
    assert_eq!(
        giant.data, split.data,
        "segmentation changed the output bytes"
    );
}

/// Bounded-input-memory gate. Both phases run sequentially in one test
/// function: process RSS is shared, so parallel test threads would
/// inflate each other's readings.
#[test]
fn stream_memory_stays_bounded_across_duration() {
    // The ceiling is about input retention; a debug-build encode is too
    // slow for the synthetic streams to be a useful check.
    if cfg!(debug_assertions) {
        eprintln!("skipping memory-ceiling test in debug build");
        return;
    }
    let _guard = MEMORY_GUARD.lock().expect("memory guard is uncontended");

    // The giant-chunk phase runs first, on the process's freshest
    // allocator state, so its baseline is as clean as it gets.
    run_giant_chunk_phase();
    for &duration in &DURATION_SECONDS {
        run_duration_phase(duration);
    }
}

// ---------------------------------------------------------------------------
// 3. Performance regression gate (encode_pcm median, release only)
// ---------------------------------------------------------------------------

const ENCODE_MEDIAN_GATE_MS: f64 = 150.0;
const ENCODE_RUNS: usize = 5;

#[test]
fn encode_pcm_release_median_within_gate() {
    if cfg!(debug_assertions) {
        eprintln!("skipping performance gate in debug build");
        return;
    }
    let _guard = MEMORY_GUARD.lock().expect("memory guard is uncontended");

    let encoder = Encoder::from_profile(PROFILE_NAME).expect("fs encoder builds");
    let wav = read_pcm16(&fixtures_dir().join("input.wav")).expect("input.wav reads");
    let pcm = wav.to_pcm16().expect("wav converts to Pcm16");

    // Warm-up: page faults, allocator growth, I-cache.
    encoder.encode_pcm(&pcm).expect("warm-up encode");

    let mut durations_ms: Vec<f64> = Vec::with_capacity(ENCODE_RUNS);
    for _ in 0..ENCODE_RUNS {
        let start = Instant::now();
        let result = encoder.encode_pcm(&pcm).expect("encode runs");
        durations_ms.push(start.elapsed().as_secs_f64() * 1e3);
        // Guard against silent bit drift: the golden hash is the contract.
        let sha = wem_profiles::resources::hex(Sha256::digest(&result.data));
        assert_eq!(
            sha, "17851d26c6210b85e498ae0452d2562d7b9e2c3e9e795c459656b9c9d8d35247",
            "encode_pcm output hash drifted from the golden WEM"
        );
    }
    durations_ms.sort_by(|a, b| a.partial_cmp(b).expect("durations order"));
    let median_ms = durations_ms[durations_ms.len() / 2];
    assert!(
        median_ms <= ENCODE_MEDIAN_GATE_MS,
        "encode_pcm median {median_ms:.1} ms exceeds the {ENCODE_MEDIAN_GATE_MS:.0} ms gate \
         (runs: {:?} ms)",
        durations_ms
            .iter()
            .map(|ms| format!("{ms:.1}"))
            .collect::<Vec<_>>()
    );
}
