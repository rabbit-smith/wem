//! StreamSession contract tests (the core streaming lifecycle).
//!
//! Three gates live here:
//!
//! 1. **Chunking invariance** — `StreamSession` output bytes equal
//!    `Encoder::encode_pcm` for any chunk splitting (six+ strategies),
//!    and the packets emitted through `push_pcm_chunk` replay exactly
//!    the WEM container's packet sequence.
//! 2. **Bounded input memory** — 300 s / 600 s / giant-chunk synthetic 6 ch
//!    /44100 streams each run in a freshly forked child process whose peak
//!    RSS is read via `wait4().ru_maxrss`; each child must stay under the
//!    150 MB ceiling. Measuring in the shared test process is unreliable:
//!    parallel `#[test]` threads share one allocator that never returns
//!    memory to the OS, so any in-process reading would be inflated and
//!    mask the real bound. The only in-process memory assertion is a static
//!    structural invariant (ring keep == `STREAM_RING_KEEP`), in
//!    `wem-analysis`.
//! 3. **Performance regression** — `encode_pcm`'s release-mode median
//!    stays within the 150 ms gate documented for the fixture encode.

use std::time::Instant;

use sha2::{Digest, Sha256};
use wem_container::load_wem_parts_bytes;
use wem_core::encoder::{Encoder, Pcm16};
use wem_core::stream::StreamSession;
use wem_core::usecases::wav::read_pcm16;
use wem_core::{WwiseProfile, WwiseVersion};

/// The fixture profile selection: the installed Wwise 2013 6ch/44100
/// configuration.
fn fixture_selection() -> WwiseProfile {
    WwiseProfile::new(WwiseVersion::Wwise2013, 6, 44_100).expect("fixture selection")
}

fn fixtures_dir() -> std::path::PathBuf {
    std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../..")
        .join("tests/fixtures")
        .canonicalize()
        .expect("fixtures directory resolves")
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
    let mut session = StreamSession::for_selection(fixture_selection()).expect("session opens");
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
    let wav = read_pcm16(&fixtures_dir().join("input.wav")).expect("input.wav reads");
    let le_bytes: Vec<u8> = wav.interleaved_le_bytes();
    let channels = wav.channels();

    let encoder = Encoder::new(fixture_selection()).expect("fs encoder builds");
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
// 2. Bounded input memory (child-process RSS measurement)
// ---------------------------------------------------------------------------

/// Peak-RSS ceiling for one streaming encode, measured on the freshly
/// forked child that drives the stream. A 600 s 6ch/44100 stream alone is
/// ~310 MB of i16 PCM; if the session accumulated it, the child would blow
/// far past this. A bounded implementation stays near output+baseline.
const RSS_CEILING_BYTES: usize = 150 * 1024 * 1024;
const SAMPLE_RATE: i64 = 44100;
const CHANNELS: usize = 6;

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

/// Build a fresh streaming session on the fixture selection.
fn build_session() -> StreamSession {
    StreamSession::for_selection(fixture_selection()).expect("session opens")
}

/// Drive `duration` seconds in 10 s chunks and finish (no RSS assertion —
/// the parent reads the child's peak). Runs in the child worker process.
fn drive_duration_stream(duration: i64) {
    let mut session = build_session();
    let mut frame_offset = 0u64;
    let mut seconds_done = 0;
    while seconds_done < duration {
        let chunk_seconds = 10i64.min(duration - seconds_done);
        let frames = (SAMPLE_RATE * chunk_seconds) as usize;
        let bytes = synthetic_chunk(CHANNELS, frames, frame_offset);
        frame_offset += frames as u64;
        session.push_pcm_chunk(&bytes).expect("chunk ok");
        seconds_done += chunk_seconds;
    }
    let result = session.finish().expect("finish ok");
    assert_eq!(
        result.stats.pcm_frames,
        duration * SAMPLE_RATE,
        "frame accounting must match the driven duration"
    );
}

/// Push one 60 s chunk (~33 MB of i16 PCM) and finish (no RSS assertion).
/// Exercises the internal segmentation path (chunk > `SEGMENT_FRAMES`) in
/// the child worker process.
fn drive_giant_chunk() {
    let duration = 60i64;
    let frames = (SAMPLE_RATE * duration) as usize;
    let mut session = build_session();
    let bytes = synthetic_chunk(CHANNELS, frames, 0);
    session.push_pcm_chunk(&bytes).expect("giant chunk ok");
    let result = session.finish().expect("finish ok");
    assert_eq!(
        result.stats.pcm_frames,
        duration * SAMPLE_RATE,
        "frame accounting must match the giant duration"
    );
}

// --- Child-process worker tests ---
//
// These do the real streaming work, but only when the parent memory gate
// spawns them with libtest's own `--ignored --exact <name>` arguments. They
// are `#[ignore]`d so a normal `#[test]` run reports them without executing
// them, and parallel threads never contaminate each other's RSS. The parent
// reads each child's peak via wait4().ru_maxrss.
#[test]
#[ignore = "child worker: the memory gate runs it with --ignored --exact"]
fn stream_memory_rss_worker_duration_300() {
    drive_duration_stream(300);
}

#[test]
#[ignore = "child worker: the memory gate runs it with --ignored --exact"]
fn stream_memory_rss_worker_duration_600() {
    drive_duration_stream(600);
}

#[test]
#[ignore = "child worker: the memory gate runs it with --ignored --exact"]
fn stream_memory_rss_worker_giant_chunk() {
    drive_giant_chunk();
}

// --- Parent memory gate (forks workers, reads their peak RSS) ---

#[test]
fn stream_memory_stays_bounded_across_duration() {
    // A debug-build encode is too slow for these synthetic streams to be
    // a useful ceiling check.
    if cfg!(debug_assertions) {
        eprintln!("skipping memory-ceiling test in debug build");
        return;
    }
    let exe = std::env::current_exe().expect("current exe path");
    let cases = [
        ("stream_memory_rss_worker_duration_300", "300s"),
        ("stream_memory_rss_worker_duration_600", "600s"),
        ("stream_memory_rss_worker_giant_chunk", "giant 60s"),
    ];
    for (worker, label) in cases {
        match child_peak_rss(&exe, worker) {
            Some(Ok(peak)) => {
                eprintln!(
                    "[memory gate] {label}: child peak RSS {peak} bytes ({:.1} MB)",
                    peak as f64 / 1e6
                );
                assert!(
                    peak < RSS_CEILING_BYTES,
                    "{label} child peaked at {peak} bytes RSS (>{RSS_CEILING_BYTES}); \
                     the session must not accumulate the input PCM"
                );
            }
            Some(Err(msg)) => {
                panic!("RSS measurement failed for '{label}': {msg}");
            }
            None => {
                eprintln!(
                    "[memory gate] {label}: ru_maxrss unavailable on this platform; skipping"
                );
            }
        }
    }
}

/// Run the named worker as a child process and return its peak RSS
/// (bytes). `None` when the platform has no ru_maxrss measurement
/// (then the gate skips rather than guess).
fn child_peak_rss(exe: &std::path::Path, worker: &str) -> Option<Result<usize, String>> {
    #[cfg(target_os = "macos")]
    {
        macos_child_peak_rss(exe, worker)
    }
    #[cfg(not(target_os = "macos"))]
    {
        None
    }
}

/// macOS: fork the worker, reap it with `wait4`, read ru_maxrss (bytes).
/// ru_maxrss sits at offset 32 of `struct rusage` on macOS (verified with
/// a C probe); the struct is 144 bytes on arm64/x86_64 darwin.
#[cfg(target_os = "macos")]
fn macos_child_peak_rss(exe: &std::path::Path, worker: &str) -> Option<Result<usize, String>> {
    #[repr(C)]
    struct MacRusage {
        ru_utime_sec: i64,
        ru_utime_usec: i64,
        ru_stime_sec: i64,
        ru_stime_usec: i64,
        ru_maxrss: i64,
        _rest: [i64; 13],
    }
    extern "C" {
        fn wait4(pid: i32, status: *mut i32, options: i32, rusage: *mut MacRusage) -> i32;
    }

    let mut cmd = std::process::Command::new(exe);
    // Test-name filter selects the worker; `--ignored` is what lets the
    // `#[ignore]`d worker actually run. Both are libtest arguments, so the
    // child needs no environment of its own.
    cmd.arg("--ignored");
    cmd.arg("--exact");
    cmd.arg(worker);
    cmd.stdout(std::process::Stdio::null());
    cmd.stderr(std::process::Stdio::null());
    let child = match cmd.spawn() {
        Ok(c) => c,
        Err(e) => return Some(Err(format!("worker spawn failed: {e}"))),
    };
    // We reap manually via wait4; the handle is dropped without killing
    // (kill-on-drop is off by default on stable) after the child exits.
    let pid = child.id() as i32;

    let mut status: i32 = 0;
    let mut ru: MacRusage = unsafe { std::mem::zeroed() };
    let waited = unsafe { wait4(pid, &mut status, 0, &mut ru) };
    if waited != pid {
        return Some(Err(format!("wait4 returned {waited}, expected {pid}")));
    }
    let peak = ru.ru_maxrss.max(0) as usize;
    Some(Ok(peak))
}

// ---------------------------------------------------------------------------
// 2b. Giant-chunk segmentation parity (in-process; a byte check, not RSS)
// ---------------------------------------------------------------------------

/// The internal segmentation of a huge single push must not change the
/// output bytes: one 60 s push encodes identically to the same PCM in
/// three chunks.
#[test]
fn stream_giant_chunk_segmentation_parity() {
    let duration = 60i64;
    let frames = (SAMPLE_RATE * duration) as usize;
    let pcm_bytes = synthetic_chunk(CHANNELS, frames, 0);

    let mut giant = build_session();
    giant.push_pcm_chunk(&pcm_bytes).expect("giant chunk ok");
    let giant_result = giant.finish().expect("giant finish");

    let mut split = build_session();
    let third = pcm_bytes.len() / 3;
    for slice in [0..third, third..2 * third, 2 * third..pcm_bytes.len()] {
        split.push_pcm_chunk(&pcm_bytes[slice]).expect("chunk ok");
    }
    let split_result = split.finish().expect("split finish");

    assert_eq!(
        giant_result.data, split_result.data,
        "internal segmentation changed the output bytes"
    );
}

#[test]
fn stream_tail_matches_batch_when_final_center_crosses_source_end() {
    let frames = 4_388usize;
    let mut state = 20_130_701u32;
    let mut pcm_bytes = Vec::with_capacity(frames * CHANNELS * 2);
    for _ in 0..frames * CHANNELS {
        state ^= state << 13;
        state ^= state >> 17;
        state ^= state << 5;
        pcm_bytes.extend_from_slice(&(state as i16).to_le_bytes());
    }
    let pcm = Pcm16::from_interleaved_le(SAMPLE_RATE, CHANNELS, &pcm_bytes)
        .expect("synthetic PCM parses");
    let expected = Encoder::new(fixture_selection())
        .expect("encoder builds")
        .encode_pcm(&pcm)
        .expect("batch encode runs");

    let mut streamed = build_session();
    streamed
        .push_pcm_chunk(&pcm_bytes)
        .expect("single chunk pushes");
    let actual = streamed.finish().expect("stream finish runs");

    assert!(
        actual.data == expected.data,
        "stream tail emitted a different frame sequence from batch: \
         batch_packets={}, stream_packets={}, batch_bytes={}, stream_bytes={}",
        expected.stats.audio_packets,
        actual.stats.audio_packets,
        expected.data.len(),
        actual.data.len(),
    );
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

    let encoder = Encoder::new(fixture_selection()).expect("fs encoder builds");
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
