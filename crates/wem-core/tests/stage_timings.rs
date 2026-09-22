//! Per-stage encode timing harness — a measurement, never a threshold.
//!
//! `Encoder::encode_pcm` reports one number. This harness reports where that
//! number goes: it replays the one-shot pipeline step by step through the
//! **public** kernel API (`AnalysisSession`, `iter_planned_pcm_windows`,
//! `pack_analysis_packet`, `build_vorbis_wem` — the same calls
//! `Encoder::encode_pcm` makes), wrapping each existing call in
//! `std::time::Instant`. No shipping code carries instrumentation for it.
//!
//! It asserts nothing about time. The only assertion is a *comparison*: the
//! container the staged replay assembles must equal the one `encode_pcm`
//! produces, so a stage split can never silently describe a pipeline other
//! than the shipped one. Remove that guard and every number here becomes
//! unfalsifiable.
//!
//! A second, clearly separated block *attributes* the coarse stages: it
//! re-runs `mdct_forward`, the `f32` rounding copy, the frozen-window
//! rebuild, and the transient detector on the material the first block
//! produced. Those are re-measurements of the same work, not disjoint
//! stages, and are labelled `attribution_*` accordingly.
//!
//! It is `#[ignore]`d: `cargo test --workspace --all-targets` reports it
//! without executing it, and `scripts/measure_encode_perf.py` runs it with
//! `--ignored --nocapture` and reads the `wem_perf` lines.
//!
//! Release only. A debug build is 10-30x slower and says nothing about the
//! shipped artifact, so the harness prints a skip notice and returns.

use std::path::PathBuf;
use std::time::{Duration, Instant};

use wem_analysis::config::f32_of;
use wem_analysis::dsp::spectrum::wwise_log_curve;
use wem_analysis::dsp::transform::{mdct_forward, vorbis_window};
use wem_analysis::preprocessing::detector_input::detector_pcm_streams;
use wem_analysis::session::AnalysisSession;
use wem_analysis::transient::detector::TransientDetector;
use wem_container::wem::build_vorbis_wem;
use wem_core::pack::pack_analysis_packet;
use wem_core::usecases::wav::read_pcm16;
use wem_core::{Encoder, Pcm16, StreamSession};

mod common;

use common::{fixture_selection, fixtures_dir, repo_root, two_channel_selection};

/// Block geometry the installed profiles use (validated by the session).
const BLOCKSIZES: [i64; 2] = [256, 2048];

/// Repetitions per corpus. The statistics (min/median/p95/spread) are the
/// caller's job; this harness prints raw samples so the caller owns them.
const RUNS: usize = 7;

// ---------------------------------------------------------------------------
// Report plumbing
// ---------------------------------------------------------------------------

/// Milliseconds with three decimals — enough to read a 0.01 ms stage, no
/// more precision than a wall clock on a shared machine can carry.
fn ms(duration: Duration) -> f64 {
    duration.as_secs_f64() * 1e3
}

/// One `key=value` field of a report line.
fn field(name: &str, value: f64) -> String {
    format!(" {name}={value:.3}")
}

// ---------------------------------------------------------------------------
// The staged replay
// ---------------------------------------------------------------------------

/// One full staged replay of `Encoder::encode_pcm`'s call sequence.
struct StagedRun {
    pcm_decode: Duration,
    condition: Duration,
    select_modes: Duration,
    plan_modes: Duration,
    /// Planning + window materialization, measured as a difference: the same
    /// session state and the same input through `select_modes` alone, then
    /// through `selected_windows` (which is `select_modes` plus planning plus
    /// windowing). Both go through the shipped path; nothing is re-derived.
    plan_and_windows: Duration,
    analyze_short: Duration,
    analyze_long: Duration,
    analyze_short_frames: usize,
    analyze_long_frames: usize,
    pack: Duration,
    container: Duration,
    staged_total: Duration,
    encode_pcm: Duration,
    bytes: usize,
    /// The staged container and the `encode_pcm` container, compared as bytes.
    matches_encode_pcm: bool,
    /// Everything the attribution block needs, kept from this pass.
    windows_materialized: Vec<wem_analysis::preprocessing::windowing::WindowedFrame>,
    conditioned: Vec<Vec<f64>>,
}

/// Replay one encode and time every bracketed stage.
///
/// The sequence is `Encoder::encode_pcm`'s, statement for statement:
/// `to_float_rows` -> `condition_pcm` -> `select_modes` / `selected_windows`
/// -> `analyze_window` per frame -> `pack_analysis_packet` per frame ->
/// `build_vorbis_wem`.
///
/// Scheduling is timed on its own session and planning+windowing is the
/// difference to a second, identically-fresh session's `selected_windows`.
/// That is deliberate: `selected_windows` captures its end-of-stream training
/// window from the selector cursor *inside* `select_modes`, so the value is
/// not reachable from outside, and re-deriving it in this file would put a
/// second copy of the shipping rule next to the shipping rule. Two fresh
/// sessions do identical work to the split point, so the difference is the
/// planning+windowing cost.
#[allow(clippy::too_many_lines)] // one linear replay; splitting it would hide the order
fn staged_run(encoder: &Encoder, pcm: &Pcm16) -> StagedRun {
    let channels = pcm.channel_count() as i64;
    let resources = encoder.analysis_resources().clone();
    let build_session = || {
        AnalysisSession::new(channels, pcm.sample_rate(), BLOCKSIZES, resources.clone())
            .expect("analysis session builds")
    };

    let mut decode_session = build_session();
    let start = Instant::now();
    let rows = pcm.to_float_rows();
    let pcm_decode = start.elapsed();

    let start = Instant::now();
    let conditioned = decode_session
        .condition_pcm(&rows)
        .expect("conditioning runs");
    let condition = start.elapsed();
    // The rows are *not* dropped here: `condition_pcm` borrows them when the
    // profile selects no input conditioner, and `encode_pcm` keeps its own
    // `pcm_rows` binding alive across the whole loop for the same reason.

    // Session A: the scheduler alone.
    let mut scheduler_session = build_session();
    let start = Instant::now();
    let modes = scheduler_session
        .select_modes(&conditioned)
        .expect("mode selection runs");
    let select_modes = start.elapsed();

    // Session B: the full `selected_windows` path, whose output drives the
    // rest of the pipeline (and the byte comparison below).
    let mut session = build_session();
    let start = Instant::now();
    let (windows_modes, windows) = session
        .selected_windows(&conditioned)
        .expect("scheduling + window materialization run");
    let plan_and_windows = start.elapsed();
    assert_eq!(
        windows_modes, modes,
        "the two fresh sessions must agree on the mode sequence"
    );

    let start = Instant::now();
    let plans = wem_scheduling::plan_mode_sequence(&modes, &BLOCKSIZES, 1).expect("plans build");
    let plan_modes = start.elapsed();
    assert_eq!(plans.len(), windows.len(), "one plan per window");
    drop(plans);

    let mut analyze_short = Duration::ZERO;
    let mut analyze_long = Duration::ZERO;
    let mut analyze_short_frames = 0usize;
    let mut analyze_long_frames = 0usize;
    let mut pack = Duration::ZERO;
    let mut audio_packets: Vec<Vec<u8>> = Vec::with_capacity(windows.len());
    for window in &windows {
        let long = window.current() == 1;
        let start = Instant::now();
        let analysis = session
            .analyze_window(window.clone(), None)
            .expect("analysis runs");
        let elapsed = start.elapsed();
        if long {
            analyze_long += elapsed;
            analyze_long_frames += 1;
        } else {
            analyze_short += elapsed;
            analyze_short_frames += 1;
        }
        let start = Instant::now();
        let packet = pack_analysis_packet(
            encoder.setup(),
            encoder.codebooks(),
            &analysis,
            channels as u32,
        )
        .expect("packet packs");
        pack += start.elapsed();
        audio_packets.push(packet);
    }

    let mut fields = *encoder.container_plan().fmt();
    fields.dw_total_pcm_frames = pcm.frame_count() as u32;
    let mut packets = Vec::with_capacity(1 + audio_packets.len());
    packets.push(encoder.setup_packet().to_vec());
    packets.extend(audio_packets);
    let start = Instant::now();
    let built = build_vorbis_wem(
        fields,
        &packets,
        encoder.container_plan().seek_table(),
        encoder.container_plan().endian(),
        encoder.container_plan().extra_chunks(),
        true,
        None,
    )
    .expect("container builds");
    let container = start.elapsed();

    let start = Instant::now();
    let reference = encoder.encode_pcm(pcm).expect("encode_pcm runs");
    let encode_pcm = start.elapsed();

    let staged_total =
        pcm_decode + condition + plan_and_windows + analyze_short + analyze_long + pack + container;

    StagedRun {
        pcm_decode,
        condition,
        select_modes,
        plan_modes,
        plan_and_windows,
        analyze_short,
        analyze_long,
        analyze_short_frames,
        analyze_long_frames,
        pack,
        container,
        staged_total,
        encode_pcm,
        bytes: built.wem_bytes.len(),
        matches_encode_pcm: built.wem_bytes == reference.data,
        windows_materialized: windows,
        // `into_owned` is free when a conditioner rewrote the rows and one
        // copy of the rows when it did not; either way it happens after every
        // timing bracket above, so no measured duration includes it.
        conditioned: conditioned.into_owned(),
    }
}

/// The attribution block: re-measure the inner pieces a bracketed stage can
/// only report as a whole. Every item here re-runs work the staged pass
/// already paid for; none of it is additive with the stage split.
struct Attribution {
    mdct: Duration,
    mdct_calls: usize,
    mdct_f32_roundtrip: Duration,
    fft_log_curve: Duration,
    fft_calls: usize,
    twiddle_recurrence: Duration,
    twiddle_steps: usize,
    window_rebuild: Duration,
    window_rebuild_calls: usize,
    transient: Duration,
    transient_quanta: usize,
}

fn attribution(
    encoder: &Encoder,
    run: &StagedRun,
) -> Result<Attribution, wem_analysis::config::AnalysisError> {
    let resources = encoder.analysis_resources();

    // --- MDCT and the f32 rounding copy that feeds it ---------------------
    let mut mdct = Duration::ZERO;
    let mut mdct_f32_roundtrip = Duration::ZERO;
    let mut mdct_calls = 0usize;
    let mut sink = 0.0f64;
    for window in &run.windows_materialized {
        let size = BLOCKSIZES[window.current() as usize];
        let look = resources
            .mdct_looks
            .get(&size)
            .expect("profile carries the block's MDCT look");
        for row in &window.samples {
            let start = Instant::now();
            let rounded: Vec<f64> = row.iter().map(|value| f32_of(*value)).collect();
            mdct_f32_roundtrip += start.elapsed();
            let start = Instant::now();
            let coeffs = mdct_forward(look, &rounded)?;
            mdct += start.elapsed();
            mdct_calls += 1;
            sink += coeffs[0];
        }
    }
    std::hint::black_box(sink);

    // --- FFT + log curve (the second half of `transform_one_channel`) ------
    let frozen_twiddles = resources.frozen_twiddles()?;
    let mut fft_log_curve = Duration::ZERO;
    let mut fft_calls = 0usize;
    for window in &run.windows_materialized {
        for row in &window.samples {
            let rounded: Vec<f64> = row.iter().map(|value| f32_of(*value)).collect();
            let start = Instant::now();
            let spectrum = wwise_log_curve(&rounded, frozen_twiddles)?;
            fft_log_curve += start.elapsed();
            fft_calls += 1;
            sink += spectrum[0];
        }
    }
    std::hint::black_box(sink);

    // --- Cost model: the FFT's twiddle recurrence -------------------------
    // `wwise_fft_packed` rebuilds the twiddle factor `(wr, wi)` from `(1, 0)`
    // by running complex multiplication once per butterfly, inside every
    // `start` block, instead of reading a sequence derived once per stage.
    // The recurrence is a pure function of `(step_re, step_im, k)`, so the
    // value it produces is the same at every block. This replays exactly that
    // recurrence — same operations, same `f32` rounding points, same
    // iteration count as the shipped loop — to price the redundancy. It is a
    // *cost model*, not part of the shipped path: it says what the recurrence
    // costs, not what removing it would save (the removed multiplies also
    // shorten the dependency chain).
    let mut twiddle_recurrence = Duration::ZERO;
    let mut twiddle_steps = 0usize;
    let mut twiddle_sink = 0.0f64;
    let start = Instant::now();
    for window in &run.windows_materialized {
        let n = BLOCKSIZES[window.current() as usize] as usize;
        // One replay per channel row: `wwise_log_curve` is called once per
        // channel per frame (1230 times), and this must match that count.
        for _row in &window.samples {
            // A unit-magnitude rotation, so `(wr, wi)` stays a normal float
            // for every `k`: the recurrence's cost is data-independent. The
            // pair goes through `black_box` so the recurrence cannot be
            // constant-folded away — the shipped loop reads it from a table
            // lookup, and a compile-time constant is a different program.
            let (step_re, step_im) = std::hint::black_box((0.999_998_9f64, 0.001_234_5f64));
            let mut length = 2usize;
            while length <= n {
                let half = length >> 1;
                let mut last_wr = 0.0f64;
                for _start_block in (0..n).step_by(length) {
                    let mut wr = 1.0f64;
                    let mut wi = 0.0f64;
                    for _k in 0..half {
                        let (new_wr, new_wi) = (
                            f32_of(wr * step_re - wi * step_im),
                            f32_of(wr * step_im + wi * step_re),
                        );
                        wr = new_wr;
                        wi = new_wi;
                        twiddle_steps += 1;
                    }
                    last_wr = wr;
                }
                twiddle_sink += last_wr;
                length <<= 1;
            }
        }
    }
    twiddle_recurrence += start.elapsed();
    std::hint::black_box(twiddle_sink);
    std::hint::black_box(twiddle_steps);

    // --- Frozen-window rebuild inside apply_vorbis_window ------------------
    // `apply_vorbis_window` calls its `get_window(size)` closure once per
    // neighbouring size, per channel, per frame; each call rebuilds the whole
    // symmetric window from the frozen half. Same geometry, same table, same
    // conversion — timed here at the exact call count the encode pays.
    let frozen = resources
        .frozen
        .as_ref()
        .expect("profile carries frozen window halves");
    let mut window_rebuild = Duration::ZERO;
    let mut window_rebuild_calls = 0usize;
    // `apply_vorbis_window` is called once per channel per frame, and each
    // call rebuilds both neighbouring windows — so the count is
    // frames * channels * 2, not frames * 2.
    for window in &run.windows_materialized {
        let current = window.current();
        let (left, right) = if current == 0 {
            (0, 0)
        } else {
            (window.previous(), window.following())
        };
        for _channel in &window.samples {
            for size in [BLOCKSIZES[left as usize], BLOCKSIZES[right as usize]] {
                let half = frozen
                    .window_halves
                    .get(&size)
                    .expect("profile carries the window half");
                let start = Instant::now();
                let widened: Vec<f64> = half.iter().map(|value| *value as f64).collect();
                let rebuilt = vorbis_window(size, Some(&widened))?;
                window_rebuild += start.elapsed();
                window_rebuild_calls += 1;
                sink += rebuilt[0];
            }
        }
    }
    std::hint::black_box(sink);

    // --- Transient detector ------------------------------------------------
    let short_bins = BLOCKSIZES[0] / 2;
    let hop = short_bins / 2;
    let mut transient = Duration::ZERO;
    let mut transient_quanta = 0usize;
    let start = Instant::now();
    let detector_streams = detector_pcm_streams(&run.conditioned, None, 0, None, &BLOCKSIZES)?;
    transient += start.elapsed();
    let pre_eos_quanta = (detector_streams[0].len() as i64 / hop - 4).max(0);
    let mut detector = TransientDetector::new(
        run.conditioned.len() as i64,
        resources.transient.clone(),
        resources
            .mdct_looks
            .get(&short_bins)
            .expect("profile carries the detector MDCT look")
            .clone(),
        short_bins,
    )?;
    let mut cooldown = 0i64;
    let mut flags = 0i64;
    {
        let ingest = |detector: &mut TransientDetector,
                      streams: &[Vec<f64>],
                      from: i64,
                      to: i64,
                      cooldown: &mut i64|
         -> Result<i64, wem_analysis::config::AnalysisError> {
            let mut acc = 0i64;
            for quantum_index in from..to {
                let begin = (quantum_index * hop) as usize;
                let end = begin + short_bins as usize;
                let quantum: Vec<Vec<f64>> = streams
                    .iter()
                    .map(|channel| channel[begin..end].to_vec())
                    .collect();
                *cooldown = (*cooldown + 1).min(24);
                acc |= detector.analyze_quantum(&quantum, *cooldown)?;
            }
            Ok(acc)
        };
        flags |= ingest(
            &mut detector,
            &detector_streams,
            0,
            pre_eos_quanta,
            &mut cooldown,
        )?;
        transient_quanta += pre_eos_quanta as usize;
        let start = Instant::now();
        // The tail training length is private to the session and only sizes
        // the LPC extrapolation (a few hundred samples); the dominant cost of
        // this call is the per-channel f32 copy of the whole source, which is
        // independent of it. The quanta below are the tail's ~32 of ~2200.
        let tail_streams = detector_pcm_streams(
            &run.conditioned,
            None,
            BLOCKSIZES[1] * 3,
            Some(BLOCKSIZES[1]),
            &BLOCKSIZES,
        )?;
        transient += start.elapsed();
        let available = (tail_streams[0].len() as i64 - short_bins) / hop + 1;
        flags |= ingest(
            &mut detector,
            &tail_streams,
            pre_eos_quanta,
            available,
            &mut cooldown,
        )?;
        transient_quanta += (available - pre_eos_quanta).max(0) as usize;
    }
    std::hint::black_box(flags);

    Ok(Attribution {
        mdct,
        mdct_calls,
        mdct_f32_roundtrip,
        fft_log_curve,
        fft_calls,
        twiddle_recurrence,
        twiddle_steps,
        window_rebuild,
        window_rebuild_calls,
        transient,
        transient_quanta,
    })
}

// ---------------------------------------------------------------------------
// Reporting
// ---------------------------------------------------------------------------

fn print_staged(corpus: &str, run: usize, sample: &StagedRun) {
    // `plan_and_windows_ms` contains `select_modes_ms` (see `staged_run`), so
    // it is the term that belongs in the sum; `select_modes_ms` and
    // `plan_modes_ms` are reported separately as diagnostics only.
    let mut line = format!("wem_perf corpus={corpus} run={run}");
    line += &field("pcm_decode_ms", ms(sample.pcm_decode));
    line += &field("condition_ms", ms(sample.condition));
    line += &field("select_modes_ms", ms(sample.select_modes));
    line += &field("plan_modes_ms", ms(sample.plan_modes));
    line += &field("plan_and_windows_ms", ms(sample.plan_and_windows));
    line += &field("analyze_short_ms", ms(sample.analyze_short));
    line += &field("analyze_long_ms", ms(sample.analyze_long));
    line += &field("pack_ms", ms(sample.pack));
    line += &field("container_ms", ms(sample.container));
    line += &field("staged_total_ms", ms(sample.staged_total));
    line += &field("encode_pcm_ms", ms(sample.encode_pcm));
    line += &format!(" short_frames={}", sample.analyze_short_frames);
    line += &format!(" long_frames={}", sample.analyze_long_frames);
    line += &format!(" bytes={}", sample.bytes);
    line += &format!(" staged_matches_encode_pcm={}", sample.matches_encode_pcm);
    println!("{line}");
}

fn print_attribution(corpus: &str, run: usize, attribution: &Attribution) {
    let mut line = format!("wem_perf_attribution corpus={corpus} run={run}");
    line += &field("mdct_ms", ms(attribution.mdct));
    line += &field("mdct_f32_roundtrip_ms", ms(attribution.mdct_f32_roundtrip));
    line += &field("fft_log_curve_ms", ms(attribution.fft_log_curve));
    line += &field(
        "fft_twiddle_recurrence_cost_model_ms",
        ms(attribution.twiddle_recurrence),
    );
    line += &field("window_rebuild_ms", ms(attribution.window_rebuild));
    line += &field("transient_ms", ms(attribution.transient));
    line += &format!(" mdct_calls={}", attribution.mdct_calls);
    line += &format!(" fft_calls={}", attribution.fft_calls);
    line += &format!(" twiddle_steps={}", attribution.twiddle_steps);
    line += &format!(" window_rebuild_calls={}", attribution.window_rebuild_calls);
    line += &format!(" transient_quanta={}", attribution.transient_quanta);
    println!("{line}");
}

// ---------------------------------------------------------------------------
// Corpora
// ---------------------------------------------------------------------------

fn wav_files(directory: &PathBuf) -> Vec<PathBuf> {
    let mut files: Vec<PathBuf> = std::fs::read_dir(directory)
        .expect("corpus directory reads")
        .filter_map(|entry| entry.ok().map(|entry| entry.path()))
        .filter(|path| path.extension().is_some_and(|extension| extension == "wav"))
        .collect();
    files.sort();
    files
}

/// The fixture: the 6ch/44100 stream the whole byte-exactness claim is
/// stated over, measured stage by stage.
fn measure_fixture() -> Result<(), String> {
    let wav = read_pcm16(&fixtures_dir().join("input.wav")).map_err(|e| e.to_string())?;
    let pcm = wav.to_pcm16().map_err(|e| e.to_string())?;
    let encoder = Encoder::new(fixture_selection()).map_err(|e| e.to_string())?;

    println!(
        "wem_perf_fixture channels={} sample_rate={} frames={} seconds={:.3} \
         conditioner={}",
        pcm.channel_count(),
        pcm.sample_rate(),
        pcm.frame_count(),
        pcm.frame_count() as f64 / pcm.sample_rate() as f64,
        encoder.analysis_resources().input_conditioner.is_some(),
    );

    let mut first_mismatch: Option<bool> = None;
    for run in 0..RUNS {
        let sample = staged_run(&encoder, &pcm);
        if run == 0 {
            first_mismatch = Some(sample.matches_encode_pcm);
        }
        print_staged("fixture", run, &sample);
        let attribution = attribution(&encoder, &sample).map_err(|e| e.to_string())?;
        print_attribution("fixture", run, &attribution);
    }

    // A comparison, not a threshold: if the staged replay ever stops
    // reproducing `encode_pcm`, every number above describes a pipeline that
    // is not the shipped one.
    if first_mismatch == Some(false) {
        return Err(
            "staged replay assembled different container bytes than encode_pcm; \
             the stage split would describe a different pipeline"
                .to_string(),
        );
    }
    Ok(())
}

/// The 2ch/48000 corpora: end-to-end totals only (the stage split is the
/// fixture's; these confirm the geometry change moves the total the way the
/// stage split predicts).
fn measure_two_channel() -> Result<(), String> {
    let encoder = Encoder::new(two_channel_selection()).map_err(|e| e.to_string())?;
    let mut corpus: Vec<(String, PathBuf)> = Vec::new();
    for directory in [
        repo_root().join("tests/data/2ch-reference"),
        repo_root().join("tests/data/2ch-stress"),
    ] {
        for path in wav_files(&directory) {
            let name = path
                .file_stem()
                .map(|stem| stem.to_string_lossy().into_owned())
                .unwrap_or_else(|| "?".to_string());
            corpus.push((name, path));
        }
    }
    for (name, path) in corpus {
        let wav = read_pcm16(&path).map_err(|e| e.to_string())?;
        let pcm = wav.to_pcm16().map_err(|e| e.to_string())?;
        for run in 0..RUNS {
            let start = Instant::now();
            let result = encoder.encode_pcm(&pcm).map_err(|e| e.to_string())?;
            let elapsed = start.elapsed();
            println!(
                "wem_perf corpus=2ch/{name} run={run} encode_pcm_ms={:.3} frames={} bytes={}",
                ms(elapsed),
                result.stats.pcm_frames,
                result.len(),
            );
        }
    }
    Ok(())
}

/// Streaming versus one-shot on the fixture: the same PCM through
/// `StreamSession` in one chunk and in 4096-frame chunks, against
/// `encode_pcm`. The two must produce the same bytes (a comparison); the
/// timing difference is what the incremental path's buffer handling costs or
/// saves.
///
/// The push loop is timed together with `finish`, because the deferred tail
/// frames are finished there; the split is printed separately so the tail's
/// share is visible.
fn measure_streaming() -> Result<(), String> {
    let wav = read_pcm16(&fixtures_dir().join("input.wav")).map_err(|e| e.to_string())?;
    let pcm = wav.to_pcm16().map_err(|e| e.to_string())?;
    let encoder = Encoder::new(fixture_selection()).map_err(|e| e.to_string())?;
    let batch = encoder.encode_pcm(&pcm).map_err(|e| e.to_string())?.data;
    let bytes = wav.interleaved_le_bytes().to_vec();
    let channels = wav.channels() as usize;
    let frame_bytes = channels * 2;

    for (label, chunk_frames) in [
        ("one-chunk", bytes.len() / frame_bytes),
        ("4096-frame-chunks", 4096usize),
    ] {
        for run in 0..RUNS {
            let mut session =
                StreamSession::for_selection(fixture_selection()).map_err(|e| e.to_string())?;
            let start = Instant::now();
            let mut emitted = 0usize;
            for chunk in bytes.chunks(chunk_frames * frame_bytes) {
                emitted += session
                    .push_pcm_chunk(chunk)
                    .map_err(|e| e.to_string())?
                    .len();
            }
            let push = start.elapsed();
            let start = Instant::now();
            let result = session.finish().map_err(|e| e.to_string())?;
            let finish = start.elapsed();
            println!(
                "wem_perf corpus=stream/{label} run={run} push_ms={:.3} finish_ms={:.3} \
                 stream_total_ms={:.3} emitted_packets={} bytes={} stream_matches_batch={}",
                ms(push),
                ms(finish),
                ms(push + finish),
                emitted,
                result.len(),
                result.data == batch,
            );
        }
    }
    Ok(())
}

#[test]
#[ignore = "measurement harness: scripts/measure_encode_perf.py runs it with --ignored --nocapture"]
fn stage_timings_report() {
    if cfg!(debug_assertions) {
        eprintln!(
            "stage_timings: skipped in a debug build; timings require \
             `cargo test --release -p wem-core --test stage_timings`"
        );
        return;
    }
    measure_fixture().expect("fixture stage measurement");
    measure_two_channel().expect("2ch corpus measurement");
    measure_streaming().expect("streaming comparison");
}

// ---------------------------------------------------------------------------
// Duration-curve driver (additive: no line above this point changed)
// ---------------------------------------------------------------------------

/// The per-stage split at the durations the duration-curve lane measures.
///
/// `measure_fixture` above replays the *fixture* (3.161 s / 6 ch) stage by
/// stage. This driver replays a generated measurement WAV through exactly the
/// same `staged_run`, with the same `staged_matches_encode_pcm` guard, so the
/// stage *shares* can be compared across durations instead of assumed
/// constant. It is additive: the two reporting tests above are untouched, and
/// `scripts/measure_duration_curves.py` is the only caller that selects this
/// name.
///
/// Environment (set by `scripts/measure_duration_curves.py`):
///
/// | variable | meaning |
/// |---|---|
/// | `WEM_CURVES_WAV` | the generated measurement WAV (required) |
/// | `WEM_CURVES_SELECTION` | `fixture` (6ch/44100) or `two_channel` (2ch/48000) |
/// | `WEM_CURVES_LABEL` | corpus name carried into the `wem_perf` lines |
/// | `WEM_CURVES_RUNS` | repetitions (default 3) |
/// | `WEM_CURVES_ATTRIBUTION` | `0` skips the single-threaded attribution block |
///
/// The attribution block re-runs work the staged pass already paid for, so it
/// is printed for the first run only and is not additive with the split.
#[test]
#[ignore = "measurement harness: scripts/measure_duration_curves.py runs it with --ignored --nocapture"]
fn stage_timings_duration_report() {
    if cfg!(debug_assertions) {
        eprintln!(
            "stage_timings_duration_report: skipped in a debug build; timings require \
             `cargo test --release -p wem-core --test stage_timings`"
        );
        return;
    }
    let wav_path = std::env::var("WEM_CURVES_WAV").expect("WEM_CURVES_WAV is set");
    let selection = match std::env::var("WEM_CURVES_SELECTION").as_deref() {
        Ok("fixture") => fixture_selection(),
        Ok("two_channel") => two_channel_selection(),
        other => panic!("WEM_CURVES_SELECTION must be `fixture` or `two_channel`, got {other:?}"),
    };
    let label = std::env::var("WEM_CURVES_LABEL").unwrap_or_else(|_| "curves".to_string());
    let runs: usize = std::env::var("WEM_CURVES_RUNS")
        .ok()
        .and_then(|value| value.parse().ok())
        .unwrap_or(3);
    let with_attribution = std::env::var("WEM_CURVES_ATTRIBUTION").as_deref() != Ok("0");

    let wav = read_pcm16(&PathBuf::from(&wav_path)).expect("measurement input reads");
    let pcm = wav.to_pcm16().expect("measurement input converts to PCM");
    let encoder = Encoder::new(selection).expect("profile loads");
    // A comparison, not a threshold: the input must be the geometry the
    // profile selection names, or the split would describe another encoder.
    assert_eq!(
        encoder.profile().channels(),
        pcm.channel_count() as i64,
        "input channel count does not match the selected profile"
    );
    assert_eq!(
        encoder.profile().sample_rate(),
        pcm.sample_rate(),
        "input sample rate does not match the selected profile"
    );
    println!(
        "wem_perf_input corpus={label} channels={} sample_rate={} frames={} seconds={:.3} \
         conditioner={}",
        pcm.channel_count(),
        pcm.sample_rate(),
        pcm.frame_count(),
        pcm.frame_count() as f64 / pcm.sample_rate() as f64,
        encoder.analysis_resources().input_conditioner.is_some(),
    );

    let mut first_mismatch: Option<bool> = None;
    for run in 0..runs {
        let sample = staged_run(&encoder, &pcm);
        if run == 0 {
            first_mismatch = Some(sample.matches_encode_pcm);
        }
        print_staged(&label, run, &sample);
        if with_attribution && run == 0 {
            let attributed = attribution(&encoder, &sample).expect("attribution runs");
            print_attribution(&label, run, &attributed);
        }
    }
    if first_mismatch == Some(false) {
        panic!(
            "staged replay assembled different container bytes than encode_pcm; \
             the stage split would describe a different pipeline"
        );
    }
}
