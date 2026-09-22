//! The decode session's memory bound, measured on the child that does the
//! decoding.
//!
//! The encode side has had this check since the streaming session landed
//! (`crates/wem-core/tests/streaming.rs`, `RSS_CEILING_BYTES`); the decode side
//! had the same claim in prose (`src/decoder.rs`, "Incrementality and memory")
//! and no test, and the claim was false for as long as it was unwritten:
//! `SynthesisOla` used to accumulate the whole timeline it synthesized into, so
//! a live session held 8 bytes per decoded sample per channel — twice the f32
//! PCM it hands the caller — for as long as it lived, and a 64x longer stream
//! measured 10.6x the peak RSS (22 -> 233 MB;
//! `docs/findings/decode-performance.md`). This file is that bound's check: it
//! fails on the tree that preceded the fix, with the child peaking at 230.7 MB
//! against the 67 MB ceiling below.
//!
//! **The material.** The measured child decodes the fixture's own PCM16 frames
//! repeated [`STREAM_MULTIPLE`] times, encoded by the streaming encoder — the
//! same construction, and the same top point, as the peak-RSS curve in the
//! finding above. It is generated in the parent (into the crate's gitignored
//! `target/` tree) rather than in the child: the encoding session's own
//! footprint is bounded but not small, and it would sit under the child's peak
//! and blur the ceiling it is being held to.
//!
//! **Why a child process.** The ceiling is a property of the product, so it is
//! asserted; but the *measurement* has to be the decode's own. In-process it
//! would not be: parallel `#[test]` threads share one allocator that does not
//! return memory to the OS, so any reading taken in the shared process is
//! inflated by whatever else ran. The parent forks the test binary for the
//! `#[ignore]`d worker, reaps it with `wait4`, and reads that child's own
//! `ru_maxrss`. The child asserts what it decoded (the container's declared
//! frame count) and exits non-zero if anything failed; the parent fails the
//! test when the measurement itself fails, because then the bound is simply
//! unobserved.
//!
//! Declared ambient reads: `WEM_DECODE_WEM` names the WEM the worker child
//! decodes — the same variable, and the same meaning, as the decode
//! measurement harness's child (`crates/wem-core/tests/decode_stage_timings.rs`).
//! The parent sets it; a caller running the worker by hand must too.

use std::io::Read;
use std::path::{Path, PathBuf};

use wem_container::fmt::{VorbisFmtFields, WWISE_VORBIS_FORMAT_TAG};
use wem_container::riff::parse_chunks;
use wem_core::decoder::DecodeSession;
use wem_core::stream::StreamSession;
use wem_core::usecases::wav::read_pcm16;

mod common;

use common::{fixture_selection, fixtures_dir};

/// Fixture-length multiples the long stream tiles. 64x is the top point of the
/// published RSS curve (8 921 472 frames, 202.3 s, 6.99 MB of WEM), which is
/// what makes this test's subject the stream the finding measured.
const STREAM_MULTIPLE: usize = 64;

/// Bytes per push the measured child feeds the session — bounded chunks, which
/// is how a caller streams a WEM it did not write.
const CHUNK_BYTES: usize = 64 * 1024;

/// Peak-RSS ceiling for the measured child, in bytes.
///
/// The stream is 8 921 472 frames over 6 channels: 214 MB of f32 PCM, all of
/// which the child is handed and discards. A session that retains its own
/// timeline on top of that measured **230.7 MB** of peak RSS on the tree
/// before the fix — 10.6x the same decode at 1x the length. What a bounded
/// session costs, measured on the fixed tree, is **16.0 MB**: the process
/// baseline (~10 MB), the compiled carrier's tables, the 6.99 MB WEM arriving
/// in 64 KiB pieces, one reply buffer's worth of PCM and one overlap-add
/// window per channel. The ceiling sits between the two readings with room on
/// both sides — 4.2x the bounded figure, a third of the unbounded one — and
/// both figures are the ones this very test reports, so the margin can be
/// re-derived from a run log rather than believed. It is stated in bytes and
/// not as a ratio because a ratio would need a second, shorter decode inside
/// the same measured child.
const RSS_CEILING_BYTES: usize = 64 * 1024 * 1024;

/// Duration of one push during generation, in frames (10 s at 44100): the
/// encoding session's own bound is stated per chunk in `streaming.rs`, and
/// nothing here needs one push of the whole stream.
const GENERATE_CHUNK_FRAMES: usize = 441_000;

/// The measured worker's test-function name (spawned with `--ignored --exact`).
const WORKER: &str = "decode_memory_rss_worker";

/// The WEM the worker decodes: the fixture's PCM16 frames repeated
/// [`STREAM_MULTIPLE`] times, encoded once and cached in the crate's
/// gitignored `target/` tree.
///
/// The cache is validated against the container's own declared frame count
/// rather than trusted: a stale file from another construction would silently
/// move the ceiling's subject, and the count is the one number the test's
/// arithmetic depends on.
///
/// Byte for byte this is the stream the measurement tool generates for the same
/// point (`scripts/measure_decode_perf.py --rss-points 64`, whose
/// `crates/target/decode-perf/wems/fixture-6ch44100-64x.wem` compares equal by
/// SHA-256), so the ceiling this test asserts and the curve that finding
/// records describe the same 202.3 s of material.
fn long_stream_wem() -> PathBuf {
    let wav = read_pcm16(&fixtures_dir().join("input.wav")).expect("input.wav reads");
    let channels = wav.channels();
    let sample_rate = wav.sample_rate();
    let frames = wav.frames();
    let expected = (frames * STREAM_MULTIPLE) as u64;

    // Cargo's own scratch directory for integration tests: under the
    // workspace's gitignored `target/` tree, so a generated 7 MB WEM is never
    // untracked noise in `git status`.
    let directory = Path::new(env!("CARGO_TARGET_TMPDIR")).join("decode-memory");
    std::fs::create_dir_all(&directory).expect("the scratch directory is creatable");
    let path = directory.join(format!(
        "fixture-{channels}ch{sample_rate}-{STREAM_MULTIPLE}x.wem"
    ));
    if let Ok(cached) = std::fs::read(&path) {
        if declared_frames(&cached) == Ok(expected) {
            eprintln!(
                "[decode memory] reusing {} ({} declared frames)",
                path.display(),
                expected
            );
            return path;
        }
    }

    let source = wav.interleaved_le_bytes();
    let bytes_per_frame = channels * 2;
    let total = frames * STREAM_MULTIPLE;
    let mut session = StreamSession::for_selection(fixture_selection()).expect("session opens");
    let mut generated = 0usize;
    while generated < total {
        let take = GENERATE_CHUNK_FRAMES.min(total - generated);
        let mut chunk = Vec::with_capacity(take * bytes_per_frame);
        for frame in generated..generated + take {
            let source_frame = frame % frames;
            let start = source_frame * bytes_per_frame;
            chunk.extend_from_slice(&source[start..start + bytes_per_frame]);
        }
        session
            .push_pcm_chunk(&chunk)
            .expect("the streaming encoder accepts the tiled fixture PCM");
        generated += take;
    }
    let result = session.finish().expect("the streaming encode completes");
    let declared = declared_frames(&result.data).expect("the generated WEM declares its frames");
    assert_eq!(
        declared, expected,
        "the generated stream encodes {declared} frames, not the {expected} \
         {STREAM_MULTIPLE}x the fixture's {frames} asks for"
    );
    std::fs::write(&path, &result.data).expect("the generated WEM is writable");
    eprintln!(
        "[decode memory] generated {} ({} frames, {} bytes)",
        path.display(),
        declared,
        result.data.len()
    );
    path
}

/// The container's declared PCM frame count, from its own `fmt ` chunk — the
/// one number this test's arithmetic rests on
/// (`crates/wem-core/src/decoder.rs` is held to it at `Finish` too).
fn declared_frames(wem: &[u8]) -> Result<u64, String> {
    let (_, chunks) = parse_chunks(wem).map_err(|error| error.to_string())?;
    let fmt_chunk = chunks
        .iter()
        .find(|chunk| chunk.id == *b"fmt " && chunk.payload.len() == chunk.size as usize)
        .ok_or_else(|| "no complete fmt chunk".to_string())?;
    let fmt = VorbisFmtFields::parse(&fmt_chunk.payload).map_err(|error| error.to_string())?;
    if fmt.w_format_tag != WWISE_VORBIS_FORMAT_TAG {
        return Err(format!(
            "the container's format tag is {:#06x}, not the Wwise one",
            fmt.w_format_tag
        ));
    }
    Ok(u64::from(fmt.dw_total_pcm_frames))
}

/// Decode the whole WEM in bounded pushes, discarding every sample, and assert
/// the session delivered exactly what the container declares. Runs in the
/// child worker process.
fn drive_decode(path: &str) {
    let head = {
        let mut file = std::fs::File::open(path).expect("the WEM file opens");
        let mut head = vec![0u8; CHUNK_BYTES];
        let read = file.read(&mut head).expect("the WEM file reads");
        head.truncate(read);
        head
    };
    let declared = declared_frames(&head).expect("the container declares its frame count");

    let mut file = std::fs::File::open(path).expect("the WEM file opens");
    let mut session = DecodeSession::new();
    let mut buffer = vec![0u8; CHUNK_BYTES];
    let mut channels = 0usize;
    let mut frames = 0u64;
    loop {
        let read = file.read(&mut buffer).expect("the WEM file reads");
        if read == 0 {
            break;
        }
        let step = session.push_bytes(&buffer[..read]);
        channels = step
            .header
            .as_ref()
            .map_or(channels, |header| header.channels as usize);
        // `checked_div` is the geometry's own guard: before the header has been
        // announced there are no channels to divide by and no frames either.
        if let Some(in_step) = step.pcm.len().checked_div(channels) {
            frames += in_step as u64;
        }
        // `pcm` is dropped here: the caller keeps the samples, the session does
        // not, and this child is the caller.
        step.outcome.expect("the session accepts the WEM");
    }
    let step = session.finish();
    channels = step
        .header
        .as_ref()
        .map_or(channels, |header| header.channels as usize);
    if let Some(in_step) = step.pcm.len().checked_div(channels) {
        frames += in_step as u64;
    }
    step.outcome.expect("the session finishes the WEM");

    assert_eq!(
        frames, declared,
        "the child delivered {frames} frames of the {declared} the container declares"
    );
    eprintln!("[decode memory] worker decoded {frames} frames over {channels} channels");
}

// ---------------------------------------------------------------------------
// The measured worker (an `#[ignore]`d test function the parent spawns)
// ---------------------------------------------------------------------------

#[test]
#[ignore = "child worker: the memory check runs it with --ignored --exact"]
fn decode_memory_rss_worker() {
    let path = std::env::var("WEM_DECODE_WEM")
        .expect("WEM_DECODE_WEM names the WEM file this child decodes");
    drive_decode(&path);
}

// ---------------------------------------------------------------------------
// The memory check (forks the worker, asserts the ceiling, prints the reading)
// ---------------------------------------------------------------------------

/// Decode a 64x stream in a forked child and hold that child's peak RSS to
/// [`RSS_CEILING_BYTES`], printing the reading beside the verdict so a run log
/// carries the number and not only the pass.
///
/// The child is the test binary itself, run `--ignored --exact`; a child that
/// fails (a rejected packet, a frame-count shortfall, a panic) fails this test,
/// because a ceiling read off a child that did not decode is not a measurement.
#[test]
fn decode_memory_stays_bounded_across_stream_length() {
    // A debug build encodes the 202 s material and decodes it orders of
    // magnitude too slowly for a useful ceiling check.
    if cfg!(debug_assertions) {
        eprintln!("skipping the decode memory-ceiling test in a debug build");
        return;
    }
    let exe = std::env::current_exe().expect("current exe path");
    let wem = long_stream_wem();
    match child_peak_rss(&exe, WORKER, &wem) {
        Some(Ok(peak)) => {
            eprintln!(
                "[decode memory] {STREAM_MULTIPLE}x stream: child peak RSS {peak} bytes \
                 ({:.1} MB; {:.0}% of the {:.0} MB ceiling)",
                peak as f64 / 1e6,
                100.0 * peak as f64 / RSS_CEILING_BYTES as f64,
                RSS_CEILING_BYTES as f64 / 1e6,
            );
            assert!(
                peak < RSS_CEILING_BYTES,
                "the child decoding the {STREAM_MULTIPLE}x stream peaked at {peak} bytes RSS \
                 (>{RSS_CEILING_BYTES}); a live decode session must not retain a timeline that \
                 grows with the stream"
            );
        }
        Some(Err(message)) => {
            panic!("the decode RSS measurement failed: {message}");
        }
        None => {
            eprintln!("[decode memory] ru_maxrss unavailable on this platform; skipping");
        }
    }
}

/// Run the named worker as a child process and return its peak RSS (bytes).
/// `None` when the platform has no `ru_maxrss` measurement (then the check
/// skips rather than guess).
fn child_peak_rss(exe: &Path, worker: &str, wem: &Path) -> Option<Result<usize, String>> {
    #[cfg(target_os = "macos")]
    {
        macos_child_peak_rss(exe, worker, wem)
    }
    #[cfg(not(target_os = "macos"))]
    {
        let _ = (exe, worker, wem);
        None
    }
}

/// macOS: fork the worker, reap it with `wait4`, read `ru_maxrss` (bytes).
/// `ru_maxrss` sits at offset 32 of `struct rusage` on macOS (verified with a C
/// probe); the struct is 144 bytes on arm64/x86_64 darwin.
#[cfg(target_os = "macos")]
fn macos_child_peak_rss(exe: &Path, worker: &str, wem: &Path) -> Option<Result<usize, String>> {
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
    // `#[ignore]`d worker actually run. The material travels in the
    // environment, and the child's own output is left inherited so a failure
    // reports itself.
    cmd.arg("--ignored");
    cmd.arg("--exact");
    cmd.arg(worker);
    cmd.env("WEM_DECODE_WEM", wem);
    let child = match cmd.spawn() {
        Ok(child) => child,
        Err(error) => return Some(Err(format!("worker spawn failed: {error}"))),
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
    // The peak belongs to a decode that happened: a worker that exited non-zero
    // (a panic, a rejected packet, a short decode) has answered a different
    // question, and its reading must not be compared against the ceiling.
    let signalled = status & 0x7f;
    if signalled != 0 {
        return Some(Err(format!("the worker was killed by signal {signalled}")));
    }
    let code = (status >> 8) & 0xff;
    if code != 0 {
        return Some(Err(format!("the worker exited {code}")));
    }
    let peak = ru.ru_maxrss.max(0) as usize;
    Some(Ok(peak))
}
