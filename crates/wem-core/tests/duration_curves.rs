//! Duration-curve measurement harness — test-only, `#[ignore]`d, no thresholds.
//!
//! The owner's question is "how do peak memory and CPU scale with the length of
//! a realistic input, for both installed geometries, and is anything left worth
//! optimizing at five minutes?". This file is the instrument, not the answer:
//! it reports raw samples and asserts nothing about them.
//!
//! Shape: one `#[ignore]`d **worker** per path, each doing one encode in a
//! freshly spawned child process, and one `#[ignore]`d **parent reporter**
//! (`duration_curves_point`) that runs exactly one worker and prints the
//! child's peak RSS, user CPU, system CPU and wall time. That is the same
//! child-process pattern `streaming.rs` uses for its bounded-memory
//! check (`wait4().ru_maxrss`), with `ru_utime`/`ru_stime` read from the same
//! `rusage` so a run reports memory *and* CPU together, from the reaped child,
//! never from the shared test process (whose allocator never returns memory to
//! the OS and whose threads would contaminate any in-process reading).
//!
//! The four paths:
//!
//! * `noop` — the process baseline: the harness itself, no input, no encode.
//! * `load` — `read_pcm16` + `to_pcm16` only: what the caller's PCM costs
//!   resident (the WAV bytes plus the `Pcm16` copy), no analysis.
//! * `batch` — the one-shot path, `Encoder::encode_pcm`, over that PCM. The
//!   WAV bytes stay resident across the encode, exactly as the CLI holds them.
//! * `stream` — `StreamSession` fed from a *streaming* WAV reader one 10 s
//!   block at a time, so the whole input is never resident. This is what makes
//!   the streaming curve a statement about the session's ring buffer rather
//!   than about the caller's buffer.
//!
//! Three *phase* workers decompose the `batch` peak. They are not shipping
//! paths: each replays `Encoder::encode_pcm`'s own statements up to a point
//! (`to_float_rows`, then `condition_pcm`, then `selected_windows`) and holds
//! the result, so the difference between two consecutive phases is where the
//! one-shot path's resident bytes come from:
//!
//! * `rows`      — the float rows (`to_float_rows`).
//! * `condition` — the conditioned rows the analysis actually reads.
//! * `windows`   — the materialized frame sequence (`selected_windows`).
//! * `source`    — `selected_window_source` held, the shape `encode_pcm` uses.
//! * `detector`  — the two detector stream sets `select_modes` builds, held
//!   together as they are when the second is assigned over the first.
//!
//! One further worker decomposes the *output* side rather than the input side:
//!
//! * `stream_push` — the streaming push loop stopped before `finish`, so the
//!   `stream` peak is either the session's own retention (this worker) or the
//!   copies `finish` makes on top of it (the difference).
//! * `assembly`    — `build_vorbis_wem` alone, over a synthetic packet set
//!   carrying a *measured* packet count and output size, so the copies the
//!   container builder makes can be separated from the session's retention.
//!
//! Every worker reads the same generated WAV for its (duration, geometry), so
//! the only difference between the `batch` and `stream` curves is where the
//! input lives.
//!
//! Environment (set by `scripts/measure_duration_curves.py`):
//!
//! | variable | meaning |
//! |---|---|
//! | `WEM_CURVES_WAV` | the generated measurement WAV |
//! | `WEM_CURVES_SELECTION` | `fixture` (6ch/44100) or `two_channel` (2ch/48000) |
//! | `WEM_CURVES_GEOMETRY` | label carried through to the report line |
//! | `WEM_CURVES_PATH` | `batch` / `stream` / `load` / `noop` / `assembly` … |
//! | `WEM_CURVES_SECONDS` | nominal duration of the input |
//! | `WEM_CURVES_REP` | repetition index, carried through only |
//! | `WEM_CURVES_PACKETS` | `assembly` only: audio packet count to synthesize |
//! | `WEM_CURVES_PACKET_BYTES` | `assembly` only: bytes per synthesized packet |
//!
//! Run one point by hand:
//!
//! ```sh
//! WEM_CURVES_WAV=crates/target/curves/inputs/6ch44100-30s.wav \
//! WEM_CURVES_SELECTION=fixture WEM_CURVES_GEOMETRY=6ch44100 \
//! WEM_CURVES_PATH=batch WEM_CURVES_SECONDS=30 WEM_CURVES_REP=1 \
//! cargo test --release -p wem-core --test duration_curves -- \
//!   --ignored --exact duration_curves_point --nocapture
//! ```

use std::fs::File;
use std::io::{BufReader, Read};
use std::path::{Path, PathBuf};
use std::time::Instant;

use wem_analysis::preprocessing::detector_input::detector_pcm_streams;
use wem_analysis::session::AnalysisSession;
use wem_container::wem::build_vorbis_wem;
use wem_core::encoder::{Encoder, Pcm16};
use wem_core::stream::StreamSession;
use wem_core::usecases::wav::read_pcm16;
use wem_core::WwiseProfile;

mod common;

use common::{fixture_selection, two_channel_selection};

/// Streaming block size: the same 10 s block the existing bounded-memory
/// check drives, so this curve is comparable with that ceiling. The reader
/// turns it into whole frames once it knows the input's sample rate.
const STREAM_BLOCK_SECONDS: i64 = 10;

// ---------------------------------------------------------------------------
// The measurement spec (environment in, one report line out)
// ---------------------------------------------------------------------------

#[derive(Debug, Clone)]
struct Spec {
    wav: PathBuf,
    geometry: String,
    path: String,
    seconds: i64,
    rep: String,
}

impl Spec {
    fn from_env() -> Result<Self, String> {
        fn required(name: &str) -> Result<String, String> {
            std::env::var(name).map_err(|_| format!("{name} is not set"))
        }
        Ok(Spec {
            wav: PathBuf::from(required("WEM_CURVES_WAV")?),
            geometry: required("WEM_CURVES_GEOMETRY")?,
            path: required("WEM_CURVES_PATH")?,
            seconds: required("WEM_CURVES_SECONDS")?
                .parse()
                .map_err(|error| format!("WEM_CURVES_SECONDS: {error}"))?,
            rep: std::env::var("WEM_CURVES_REP").unwrap_or_else(|_| "1".to_string()),
        })
    }

    fn selection(&self) -> Result<WwiseProfile, String> {
        match std::env::var("WEM_CURVES_SELECTION").as_deref() {
            Ok("fixture") => Ok(fixture_selection()),
            Ok("two_channel") => Ok(two_channel_selection()),
            other => Err(format!(
                "WEM_CURVES_SELECTION must be `fixture` or `two_channel`, got {other:?}"
            )),
        }
    }

    /// The worker test this path runs.
    fn worker(&self) -> Result<&'static str, String> {
        match self.path.as_str() {
            "noop" => Ok("duration_curves_worker_noop"),
            "load" => Ok("duration_curves_worker_load"),
            "rows" => Ok("duration_curves_worker_rows"),
            "condition" => Ok("duration_curves_worker_condition"),
            "windows" => Ok("duration_curves_worker_windows"),
            "detector" => Ok("duration_curves_worker_detector"),
            "source" => Ok("duration_curves_worker_source"),
            "batch" => Ok("duration_curves_worker_batch"),
            "stream" => Ok("duration_curves_worker_stream"),
            "stream_push" => Ok("duration_curves_worker_stream_push"),
            "assembly" => Ok("duration_curves_worker_assembly"),
            other => Err(format!(
                "WEM_CURVES_PATH must be noop/load/rows/condition/windows/detector/source/\
                 batch/stream/stream_push/assembly, got {other:?}"
            )),
        }
    }
}

fn prefix(spec: &Spec, path: &str) -> String {
    format!(
        "geometry={} path={path} seconds={} rep={}",
        spec.geometry, spec.seconds, spec.rep
    )
}

// ---------------------------------------------------------------------------
// A streaming WAV reader (test-only)
// ---------------------------------------------------------------------------

/// Minimal streaming reader for the canonical signed-16 PCM WAV the input
/// generator writes.
///
/// `read_pcm16` materializes the whole file — which is the point of the one-shot
/// path — so the streaming worker cannot use it without measuring the caller's
/// buffer instead of the session's. This parses the RIFF chunk list from a
/// buffered reader and hands out the `data` chunk in whole-frame blocks; the
/// peak it adds is one block.
struct WavChunkReader {
    reader: BufReader<File>,
    channels: usize,
    sample_rate: i64,
    remaining: u64,
    block_frames: usize,
}

impl WavChunkReader {
    fn open(path: &Path, block_seconds: i64) -> Result<Self, String> {
        let mut reader =
            BufReader::with_capacity(1 << 16, File::open(path).map_err(|e| e.to_string())?);
        let mut header = [0u8; 12];
        reader.read_exact(&mut header).map_err(|e| e.to_string())?;
        if &header[0..4] != b"RIFF" || &header[8..12] != b"WAVE" {
            return Err(format!("{} is not a RIFF/WAVE file", path.display()));
        }
        let mut format = None;
        loop {
            let mut chunk = [0u8; 8];
            reader
                .read_exact(&mut chunk)
                .map_err(|e| format!("{}: truncated chunk header: {e}", path.display()))?;
            let size = u32::from_le_bytes([chunk[4], chunk[5], chunk[6], chunk[7]]) as u64;
            match &chunk[0..4] {
                b"fmt " => {
                    let mut body = vec![0u8; size as usize];
                    reader.read_exact(&mut body).map_err(|e| e.to_string())?;
                    if body.len() < 16 {
                        return Err(format!("{}: short `fmt ` chunk", path.display()));
                    }
                    let tag = u16::from_le_bytes([body[0], body[1]]);
                    let channels = u16::from_le_bytes([body[2], body[3]]) as usize;
                    let sample_rate =
                        u32::from_le_bytes([body[4], body[5], body[6], body[7]]) as i64;
                    let bits = u16::from_le_bytes([body[14], body[15]]);
                    if tag != 1 || bits != 16 {
                        return Err(format!(
                            "{}: expected PCM (format 1) 16-bit, got format {tag} / {bits} bits",
                            path.display()
                        ));
                    }
                    format = Some((channels, sample_rate));
                }
                b"data" => {
                    let (channels, sample_rate) = format
                        .ok_or_else(|| format!("{}: `data` before `fmt `", path.display()))?;
                    return Ok(WavChunkReader {
                        reader,
                        channels,
                        sample_rate,
                        remaining: size,
                        block_frames: (block_seconds * sample_rate) as usize,
                    });
                }
                _ => {
                    // Skip the chunk, plus its pad byte when `size` is odd.
                    let skip = size + (size & 1);
                    let mut sink = std::io::sink();
                    let copied = std::io::copy(&mut (&mut reader).take(skip), &mut sink)
                        .map_err(|e| e.to_string())?;
                    if copied != skip {
                        return Err(format!("{}: truncated chunk", path.display()));
                    }
                }
            }
        }
    }

    /// The next block of whole frames, or `None` at end of data.
    fn next_block(&mut self) -> Result<Option<Vec<u8>>, String> {
        let frame_bytes = self.channels * 2;
        let want = (self.block_frames * frame_bytes).min(self.remaining as usize);
        let want = want - want % frame_bytes;
        if want == 0 {
            return Ok(None);
        }
        let mut block = vec![0u8; want];
        self.reader
            .read_exact(&mut block)
            .map_err(|e| format!("short read in `data`: {e}"))?;
        self.remaining -= want as u64;
        Ok(Some(block))
    }
}

// ---------------------------------------------------------------------------
// Workers (each runs in its own child process)
// ---------------------------------------------------------------------------

/// Process baseline: the harness, no input, no encode.
#[test]
#[ignore = "worker: scripts/measure_duration_curves.py runs it through duration_curves_point"]
fn duration_curves_worker_noop() {
    let spec = Spec::from_env().expect("spec");
    println!(
        "wem_curves_worker {} load_ms=0.000 encode_ms=0.000 total_ms=0.000 frames=0 \
         channels=0 sample_rate=0 wem_bytes=0 chunks=0 input_bytes=0 conditioner=false",
        prefix(&spec, "noop")
    );
}

/// What the caller's PCM costs resident: the WAV bytes plus the `Pcm16` copy.
#[test]
#[ignore = "worker: scripts/measure_duration_curves.py runs it through duration_curves_point"]
fn duration_curves_worker_load() {
    let spec = Spec::from_env().expect("spec");
    let started = Instant::now();
    let wav = read_pcm16(&spec.wav).expect("input reads");
    let pcm = wav.to_pcm16().expect("PCM converts");
    let load = started.elapsed();
    // Both buffers are still alive here: this is the point of the path.
    let input_bytes =
        wav.interleaved_le_bytes().len() + pcm.frame_count() as usize * pcm.channel_count() * 2;
    println!(
        "wem_curves_worker {} load_ms={:.3} encode_ms=0.000 total_ms={:.3} frames={} \
         channels={} sample_rate={} wem_bytes=0 chunks=0 input_bytes={input_bytes} \
         conditioner=false",
        prefix(&spec, "load"),
        load.as_secs_f64() * 1e3,
        started.elapsed().as_secs_f64() * 1e3,
        pcm.frame_count(),
        pcm.channel_count(),
        pcm.sample_rate(),
    );
}

/// Phase replay of `Encoder::encode_pcm`'s statements up to the point named by
/// `WEM_CURVES_PATH`: the float rows, the conditioned rows, or the materialized
/// frame sequence. Returns the input's resident bytes and the phase wall time.
///
/// The statement order is `encode_pcm`'s (`encoder.rs`, `encode_pcm`):
/// `let pcm_rows = pcm.to_float_rows()`, then
/// `session.condition_pcm(&pcm_rows)`, then `session.selected_windows(..)`.
/// The rows are a binding rather than a temporary because `condition_pcm`
/// returns a `Cow` that *borrows* them when the profile selects no input
/// conditioner — which is the shape the shipped call has, and the shape this
/// replay has to keep.
fn drive_phase(path: &str, encoder: &Encoder, pcm: &Pcm16) -> (usize, std::time::Duration) {
    let blocksizes = encoder.profile().block_sizes();
    let resources = encoder.analysis_resources().clone();
    let build_session = || {
        AnalysisSession::new(
            pcm.channel_count() as i64,
            pcm.sample_rate(),
            blocksizes,
            resources.clone(),
        )
        .expect("analysis session builds")
    };
    let started = Instant::now();
    match path {
        "rows" => {
            let rows = pcm.to_float_rows();
            std::hint::black_box(&rows);
            (rows.iter().map(Vec::len).sum(), started.elapsed())
        }
        "condition" => {
            let mut session = build_session();
            let rows = pcm.to_float_rows();
            let conditioned = session.condition_pcm(&rows).expect("conditioning runs");
            std::hint::black_box(&conditioned);
            (conditioned.iter().map(Vec::len).sum(), started.elapsed())
        }
        "windows" => {
            let mut session = build_session();
            let rows = pcm.to_float_rows();
            let conditioned = session.condition_pcm(&rows).expect("conditioning runs");
            let (_modes, windows) = session
                .selected_windows(&conditioned)
                .expect("scheduling + windowing run");
            std::hint::black_box(&windows);
            (
                windows
                    .iter()
                    .map(|window| window.samples.iter().map(Vec::len).sum::<usize>())
                    .sum(),
                started.elapsed(),
            )
        }
        // The two terms `selected_window_source` added to the one-shot path.
        "detector" => {
            let rows = pcm.to_float_rows();
            // `select_modes` calls `detector_pcm_streams` twice into the *same*
            // binding, so the second call builds its result while the first is
            // still alive; this replays that overlap, which is where the peak
            // is, rather than one call on its own. Only `tail_training` is
            // approximated (its default is the one the first call passes), and
            // it selects LPC coefficients: it does not change the stream
            // length, which is what this measures.
            let first = detector_pcm_streams(&rows, None, 0, None, &blocksizes)
                .expect("first detector stream set builds");
            let second = detector_pcm_streams(&rows, None, blocksizes[1] * 3, None, &blocksizes)
                .expect("second detector stream set builds");
            std::hint::black_box((&first, &second));
            (
                first.iter().chain(second.iter()).map(Vec::len).sum(),
                started.elapsed(),
            )
        }
        "source" => {
            let mut session = build_session();
            let rows = pcm.to_float_rows();
            let conditioned = session.condition_pcm(&rows).expect("conditioning runs");
            let (_modes, source) = session
                .selected_window_source(&conditioned)
                .expect("scheduling + window source build");
            std::hint::black_box(&source);
            (source.plans().len(), started.elapsed())
        }
        other => panic!("unknown phase {other:?}"),
    }
}

/// Phase worker: `to_float_rows` only.
#[test]
#[ignore = "worker: scripts/measure_duration_curves.py runs it through duration_curves_point"]
fn duration_curves_worker_rows() {
    duration_curves_worker_phase("rows");
}

/// Phase worker: `condition_pcm` (the statement the shipped path runs).
#[test]
#[ignore = "worker: scripts/measure_duration_curves.py runs it through duration_curves_point"]
fn duration_curves_worker_condition() {
    duration_curves_worker_phase("condition");
}

/// Phase worker: `selected_windows` over the conditioned rows.
#[test]
#[ignore = "worker: scripts/measure_duration_curves.py runs it through duration_curves_point"]
fn duration_curves_worker_windows() {
    duration_curves_worker_phase("windows");
}

/// Phase worker: the detector stream sets, as `select_modes` builds them.
#[test]
#[ignore = "worker: scripts/measure_duration_curves.py runs it through duration_curves_point"]
fn duration_curves_worker_detector() {
    duration_curves_worker_phase("detector");
}

/// Phase worker: `selected_window_source` held, as `encode_pcm` holds it.
#[test]
#[ignore = "worker: scripts/measure_duration_curves.py runs it through duration_curves_point"]
fn duration_curves_worker_source() {
    duration_curves_worker_phase("source");
}

fn duration_curves_worker_phase(path: &str) {
    let spec = Spec::from_env().expect("spec");
    let encoder = Encoder::new(spec.selection().expect("selection")).expect("profile loads");
    let started = Instant::now();
    let wav = read_pcm16(&spec.wav).expect("input reads");
    let pcm: Pcm16 = wav.to_pcm16().expect("PCM converts");
    let load = started.elapsed();
    let input_bytes = wav.interleaved_le_bytes().len();
    let (phase_values, phase) = drive_phase(path, &encoder, &pcm);
    println!(
        "wem_curves_worker {} load_ms={:.3} phase_ms={:.3} total_ms={:.3} frames={} \
         channels={} sample_rate={} wem_bytes=0 chunks=0 input_bytes={input_bytes} \
         phase_values={phase_values} conditioner={}",
        prefix(&spec, path),
        load.as_secs_f64() * 1e3,
        phase.as_secs_f64() * 1e3,
        (load + phase).as_secs_f64() * 1e3,
        pcm.frame_count(),
        pcm.channel_count(),
        pcm.sample_rate(),
        encoder.analysis_resources().input_conditioner.is_some(),
    );
}

/// The one-shot path over the whole input at once.
#[test]
#[ignore = "worker: scripts/measure_duration_curves.py runs it through duration_curves_point"]
fn duration_curves_worker_batch() {
    let spec = Spec::from_env().expect("spec");
    let encoder = Encoder::new(spec.selection().expect("selection")).expect("profile loads");
    let started = Instant::now();
    let wav = read_pcm16(&spec.wav).expect("input reads");
    let pcm: Pcm16 = wav.to_pcm16().expect("PCM converts");
    let load = started.elapsed();
    assert_eq!(
        pcm.frame_count(),
        spec.seconds * pcm.sample_rate(),
        "frame count"
    );
    let started = Instant::now();
    let result = encoder.encode_pcm(&pcm).expect("one-shot encode");
    let encode = started.elapsed();
    // The WAV bytes stay resident across the encode, as the CLI holds them.
    let input_bytes = wav.interleaved_le_bytes().len();
    println!(
        "wem_curves_worker {} load_ms={:.3} encode_ms={:.3} total_ms={:.3} frames={} \
         channels={} sample_rate={} wem_bytes={} audio_packets={} chunks=1 input_bytes={input_bytes} \
         conditioner={}",
        prefix(&spec, "batch"),
        load.as_secs_f64() * 1e3,
        encode.as_secs_f64() * 1e3,
        (load + encode).as_secs_f64() * 1e3,
        pcm.frame_count(),
        pcm.channel_count(),
        pcm.sample_rate(),
        result.len(),
        result.stats.audio_packets,
        encoder.analysis_resources().input_conditioner.is_some(),
    );
}

/// The streaming path without `finish`: it attributes the `stream` curve.
///
/// `finish` builds the container, which requires every emitted packet to still
/// be reachable, so the `stream` peak is either the session's own retention
/// (this worker) or the copies `finish` makes on top of it (the difference).
/// Not a shipping path: the same push loop as `duration_curves_worker_stream`,
/// stopped before the terminal call.
#[test]
#[ignore = "worker: scripts/measure_duration_curves.py runs it through duration_curves_point"]
fn duration_curves_worker_stream_push() {
    let spec = Spec::from_env().expect("spec");
    let selection = spec.selection().expect("selection");
    let mut session = StreamSession::for_selection(selection).expect("session opens");
    let mut reader = WavChunkReader::open(&spec.wav, STREAM_BLOCK_SECONDS).expect("input opens");
    let started = Instant::now();
    let mut chunks = 0usize;
    let mut input_bytes = 0usize;
    while let Some(block) = reader.next_block().expect("block reads") {
        chunks += 1;
        input_bytes += block.len();
        session.push_pcm_chunk(&block).expect("chunk pushes");
    }
    let push = started.elapsed();
    let frames = session.pcm_frames();
    println!(
        "wem_curves_worker {} push_ms={:.3} finish_ms=0.000 total_ms={:.3} frames={} \
         channels={} sample_rate={} wem_bytes=0 chunks={chunks} input_bytes={input_bytes} \
         conditioner=false",
        prefix(&spec, "stream_push"),
        push.as_secs_f64() * 1e3,
        push.as_secs_f64() * 1e3,
        frames,
        reader.channels,
        reader.sample_rate,
    );
    std::hint::black_box(&session);
}

/// The streaming path, fed from a streaming reader so the whole input is never
/// resident.
#[test]
#[ignore = "worker: scripts/measure_duration_curves.py runs it through duration_curves_point"]
fn duration_curves_worker_stream() {
    let spec = Spec::from_env().expect("spec");
    let selection = spec.selection().expect("selection");
    let conditioner = Encoder::new(selection)
        .expect("profile loads")
        .analysis_resources()
        .input_conditioner
        .is_some();
    let mut session = StreamSession::for_selection(selection).expect("session opens");
    let mut reader = WavChunkReader::open(&spec.wav, STREAM_BLOCK_SECONDS).expect("input opens");
    let started = Instant::now();
    let mut chunks = 0usize;
    let mut input_bytes = 0usize;
    while let Some(block) = reader.next_block().expect("block reads") {
        chunks += 1;
        input_bytes += block.len();
        session.push_pcm_chunk(&block).expect("chunk pushes");
    }
    let load = started.elapsed();
    let started = Instant::now();
    let result = session.finish().expect("stream finishes");
    let encode = started.elapsed();
    assert_eq!(
        result.stats.pcm_frames,
        spec.seconds * reader.sample_rate,
        "frame accounting"
    );
    println!(
        "wem_curves_worker {} push_ms={:.3} finish_ms={:.3} total_ms={:.3} frames={} \
         channels={} sample_rate={} wem_bytes={} audio_packets={} chunks={chunks} input_bytes={input_bytes} \
         conditioner={conditioner}",
        prefix(&spec, "stream"),
        load.as_secs_f64() * 1e3,
        encode.as_secs_f64() * 1e3,
        (load + encode).as_secs_f64() * 1e3,
        result.stats.pcm_frames,
        reader.channels,
        reader.sample_rate,
        result.len(),
        result.stats.audio_packets,
    );
}

// ---------------------------------------------------------------------------
// Output-side diagnostic: the container assembly on its own
// ---------------------------------------------------------------------------

/// The container assembly alone, on a synthetic packet set of the measured
/// shape: what `finish` pays *inside* `build_vorbis_wem`, separated from the
/// session's own retention.
///
/// `finish` is one indivisible call, and `ru_maxrss` is a high-water mark that
/// a later `drop` cannot lower, so the assembly cannot be sequenced away from
/// the session inside one child. This worker therefore takes the *measured*
/// output shape as input — `WEM_CURVES_PACKETS` audio packets of
/// `WEM_CURVES_PACKET_BYTES` bytes, both read off the `stream` point of the
/// same (geometry, duration) — and runs nothing but the container builder. Its
/// child peak is then the packet set it holds plus whatever the builder
/// allocates on top of it.
///
/// The model is cross-checked, not asserted: the driver compares
/// `assembly - noop` against the `finish` addition measured through the real
/// `StreamSession`, which is the same quantity by the other route.
///
/// Not a shipping path: it encodes nothing and makes no claim about bytes.
#[test]
#[ignore = "worker: scripts/measure_duration_curves.py runs it through duration_curves_point"]
fn duration_curves_worker_assembly() {
    let spec = Spec::from_env().expect("spec");
    let encoder = Encoder::new(spec.selection().expect("selection")).expect("profile loads");
    let packets: usize = std::env::var("WEM_CURVES_PACKETS")
        .expect("WEM_CURVES_PACKETS is set")
        .parse()
        .expect("WEM_CURVES_PACKETS is an integer");
    let packet_bytes: usize = std::env::var("WEM_CURVES_PACKET_BYTES")
        .expect("WEM_CURVES_PACKET_BYTES is set")
        .parse()
        .expect("WEM_CURVES_PACKET_BYTES is an integer");

    // Only the magnitude matters here: a deterministic filler of the measured
    // size, so the payload and the packet count match the real container. The
    // first byte of a Vorbis packet carries its mode bit, and
    // `build_vorbis_wem` derives the rendered frame total from it and checks
    // that total against `dw_total_pcm_frames`; so the two are set to a
    // self-consistent pair — every packet long-mode, and the frame count that
    // many long packets render. Neither value takes part in what the builder
    // allocates, which is the quantity this worker measures.
    let blocksizes = encoder.profile().block_sizes();
    let long_hop = (blocksizes[1] + blocksizes[1]) / 4;
    let mut set: Vec<Vec<u8>> = Vec::with_capacity(1 + packets);
    set.push(encoder.setup_packet().to_vec());
    for _ in 0..packets {
        set.push(vec![1u8; packet_bytes.max(1)]);
    }
    let set_bytes: usize = set.iter().map(Vec::len).sum();
    let rendered_frames = long_hop * packets.saturating_sub(1) as i64;

    let mut fields = *encoder.container_plan().fmt();
    fields.dw_total_pcm_frames = rendered_frames as u32;
    let started = Instant::now();
    let built = build_vorbis_wem(
        fields,
        &set,
        encoder.container_plan().seek_table(),
        encoder.container_plan().endian(),
        encoder.container_plan().extra_chunks(),
        true,
        None,
    )
    .expect("container builds");
    let build = started.elapsed();

    // The packet set stays resident across the build, as `finish`'s clone and
    // the session's own retention do.
    std::hint::black_box(&set);
    println!(
        "wem_curves_worker {} load_ms=0.000 build_ms={:.3} total_ms={:.3} frames={} \
         channels={} sample_rate={} wem_bytes={} audio_packets={} packets={packets} \
         set_bytes={set_bytes} set_total_bytes={} rendered_frames={rendered_frames} \
         chunks=1 input_bytes=0 conditioner=false",
        prefix(&spec, "assembly"),
        build.as_secs_f64() * 1e3,
        build.as_secs_f64() * 1e3,
        spec.seconds * encoder.profile().sample_rate(),
        encoder.profile().channels(),
        encoder.profile().sample_rate(),
        built.wem_bytes.len(),
        packets,
        set_bytes + packets,
    );
    std::hint::black_box(&built);
}

// ---------------------------------------------------------------------------
// Parent reporter: fork one worker, read its rusage, print both
// ---------------------------------------------------------------------------

/// Run the worker `WEM_CURVES_PATH` names in a child process and print one
/// `wem_curves` line: the child's peak RSS, user/system CPU and wall time from
/// `wait4`, plus the child's own in-process stage fields.
#[test]
#[ignore = "measurement reporter: scripts/measure_duration_curves.py runs it with --ignored --nocapture"]
fn duration_curves_point() {
    let spec = Spec::from_env().expect("spec");
    let worker = spec.worker().expect("path names a worker");
    let exe = std::env::current_exe().expect("current exe path");
    let child = run_worker(&exe, worker, &spec).expect("worker runs");
    if child.exit_code != 0 {
        panic!(
            "worker {worker} exited {} for {} (stderr below)\n{}",
            child.exit_code,
            prefix(&spec, &spec.path),
            child.stderr
        );
    }
    let worker_line = child
        .stdout
        .lines()
        .find_map(|line| line.trim().strip_prefix("wem_curves_worker "))
        .unwrap_or_else(|| {
            panic!(
                "worker {worker} printed no `wem_curves_worker` line\nstdout:\n{}",
                child.stdout
            )
        });
    println!(
        "wem_curves {} ru_maxrss_bytes={} user_ms={:.3} sys_ms={:.3} wall_ms={:.3} worker={worker}",
        worker_line,
        child.max_rss_bytes,
        child.user.as_secs_f64() * 1e3,
        child.sys.as_secs_f64() * 1e3,
        child.wall.as_secs_f64() * 1e3,
    );
}

struct ChildRun {
    exit_code: i32,
    max_rss_bytes: u64,
    user: std::time::Duration,
    sys: std::time::Duration,
    wall: std::time::Duration,
    stdout: String,
    stderr: String,
}

/// Spawn the named worker as a child of this process, capture its output, and
/// reap it with `wait4` so the peak RSS and CPU come from the kernel's
/// `rusage` for exactly that process.
fn run_worker(exe: &Path, worker: &str, spec: &Spec) -> Result<ChildRun, String> {
    #[cfg(not(target_os = "macos"))]
    {
        let _ = (exe, worker, spec);
        return Err("ru_maxrss reporting is implemented for macOS only".to_string());
    }
    #[cfg(target_os = "macos")]
    {
        use std::process::{Command, Stdio};

        let mut command = Command::new(exe);
        command
            .arg("--ignored")
            .arg("--exact")
            .arg(worker)
            .arg("--nocapture")
            .env("WEM_CURVES_SECONDS", spec.seconds.to_string())
            .env("WEM_CURVES_PATH", &spec.path)
            .env("WEM_CURVES_REP", &spec.rep)
            .stdout(Stdio::piped())
            .stderr(Stdio::piped());
        let started = Instant::now();
        let mut child = command.spawn().map_err(|e| format!("spawn: {e}"))?;
        // Drain both pipes to EOF *before* reaping, so a full pipe buffer can
        // never deadlock the wait.
        let mut stdout = String::new();
        let mut stderr = String::new();
        if let Some(mut pipe) = child.stdout.take() {
            pipe.read_to_string(&mut stdout)
                .map_err(|e| e.to_string())?;
        }
        if let Some(mut pipe) = child.stderr.take() {
            pipe.read_to_string(&mut stderr)
                .map_err(|e| e.to_string())?;
        }
        let pid = child.id() as i32;
        let mut status: i32 = 0;
        let mut usage: MacRusage = unsafe { std::mem::zeroed() };
        let waited = unsafe { wait4(pid, &mut status, 0, &mut usage) };
        let wall = started.elapsed();
        if waited != pid {
            return Err(format!("wait4 returned {waited}, expected {pid}"));
        }
        Ok(ChildRun {
            exit_code: (status >> 8) & 0xff,
            max_rss_bytes: usage.ru_maxrss.max(0) as u64,
            user: duration_from(usage.ru_utime_sec, usage.ru_utime_usec),
            sys: duration_from(usage.ru_stime_sec, usage.ru_stime_usec),
            wall,
            stdout,
            stderr,
        })
    }
}

fn duration_from(sec: i64, usec: i64) -> std::time::Duration {
    std::time::Duration::new(sec.max(0) as u64, (usec.max(0) as u32) * 1_000)
}

/// `struct rusage` on macOS: `ru_maxrss` sits at offset 32 of a 144-byte
/// struct on arm64/x86_64 darwin (the same layout `streaming.rs` reads).
#[cfg(target_os = "macos")]
#[repr(C)]
struct MacRusage {
    ru_utime_sec: i64,
    ru_utime_usec: i64,
    ru_stime_sec: i64,
    ru_stime_usec: i64,
    ru_maxrss: i64,
    _rest: [i64; 13],
}

#[cfg(target_os = "macos")]
extern "C" {
    fn wait4(pid: i32, status: *mut i32, options: i32, rusage: *mut MacRusage) -> i32;
}
