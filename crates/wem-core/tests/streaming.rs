//! The streaming lifecycle and the feeder that drives it.
//!
//! `session` pins chunking invariance and the bounded input memory of a long
//! stream (measured in a forked child's peak RSS); `feeder_parity` proves the
//! incremental feeder selects exactly the samples the batch windowing kernel
//! selects, frame for frame, so the two paths cannot drift. One object, two
//! angles; each module keeps its own test names and assertion messages.

mod common;

mod session {
    //! StreamSession tests (the core streaming lifecycle).
    //!
    //! Three checks live here:
    //!
    //! 1. **Chunking invariance** — `StreamSession` output bytes equal
    //!    `Encoder::encode_pcm` for any chunk splitting (six+ strategies),
    //!    and the packets emitted through `push_pcm_chunk` replay exactly
    //!    the WEM container's packet sequence.
    //! 2. **Bounded input memory** — 300 s / 600 s / giant-chunk synthetic 6 ch
    //!    /44100 streams, and one short one-shot `Encoder::encode_pcm` batch
    //!    encode of the same synthetic material, each run in a freshly forked
    //!    child process whose peak RSS is read via `wait4().ru_maxrss`; each
    //!    child must stay under its ceiling. Measuring in the shared test
    //!    process is unreliable: parallel `#[test]` threads share one allocator
    //!    that never returns memory to the OS, so any in-process reading would
    //!    be inflated and mask the real bound. The only in-process memory
    //!    assertion is a static structural invariant (ring keep ==
    //!    `STREAM_RING_KEEP`), in `wem-analysis`. The batch case is what pins
    //!    the one-shot analysis input: it holds the caller's PCM (by
    //!    definition) but must not additionally materialize the frame sequence
    //!    before analysing any frame.
    //! 3. **The caller's cap** — the pool a session builds is the smaller of the
    //!    caller's cap and its channel count, and no cap moves a byte: every one
    //!    of them, including one worker and one above the channel count,
    //!    assembles the committed reference container.
    //!
    //! **No timing lives here.** This module used to assert `encode_pcm`'s
    //! release-mode median against a 150 ms budget; that assertion is gone,
    //! because a wall-clock budget on a shared machine measures the machine and
    //! flakes rather than stating something about the product. The numbers it
    //! wanted are measured without a verdict by `scripts/measure_encode_perf.py`
    //! (CLI stage timers as min / median / p95 / spread, with the machine
    //! identity and the load it ran under) and by
    //! `crates/wem-core/tests/stage_timings.rs` (the per-stage split,
    //! `#[ignore]`d). The RSS ceilings below are a different kind of claim and
    //! stay: how much input a session may accumulate is a property of the
    //! product, not of the machine that ran it. Byte assertions stay too — this
    //! module still compares the streaming output against `encode_pcm` byte for
    //! byte.

    use std::num::NonZeroUsize;

    use wem_container::load_wem_parts_bytes;
    use wem_core::encoder::{Encoder, EncoderOptions, Pcm16};
    use wem_core::stream::StreamSession;
    use wem_core::usecases::wav::read_pcm16;

    use crate::common::{fixture_selection, fixtures_dir, read_fixture};

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
        // A borrow of the WAV's own bytes: every strategy slices them.
        let le_bytes: &[u8] = wav.interleaved_le_bytes();
        let channels = wav.channels();

        let encoder = Encoder::new(fixture_selection()).expect("fs encoder builds");
        let pcm = wav.to_pcm16().expect("wav converts to Pcm16");
        let reference = encoder.encode_pcm(&pcm).expect("encode runs");

        for (name, cuts) in chunk_strategies(le_bytes.len(), channels as i64) {
            let (emitted, result) = run_session_with_chunks(le_bytes, &cuts);
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
    /// Peak-RSS ceiling for one *one-shot* batch encode of
    /// [`BATCH_RSS_DURATION_SECONDS`]. The batch path necessarily holds the
    /// caller-supplied PCM twice (the i16 bytes it was handed, plus the
    /// channel-major float rows the analysis reads) and the analysis input
    /// keeps the LPC priming/tail source it derives from them; what it must
    /// not hold is a second whole-frame pass over that PCM. At 10 s / 6 ch the
    /// float rows are ~21 MB, so the ceiling leaves room for the session's own
    /// input state plus the baseline and rejects a materialized frame
    /// sequence (~21 MB more at this duration, growing with the stream).
    const BATCH_RSS_CEILING_BYTES: usize = 100 * 1024 * 1024;
    /// Duration of the synthetic one-shot stream the batch RSS worker encodes.
    const BATCH_RSS_DURATION_SECONDS: i64 = 10;
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

    /// Encode one [`BATCH_RSS_DURATION_SECONDS`] synthetic stream through the
    /// one-shot path (`Encoder::encode_pcm`, no RSS assertion — the parent
    /// reads the child's peak). The batch path owns the caller's whole PCM by
    /// definition; what this worker measures is everything the analysis input
    /// adds on top of it. Runs in the child worker process.
    fn drive_batch_encode() {
        let encoder = Encoder::new(fixture_selection()).expect("profile loads");
        let frames = (SAMPLE_RATE * BATCH_RSS_DURATION_SECONDS) as usize;
        let bytes = synthetic_chunk(CHANNELS, frames, 0);
        let pcm = Pcm16::from_interleaved_le(SAMPLE_RATE, CHANNELS, bytes).expect("PCM geometry");
        let result = encoder.encode_pcm(&pcm).expect("batch encode ok");
        assert_eq!(
            result.stats.pcm_frames,
            BATCH_RSS_DURATION_SECONDS * SAMPLE_RATE,
            "frame accounting must match the batch duration"
        );
    }

    // --- Child-process worker tests ---
    //
    // These do the real streaming work, but only when the parent memory check
    // spawns them with libtest's own `--ignored --exact <name>` arguments. They
    // are `#[ignore]`d so a normal `#[test]` run reports them without executing
    // them, and parallel threads never contaminate each other's RSS. The parent
    // reads each child's peak via wait4().ru_maxrss.
    #[test]
    #[ignore = "child worker: the memory check runs it with --ignored --exact"]
    fn stream_memory_rss_worker_duration_300() {
        drive_duration_stream(300);
    }

    #[test]
    #[ignore = "child worker: the memory check runs it with --ignored --exact"]
    fn stream_memory_rss_worker_duration_600() {
        drive_duration_stream(600);
    }

    #[test]
    #[ignore = "child worker: the memory check runs it with --ignored --exact"]
    fn stream_memory_rss_worker_giant_chunk() {
        drive_giant_chunk();
    }

    #[test]
    #[ignore = "child worker: the memory check runs it with --ignored --exact"]
    fn batch_memory_rss_worker() {
        drive_batch_encode();
    }

    // --- Parent memory check (forks workers, asserts their ceiling, prints the reading) ---

    /// Drive each streaming worker in a forked child, hold its peak RSS to the
    /// case's ceiling, and print the reading.
    ///
    /// The ceiling is the assertion, because how much input a session
    /// accumulates is a property of the product. The reading (`[memory check]
    /// <case>: child peak RSS … bytes (… MB; …% of the ceiling)`) is printed
    /// next to it so a run log carries the number and not only the verdict, and
    /// the test still fails when the *measurement* fails (a worker that will not
    /// spawn, reap or report), because then the bound is simply unobserved.
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
            (
                "stream_memory_rss_worker_duration_300",
                "300s".to_string(),
                RSS_CEILING_BYTES,
            ),
            (
                "stream_memory_rss_worker_duration_600",
                "600s".to_string(),
                RSS_CEILING_BYTES,
            ),
            (
                "stream_memory_rss_worker_giant_chunk",
                "giant 60s".to_string(),
                RSS_CEILING_BYTES,
            ),
            (
                "batch_memory_rss_worker",
                format!("batch {BATCH_RSS_DURATION_SECONDS}s one-shot"),
                BATCH_RSS_CEILING_BYTES,
            ),
        ];
        for (worker, label, ceiling) in cases {
            match child_peak_rss(&exe, worker) {
                Some(Ok(peak)) => {
                    eprintln!(
                        "[memory check] {label}: child peak RSS {peak} bytes ({:.1} MB; \
                         {:.0}% of the {:.0} MB ceiling)",
                        peak as f64 / 1e6,
                        100.0 * peak as f64 / ceiling as f64,
                        ceiling as f64 / 1e6,
                    );
                    assert!(
                        peak < ceiling,
                        "{label} child peaked at {peak} bytes RSS (>{ceiling}); \
                     the session must not accumulate the input PCM"
                    );
                }
                Some(Err(msg)) => {
                    panic!("RSS measurement failed for '{label}': {msg}");
                }
                None => {
                    eprintln!(
                        "[memory check] {label}: ru_maxrss unavailable on this platform; skipping"
                    );
                }
            }
        }
    }

    /// Run the named worker as a child process and return its peak RSS
    /// (bytes). `None` when the platform has no ru_maxrss measurement
    /// (then the check skips rather than guess).
    fn child_peak_rss(exe: &std::path::Path, worker: &str) -> Option<Result<usize, String>> {
        #[cfg(target_os = "macos")]
        {
            macos_child_peak_rss(exe, worker)
        }
        #[cfg(not(target_os = "macos"))]
        {
            let _ = (exe, worker);
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
        let pcm = Pcm16::from_interleaved_le(SAMPLE_RATE, CHANNELS, pcm_bytes.as_slice())
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
    // 3. The caller's cap on the internal channel pool
    // ---------------------------------------------------------------------------

    /// The workers a session must hold for one cap in **this** configuration:
    /// the smaller of the cap and the channel count with the `parallel` feature
    /// on — the cap is a bound, not a target, so one above the channel count and
    /// no cap at all both leave the encode's own size — and the calling thread
    /// alone without it, where there is no pool and the option is inert.
    #[cfg(feature = "parallel")]
    fn expected_workers(cap: Option<usize>, channels: usize) -> usize {
        cap.unwrap_or(channels).min(channels)
    }

    #[cfg(not(feature = "parallel"))]
    fn expected_workers(_cap: Option<usize>, _channels: usize) -> usize {
        1
    }

    /// Push the whole fixture through one session opened with `cap` and return
    /// the container it assembled plus the pool size the session reported. One
    /// chunk, because chunk boundaries cannot matter here and the claim under
    /// test is the pool, not the splitting.
    fn encode_with_cap(le_bytes: &[u8], cap: Option<NonZeroUsize>) -> (Vec<u8>, usize) {
        let mut session = StreamSession::for_selection_with_options(
            fixture_selection(),
            EncoderOptions {
                quality: None,
                max_channel_pool_workers: cap,
            },
        )
        .expect("session opens");
        let workers = session.channel_pool_workers();
        session
            .push_pcm_chunk(le_bytes)
            .expect("the fixture is accepted as one chunk");
        let result = session.finish().expect("the stream finishes");
        (result.data, workers)
    }

    /// The cap selects the pool and nothing else: every cap — the encode's own
    /// size, one worker, two caps below the channel count and one above it —
    /// reports the pool it got and assembles the committed reference container
    /// byte for byte. This is the caller-facing form of "pool size cannot change
    /// a byte at any size" (`docs/findings/pool-sizing.md`), and it runs in both
    /// configurations: with the feature off the reading is one worker and the
    /// bytes are the same, which is what makes the option inert rather than
    /// merely ignored.
    #[test]
    fn the_caller_cap_sizes_the_pool_and_never_the_bytes() {
        let wav = read_pcm16(&fixtures_dir().join("input.wav")).expect("input.wav reads");
        let le_bytes: &[u8] = wav.interleaved_le_bytes();
        let channels = wav.channels() as usize;
        let reference = read_fixture("reference.wem");

        // `None` first: the uncapped container is what every capped one is also
        // compared against, so a cap that changed the bytes would be reported
        // against the same run's own uncapped output and not only against the
        // committed file.
        let mut uncapped: Option<Vec<u8>> = None;
        for cap in [None, Some(1), Some(2), Some(3), Some(6), Some(16)] {
            let requested = cap.map(|n| NonZeroUsize::new(n).expect("a positive cap"));
            let (data, workers) = encode_with_cap(le_bytes, requested);

            assert_eq!(
                workers,
                expected_workers(cap, channels),
                "cap {cap:?} must leave the session with the pool this configuration builds"
            );
            assert_eq!(
                data, reference,
                "cap {cap:?} must assemble the committed reference container byte for byte"
            );
            match &uncapped {
                Some(uncapped) => assert_eq!(
                    &data, uncapped,
                    "cap {cap:?} must assemble the same container as the uncapped encode"
                ),
                None => uncapped = Some(data),
            }
        }
    }
}

mod feeder_parity {
    //! Streaming feeder parity against the batch windowing kernel.
    //!
    //! The incremental `StreamingPcmFeeder` must select exactly the samples
    //! the batch `iter_planned_pcm_windows` selects, for realistic mode
    //! lists (the batch session's own mode loop), so the two paths cannot
    //! drift on any frame they both produce.

    use wem_analysis::preprocessing::streaming::{
        StreamingPcmFeeder, STREAM_DETECTOR_HOP, STREAM_DETECTOR_WINDOW,
    };
    use wem_analysis::preprocessing::windowing::iter_planned_pcm_windows;
    use wem_analysis::session::AnalysisSession;
    use wem_scheduling::plan_mode_sequence;

    /// Analysis resources of the installed Wwise 2013 6ch/44100 configuration,
    /// resolved from a structured selection against the compiled profile carrier
    /// (never a profile name or a profile tree).
    fn encoder_analysis_resources() -> wem_analysis::config::AnalysisProfileResources {
        let selection =
            wem_profiles::WwiseProfile::new(wem_profiles::WwiseVersion::Wwise2013, 6, 44_100)
                .expect("6ch/44100 selection");
        let compiled = wem_profiles::carrier::compiled_profile_for_selection(selection)
            .expect("installed 6ch profile resolves");
        wem_profiles::assemble_analysis_resources(&compiled, None)
            .expect("analysis resources assemble")
    }

    fn sine_like(frames: i64) -> Vec<Vec<f64>> {
        (0..6)
            .map(|ch| {
                (0..frames)
                    .map(|i| {
                        let phase = (i as f64 + ch as f64 * 3.0) * 0.017;
                        (phase as u32 % 628) as f64 * 0.01
                    })
                    .collect()
            })
            .collect()
    }

    #[test]
    fn streaming_frame_rows_match_batch_materialization() {
        let frames = 9000i64;
        let pcm = sine_like(frames);

        let resources = encoder_analysis_resources();
        let frozen = resources
            .frozen
            .as_ref()
            .expect("frozen windows present")
            .window_halves
            .clone();

        // Realistic mode list: the batch session's own mode loop (which stops
        // at source_len + prefix), not a fabricated sequence.
        let mut session = AnalysisSession::new(6, 44100, [256, 2048], resources).expect("session");
        let modes = session.select_modes(&pcm).expect("select modes");
        let plans = plan_mode_sequence(&modes, &[256, 2048], 1).expect("plans");

        // Batch reference.
        let batch_frames =
            iter_planned_pcm_windows(&pcm, &plans, &[256, 2048], Some(&frozen), None)
                .expect("batch");

        // Streaming path: chunked push, then EOS. The first chunk (3000
        // samples) precedes the 4096-sample prime batch, so its drain is
        // empty — quanta flow only from the second chunk on.
        let mut feeder = StreamingPcmFeeder::new(6, [256, 2048]).expect("feeder");
        let mut drained = 0usize;
        let mut offset = 0usize;
        for chunk in [3000usize, 3000, 3000] {
            let rows: Vec<Vec<f64>> = pcm
                .iter()
                .map(|channel| channel[offset..offset + chunk].to_vec())
                .collect();
            offset += chunk;
            feeder.push(&rows).expect("push");
            drained += feeder.drain_completed_quanta().expect("drain").len();
        }
        feeder.finish_source(None).expect("finish");
        drained += feeder.drain_completed_quanta().expect("drain tail").len();
        let batch_len =
            (feeder.detector_stream_length() - STREAM_DETECTOR_WINDOW) / STREAM_DETECTOR_HOP + 1;
        assert_eq!(
            drained as i64, batch_len,
            "every detector quantum must be handed out exactly once"
        );
        // All samples are retained (9000 < ring bound), so every plan window
        // is materializable.
        for (index, batch) in batch_frames.iter().enumerate() {
            let plan = &plans[index];
            let raw = feeder
                .frame_raw_rows(plan, &[256, 2048])
                .unwrap_or_else(|e| panic!("frame {index} not ready: {e:?}"));
            let window_modes = if plan.current == 0 {
                (0, 0, 0)
            } else {
                (plan.previous, plan.current, plan.following)
            };
            let streamed: Vec<Vec<f64>> = raw
                .iter()
                .map(|row| {
                    wem_analysis::dsp::transform::apply_vorbis_window(
                        row,
                        &[256, 2048],
                        window_modes.0,
                        window_modes.1,
                        window_modes.2,
                        Some(&frozen),
                    )
                    .expect("window applies")
                })
                .collect();
            if streamed != batch.samples {
                let (ch_row, bat_row) = streamed
                    .iter()
                    .zip(batch.samples.iter())
                    .find(|(x, y)| x != y)
                    .unwrap();
                let idx = ch_row
                    .iter()
                    .zip(bat_row.iter())
                    .position(|(x, y)| x != y)
                    .unwrap();
                panic!(
                "frame {index} diverges at idx {idx}: stream={} batch={} (plan start={}, end={}, total={})",
                ch_row[idx],
                bat_row[idx],
                plan.sample_start,
                plan.sample_end,
                feeder.total_samples()
            );
            }
        }
    }

    #[test]
    fn streaming_detector_quanta_match_batch_views() {
        use wem_analysis::preprocessing::streaming::{
            STREAM_DETECTOR_HOP, STREAM_DETECTOR_WINDOW, STREAM_TAIL_SAMPLES,
        };

        let frames = 9000i64;
        let pcm = sine_like(frames);
        let batch_quanta = wem_analysis::preprocessing::detector_input::iter_detector_quanta(
            &pcm,
            STREAM_DETECTOR_HOP,
            STREAM_DETECTOR_WINDOW,
            None,
            STREAM_TAIL_SAMPLES,
            None,
            &[256, 2048],
        )
        .expect("batch quanta");

        let mut feeder = StreamingPcmFeeder::new(6, [256, 2048]).expect("feeder");
        let mut offset = 0usize;
        for chunk in [2000usize, 3000, 4000] {
            let rows: Vec<Vec<f64>> = pcm
                .iter()
                .map(|channel| channel[offset..offset + chunk].to_vec())
                .collect();
            offset += chunk;
            feeder.push(&rows).expect("push");
            let windows = feeder.drain_completed_quanta().expect("drain");
            let base = feeder.quanta_extracted_through() - windows.len() as i64;
            for (delta, window) in windows.iter().enumerate() {
                assert_eq!(
                    window,
                    &batch_quanta[(base + delta as i64) as usize],
                    "quantum {} diverges from the batch detector stream",
                    base + delta as i64
                );
            }
        }
        feeder.finish_source(None).expect("finish");
        let tail_windows = feeder.drain_completed_quanta().expect("drain");
        let base = feeder.quanta_extracted_through() - tail_windows.len() as i64;
        for (delta, window) in tail_windows.iter().enumerate() {
            assert_eq!(window, &batch_quanta[(base + delta as i64) as usize]);
        }
        assert_eq!(
            feeder.quanta_extracted_through() as usize,
            batch_quanta.len()
        );
    }

    #[test]
    fn huge_single_chunk_keeps_every_quantum() {
        // One giant chunk (the committed 2ch/48k stream shape): every quantum
        // must still be extractable as it completes, none lost to eviction.
        let frames = 30000i64;
        let pcm = sine_like(frames);
        let batch_quanta = wem_analysis::preprocessing::detector_input::iter_detector_quanta(
            &pcm,
            64,
            128,
            None,
            8192,
            None,
            &[256, 2048],
        )
        .expect("batch quanta");

        let mut feeder = StreamingPcmFeeder::new(6, [256, 2048]).expect("feeder");
        let rows: Vec<Vec<f64>> = pcm.clone();
        feeder.push(&rows).expect("one giant push");
        let windows = feeder.drain_completed_quanta().expect("drain");
        let base = feeder.quanta_extracted_through() - windows.len() as i64;
        for (delta, window) in windows.iter().enumerate() {
            assert_eq!(
                window,
                &batch_quanta[(base + delta as i64) as usize],
                "giant-chunk quantum {} diverges",
                base + delta as i64
            );
        }
        feeder.settle();
        feeder.finish_source(None).expect("finish");
        let tail_windows = feeder.drain_completed_quanta().expect("drain");
        let base = feeder.quanta_extracted_through() - tail_windows.len() as i64;
        for (delta, window) in tail_windows.iter().enumerate() {
            assert_eq!(window, &batch_quanta[(base + delta as i64) as usize],);
        }
        assert_eq!(
            feeder.quanta_extracted_through() as usize,
            batch_quanta.len()
        );
    }
}
