//! Per-stage decode timing harness — a measurement, never a threshold.
//!
//! `DecodeSession` reports PCM, not where the time went. This harness reports
//! where it goes: it replays the session's call sequence step by step through
//! the **public** kernel API — the container walk
//! (`wem_container::riff::parse_chunks`), the setup resolution (`parse_setup`
//! plus the compiled carrier's codebooks, MDCT looks and frozen windows), the
//! per-packet chain (`parse_audio_header`, `decode_floors_for_packet`,
//! `decode_residue_coeffs`, `apply_mapping_coupling_inverse`, `floor1_unwrap`,
//! `floor1_curve_from_posts`) and the synthesis (`SynthesisOla::push` /
//! `finish`) — wrapping each existing call in `std::time::Instant`. No
//! shipping code carries instrumentation for it.
//!
//! It asserts nothing about time. The only assertion is a *comparison*: the
//! interleaved f32 PCM the staged replay produces must be the PCM
//! `DecodeSession` delivers **bit for bit**, both when the session is fed the
//! WEM in one chunk and when it is fed in bounded chunks. Remove that guard
//! and every number here becomes unfalsifiable.
//!
//! It is `#[ignore]`d: `cargo test --workspace --all-targets` reports it
//! without executing it, and `scripts/measure_decode_perf.py` runs it with
//! `--ignored --exact decode_stage_timings_report --nocapture` and reads the
//! `wem_decode_perf` lines. The file's other reporting entry,
//! `decode_process_point`, is the **child** the measurement scripts spawn
//! directly (one process per measurement point, so that child's own `wait4`
//! rusage is the number); it decodes one WEM from a path and prints one
//! `wem_decode_child` line.
//!
//! Release only. A debug build is 10-30x slower and says nothing about the
//! shipped artifact, so both entries print a skip notice and return.
//!
//! Declared ambient reads (this is everything either entry reads from the
//! environment; the clock is what is being measured):
//!
//! | variable | entry | meaning |
//! |---|---|---|
//! | `WEM_DECODE_RUNS` | report | repetitions per corpus (default 5) |
//! | `WEM_DECODE_CHUNK` | report | bytes per push for the chunked replay (default 65536) |
//! | `WEM_DECODE_WEM` | child | the WEM file to decode (required) |
//! | `WEM_DECODE_CHUNK` | child | bytes per read/push; `0` reads the file whole (default 65536) |
//! | `WEM_DECODE_LABEL` | child | corpus name carried into the child's line (default the file name) |

use std::collections::HashMap;
use std::io::{Read, Seek};
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use wem_analysis::config::MdctLook;
use wem_analysis::dsp::transform::SynthesisOla;
use wem_container::fmt::{VorbisFmtFields, WWISE_VORBIS_FORMAT_TAG};
use wem_container::riff::{parse_chunks, Endian};
use wem_core::decoder::{DecodeSession, DecodedHeader};
use wem_profiles::carrier::compiled_profile_for_selection;
use wem_profiles::codebooks::load_setup_codebooks;
use wem_profiles::selection::{WwiseProfile, WwiseVersion};
use wem_vorbis::bitio::BitReader;
use wem_vorbis::codebook::Codebook;
use wem_vorbis::floor::{
    floor1_curve_from_posts, floor1_unwrap, postlist_from_floor, FLOOR1_RANGES,
};
use wem_vorbis::packet_decoder::{
    coupling_dirty_nonzero, decode_floors_for_packet, parse_audio_header,
};
use wem_vorbis::packet_encoder::apply_mapping_coupling_inverse;
use wem_vorbis::residue::{decode_residue_coeffs, ResidueStatus};
use wem_vorbis::setup::{parse_setup, Mapping0Setup, SetupInfo};

mod common;

use common::repo_root;

/// The mode of the block after the last one: `decoder.rs`'s own rule (a
/// decoder reads no window flags from a packet in this format revision).
const TERMINAL_FOLLOWING: i64 = 1;

/// Bytes per push for the chunked session replay.
const DEFAULT_CHUNK_BYTES: usize = 64 * 1024;

/// Repetitions per corpus. The statistics (min/median/p95/spread) are the
/// caller's job; this harness prints raw samples so the caller owns them.
const DEFAULT_RUNS: usize = 5;

// ---------------------------------------------------------------------------
// Report plumbing
// ---------------------------------------------------------------------------

/// Milliseconds with three decimals — enough to read a 0.010 ms stage, no more
/// precision than a wall clock on a shared machine can carry.
fn ms(duration: Duration) -> f64 {
    duration.as_secs_f64() * 1e3
}

/// One `key=value` field of a report line.
fn field(name: &str, value: f64) -> String {
    format!(" {name}={value:.3}")
}

fn env_usize(name: &str, default: usize) -> usize {
    std::env::var(name)
        .ok()
        .and_then(|value| value.trim().parse().ok())
        .unwrap_or(default)
}

/// Bit-for-bit equality of two interleaved f32 streams.
///
/// Bits, not `==`: two runs of the same arithmetic must agree exactly, and
/// `==` would call `-0.0` and `0.0` equal and report `NaN` as a mismatch.
fn same_samples(left: &[f32], right: &[f32]) -> bool {
    left.len() == right.len()
        && left
            .iter()
            .zip(right.iter())
            .all(|(a, b)| a.to_bits() == b.to_bits())
}

// ---------------------------------------------------------------------------
// The staged replay
// ---------------------------------------------------------------------------

/// Per-stage accumulators, all wall clock around calls that already exist.
#[derive(Default, Clone, Copy)]
struct Stages {
    /// Container framing: the chunk walk, the `fmt ` payload and the packet
    /// size prefixes.
    framing: Duration,
    /// Setup resolution and the session's per-stream state.
    setup: Duration,
    /// `parse_audio_header`, once per packet.
    header: Duration,
    /// `decode_floors_for_packet`, once per packet.
    floors: Duration,
    /// `decode_residue_coeffs` for every submap, plus the early-end check.
    residue: Duration,
    /// `coupling_dirty_nonzero` and `apply_mapping_coupling_inverse`.
    coupling: Duration,
    /// `postlist_from_floor` + `floor1_unwrap` + `floor1_curve_from_posts`
    /// and the per-bin envelope multiply.
    envelope: Duration,
    /// `SynthesisOla::push`, once per channel per block.
    ola: Duration,
    /// The interleave/clamp the session performs on what the overlap-add
    /// completed (`append_completed`).
    interleave: Duration,
}

impl Stages {
    fn total(&self) -> Duration {
        self.framing
            + self.setup
            + self.header
            + self.floors
            + self.residue
            + self.coupling
            + self.envelope
            + self.ola
            + self.interleave
    }
}

/// One decoded block waiting for the *following* block's mode — the session's
/// `PendingBlock`, rebuilt from public calls.
struct Block {
    mode: i64,
    spectra: Vec<Vec<f64>>,
}

/// The resolved decode configuration — the session's `Codec`, rebuilt from the
/// public carrier and the public decode-direction stages.
struct Codec {
    channels: usize,
    blocksizes: [i64; 2],
    setup: SetupInfo,
    books: Vec<Codebook>,
    looks: [MdctLook; 2],
    windows: HashMap<i64, Vec<f32>>,
    ola: Vec<SynthesisOla>,
    total_frames: u64,
    emitted: u64,
    pending: Option<Block>,
}

/// The submap one channel belongs to: the rule `decoder.rs` and the floor
/// reader both apply (a single-submap mapping puts every channel in submap 0).
fn submap_of(mapping: &Mapping0Setup, channel: usize) -> usize {
    if mapping.submaps > 1 {
        mapping.chmux.get(channel).copied().unwrap_or(0) as usize
    } else {
        0
    }
}

/// Resolve the container's geometry and setup packet against the compiled
/// carrier — `DecodeSession::open_codec`'s statements, in its order, from the
/// public carrier API.
fn open_codec(fmt: &VorbisFmtFields, setup_packet: &[u8]) -> Result<Codec, String> {
    let channels = i64::from(fmt.n_channels);
    let sample_rate = i64::from(fmt.n_samples_per_sec);
    let setup = parse_setup(setup_packet, channels).map_err(|error| error.to_string())?;
    if !setup.parse_complete {
        return Err("the setup packet does not parse completely".to_string());
    }
    let selection = WwiseProfile::new(WwiseVersion::DEFAULT, channels, sample_rate)
        .map_err(|error| error.to_string())?;
    let compiled = compiled_profile_for_selection(selection).map_err(|error| error.to_string())?;
    let profile = compiled
        .encoder_profile()
        .map_err(|error| error.to_string())?;
    let profile_sizes = profile.block_sizes();
    let books = load_setup_codebooks(
        &setup.book_ids,
        &compiled.book_tables().map_err(|error| error.to_string())?,
    )
    .map_err(|error| error.to_string())?;
    let carried = profile.setup_packet().map_err(|error| error.to_string())?;
    if carried != setup_packet {
        return Err("the setup packet is not the carrier's for this geometry".to_string());
    }
    let all_looks = compiled.mdct_looks().map_err(|error| error.to_string())?;
    let mut mode_looks = Vec::with_capacity(2);
    for size in profile_sizes {
        mode_looks.push(
            all_looks
                .get(&size)
                .cloned()
                .ok_or_else(|| format!("the carrier holds no MDCT look for {size}"))?,
        );
    }
    let looks: [MdctLook; 2] = mode_looks
        .try_into()
        .map_err(|_| "the carrier's block geometry is not a short/long pair".to_string())?;
    let windows = compiled
        .frozen_tables()
        .map_err(|error| error.to_string())?
        .ok_or_else(|| "the carrier holds no frozen window tables".to_string())?
        .window_halves;
    let mut ola = Vec::with_capacity(fmt.n_channels as usize);
    for _ in 0..fmt.n_channels {
        ola.push(SynthesisOla::new(&profile_sizes).map_err(|error| error.to_string())?);
    }
    Ok(Codec {
        channels: fmt.n_channels as usize,
        blocksizes: profile_sizes,
        setup,
        books,
        looks,
        windows,
        ola,
        total_frames: u64::from(fmt.dw_total_pcm_frames),
        emitted: 0,
        pending: None,
    })
}

/// One audio packet through the shipped decode-direction stage calls, in
/// `decoder.rs`'s `decode_audio_packet` order.
fn staged_audio_packet(
    codec: &Codec,
    payload: &[u8],
    index: u64,
    stages: &mut Stages,
) -> Result<Block, String> {
    let mut br = BitReader::new(payload);

    let start = Instant::now();
    let header = parse_audio_header(&mut br, &codec.setup)
        .map_err(|error| format!("packet {index}: {error}"))?;
    stages.header += start.elapsed();

    let mapping = codec
        .setup
        .maps
        .get(header.mapping as usize)
        .ok_or_else(|| format!("packet {index}: mapping {} is out of range", header.mapping))?;

    let start = Instant::now();
    let floors = decode_floors_for_packet(
        &mut br,
        &codec.setup,
        &codec.books,
        codec.channels as u32,
        mapping,
    )
    .map_err(|error| format!("packet {index}: {error}"))?;
    stages.floors += start.elapsed();

    let start = Instant::now();
    let dirty = coupling_dirty_nonzero(&floors.nonzero, &mapping.coupling)
        .map_err(|error| format!("packet {index}: {error}"))?;
    stages.coupling += start.elapsed();

    let mode = (header.blockflag & 1) as usize;
    let n_spectrum = (codec.blocksizes[mode] / 2) as usize;
    let packet_bits = (payload.len() as u64) * 8;
    let mut rows: Vec<Vec<f64>> = vec![vec![0.0f64; n_spectrum]; codec.channels];
    for submap in 0..mapping.submaps as usize {
        let bundle: Vec<usize> = (0..codec.channels)
            .filter(|&channel| submap_of(mapping, channel) == submap)
            .collect();
        if bundle.is_empty() {
            continue;
        }
        let bundle_used: Vec<bool> = bundle.iter().map(|&channel| dirty[channel]).collect();
        let residue_id = *mapping
            .residues
            .get(submap)
            .ok_or_else(|| format!("packet {index}: submap {submap} has no residue"))?;
        let residue = codec
            .setup
            .residues
            .get(residue_id as usize)
            .ok_or_else(|| format!("packet {index}: residue {residue_id} is out of range"))?;
        let start = Instant::now();
        let (bundle_rows, status) =
            decode_residue_coeffs(&mut br, residue, &codec.books, &bundle_used, n_spectrum)
                .map_err(|error| format!("packet {index}: {error}"))?;
        stages.residue += start.elapsed();
        if status == ResidueStatus::EndOfPacket {
            // The shipped rule: bits left at the stop can only be an unassigned
            // codeword, and that is a defect rather than an early end.
            let bit_position = br.tell_bits();
            if packet_bits.saturating_sub(bit_position) > 0 {
                return Err(format!(
                    "packet {index}: residue stopped at bit {bit_position} of {packet_bits} \
                     with bits remaining"
                ));
            }
        }
        for (position, &channel) in bundle.iter().enumerate() {
            rows[channel] = bundle_rows[position].clone();
        }
    }

    let start = Instant::now();
    apply_mapping_coupling_inverse(&mut rows, &mapping.coupling)
        .map_err(|error| format!("packet {index}: {error}"))?;
    stages.coupling += start.elapsed();

    let start = Instant::now();
    let mut spectra = Vec::with_capacity(codec.channels);
    #[allow(clippy::needless_range_loop)] // indexed by the mapping's mux, not by a collection
    for channel in 0..codec.channels {
        let submap = submap_of(mapping, channel);
        let floor_index = *mapping
            .floors
            .get(submap)
            .ok_or_else(|| format!("packet {index}: submap {submap} has no floor"))?;
        let floor = codec
            .setup
            .floors
            .get(floor_index as usize)
            .ok_or_else(|| format!("packet {index}: floor {floor_index} is out of range"))?;
        let mut row = std::mem::take(&mut rows[channel]);
        if let Some(posts) = floors.curves.get(channel).and_then(|curve| curve.as_ref()) {
            let postlist = postlist_from_floor(floor);
            let range = *FLOOR1_RANGES
                .get(floor.multiplier as usize)
                .ok_or_else(|| format!("packet {index}: invalid floor multiplier"))?;
            let absolute = floor1_unwrap(posts, &postlist, range as i64)
                .map_err(|error| format!("packet {index}: {error}"))?;
            let curve = floor1_curve_from_posts(&absolute, &postlist, n_spectrum, floor.multiplier)
                .map_err(|error| format!("packet {index}: {error}"))?;
            for (value, envelope) in row.iter_mut().zip(curve.iter()) {
                *value *= envelope;
            }
        }
        spectra.push(row);
    }
    stages.envelope += start.elapsed();

    Ok(Block {
        mode: mode as i64,
        spectra,
    })
}

/// Interleave whatever the overlap-add states completed past `before`, clamped
/// to the frames the container declares (`decoder.rs`'s `append_completed`).
fn append_completed(
    ola: &[SynthesisOla],
    before: &[usize],
    channels: usize,
    total_frames: u64,
    emitted: &mut u64,
    pcm: &mut Vec<f32>,
) -> Result<(), String> {
    if ola.len() != channels {
        return Err("overlap-add state count differs from the container's channels".to_string());
    }
    let mut completed = 0usize;
    for (channel, state) in ola.iter().enumerate() {
        let grew = state.pcm().len() - before[channel];
        if channel == 0 {
            completed = grew;
        } else if grew != completed {
            return Err("channel overlap-add states diverged".to_string());
        }
    }
    let room = total_frames.saturating_sub(*emitted);
    let allowed = room.min(completed as u64) as usize;
    for frame in 0..allowed {
        for (channel, state) in ola.iter().enumerate() {
            pcm.push(state.pcm()[before[channel] + frame] as f32);
        }
    }
    *emitted += allowed as u64;
    Ok(())
}

/// Push one block into every channel's overlap-add and interleave what it
/// completed (`Codec::push_block`).
fn push_block(
    codec: &mut Codec,
    block: &Block,
    following: i64,
    pcm: &mut Vec<f32>,
    stages: &mut Stages,
) -> Result<(), String> {
    let Codec {
        channels,
        looks,
        windows,
        ola,
        total_frames,
        emitted,
        ..
    } = codec;
    let look = &looks[block.mode as usize];
    let mut before = Vec::with_capacity(ola.len());
    for state in ola.iter() {
        before.push(state.pcm().len());
    }
    let start = Instant::now();
    for (channel, state) in ola.iter_mut().enumerate() {
        state
            .push(
                look,
                windows,
                block.mode,
                following,
                &block.spectra[channel],
            )
            .map_err(|error| error.to_string())?;
    }
    stages.ola += start.elapsed();
    let start = Instant::now();
    append_completed(ola, &before, *channels, *total_frames, emitted, pcm)?;
    stages.interleave += start.elapsed();
    Ok(())
}

/// What one staged replay produced, with the shipped session's own numbers for
/// the same bytes beside it.
struct StagedRun {
    stages: Stages,
    staged_total: Duration,
    one_chunk: SessionRun,
    chunked: SessionRun,
    matches_one_chunk: bool,
    matches_chunked: bool,
    header: DecodedHeader,
    frames: usize,
    declared_frames: u64,
    wem_bytes: usize,
    audio_packets: usize,
    chunk_bytes: usize,
}

/// One pass of the shipped `DecodeSession` over the same bytes.
struct SessionRun {
    push: Duration,
    finish: Duration,
    total: Duration,
    pcm: Vec<f32>,
    frames: usize,
    pcm_steps: usize,
}

/// Drive the shipped session over `wem` in `chunk_bytes`-sized pushes and
/// collect everything it delivered. `chunk_bytes == 0` is one push of the whole
/// stream.
fn session_run(wem: &[u8], chunk_bytes: usize) -> Result<SessionRun, String> {
    let mut session = DecodeSession::new();
    let mut pcm: Vec<f32> = Vec::new();
    let mut pcm_steps = 0usize;
    let mut channels = 0u32;
    let start = Instant::now();
    let chunks: Vec<&[u8]> = if chunk_bytes == 0 {
        vec![wem]
    } else {
        wem.chunks(chunk_bytes.max(1)).collect()
    };
    for bytes in chunks {
        let step = session.push_bytes(bytes);
        if step.header.is_some() {
            channels = step.header.as_ref().map_or(0, |header| header.channels);
        }
        if !step.pcm.is_empty() {
            pcm_steps += 1;
        }
        pcm.extend_from_slice(&step.pcm);
        step.outcome.map_err(|error| error.to_string())?;
    }
    let push = start.elapsed();
    let start = Instant::now();
    let step = session.finish();
    if step.header.is_some() {
        channels = step.header.as_ref().map_or(0, |header| header.channels);
    }
    if !step.pcm.is_empty() {
        pcm_steps += 1;
    }
    pcm.extend_from_slice(&step.pcm);
    step.outcome.map_err(|error| error.to_string())?;
    let finish = start.elapsed();
    let frames = if channels == 0 {
        0
    } else {
        pcm.len() / channels as usize
    };
    Ok(SessionRun {
        push,
        finish,
        total: push + finish,
        pcm,
        frames,
        pcm_steps,
    })
}

/// One full staged replay of `DecodeSession`'s call sequence, plus the shipped
/// session's own numbers for the same bytes.
///
/// The sequence is the session's, statement for statement: resolve the header
/// region, open the codec on the setup packet, then per audio packet
/// header -> floors -> residue -> coupling inverse -> floor envelope, pushing
/// block `k` once packet `k + 1` has been read and the tail at `Finish`.
fn staged_decode(wem: &[u8], chunk_bytes: usize) -> Result<StagedRun, String> {
    let mut stages = Stages::default();
    let staged_start = Instant::now();

    // --- Container framing --------------------------------------------------
    let start = Instant::now();
    if wem.len() < 12 {
        return Err("the WEM is shorter than a RIFF/WAVE header".to_string());
    }
    let (endian, chunks) = parse_chunks(wem).map_err(|error| error.to_string())?;
    let fmt_chunk = chunks
        .iter()
        .find(|chunk| chunk.id == *b"fmt " && chunk.payload.len() == chunk.size as usize)
        .ok_or_else(|| "the WEM carries no complete fmt chunk".to_string())?;
    let fmt = VorbisFmtFields::parse(&fmt_chunk.payload).map_err(|error| error.to_string())?;
    if fmt.w_format_tag != WWISE_VORBIS_FORMAT_TAG {
        return Err(format!(
            "the container's format tag is {:#06x}, not the Wwise one",
            fmt.w_format_tag
        ));
    }
    let data_chunk = chunks
        .iter()
        .find(|chunk| chunk.id == *b"data")
        .ok_or_else(|| "the WEM carries no data chunk".to_string())?;
    let data_size = data_chunk.size as usize;
    let mut seek_left = fmt.dw_seek_table_size as usize;
    if seek_left > data_size {
        return Err("the seek table does not fit the data payload".to_string());
    }
    // Everything before the data payload is framing; only the payload is
    // streamed (the session drains exactly these bytes).
    let mut position = (data_chunk.off + 8).min(wem.len());
    let mut remaining = data_size;
    stages.framing += start.elapsed();

    let mut codec: Option<Codec> = None;
    let mut pcm: Vec<f32> = Vec::new();
    let mut audio_packets = 0usize;
    let mut header: Option<DecodedHeader> = None;

    // The seek table is payload bytes the walk skips before the first packet.
    if seek_left > 0 {
        let skip = seek_left.min(wem.len().saturating_sub(position));
        position += skip;
        seek_left -= skip;
        remaining -= skip;
        if seek_left > 0 {
            return Err("the data payload ends inside its seek table".to_string());
        }
    }

    while remaining > 0 {
        let start = Instant::now();
        if wem.len().saturating_sub(position) < 2 {
            return Err("the data payload ends inside a packet size prefix".to_string());
        }
        let size = match endian {
            Endian::Little => u16::from_le_bytes([wem[position], wem[position + 1]]),
            Endian::Big => u16::from_be_bytes([wem[position], wem[position + 1]]),
        } as usize;
        if size + 2 > remaining {
            return Err("a packet does not fit the data payload".to_string());
        }
        if wem.len().saturating_sub(position) < 2 + size {
            return Err("the data payload ends inside a packet".to_string());
        }
        let payload = &wem[position + 2..position + 2 + size];
        position += 2 + size;
        remaining -= 2 + size;
        stages.framing += start.elapsed();

        if codec.is_none() {
            let start = Instant::now();
            let opened = open_codec(&fmt, payload)?;
            header = Some(DecodedHeader {
                channels: opened.channels as u32,
                sample_rate: fmt.n_samples_per_sec,
                total_frames: u64::from(fmt.dw_total_pcm_frames),
                setup_packet: payload.to_vec(),
            });
            codec = Some(opened);
            stages.setup += start.elapsed();
            continue;
        }

        let codec = codec.as_mut().expect("the codec was opened on packet 0");
        let index = audio_packets as u64;
        let block = staged_audio_packet(codec, payload, index, &mut stages)?;
        audio_packets += 1;
        // Commit: the previous block's following mode is now known.
        if let Some(previous) = codec.pending.take() {
            push_block(codec, &previous, block.mode, &mut pcm, &mut stages)?;
        }
        codec.pending = Some(block);
    }

    let codec = codec
        .as_mut()
        .ok_or_else(|| "the payload carried no setup packet".to_string())?;
    if let Some(block) = codec.pending.take() {
        push_block(codec, &block, TERMINAL_FOLLOWING, &mut pcm, &mut stages)?;
    }
    let start = Instant::now();
    for state in codec.ola.iter_mut() {
        state.finish();
    }
    stages.ola += start.elapsed();
    let start = Instant::now();
    {
        let Codec {
            channels,
            ola,
            total_frames,
            emitted,
            ..
        } = codec;
        let before: Vec<usize> = ola.iter().map(|state| state.pcm().len()).collect();
        append_completed(ola, &before, *channels, *total_frames, emitted, &mut pcm)?;
    }
    stages.interleave += start.elapsed();
    let declared_frames = codec.total_frames;
    let channels = codec.channels;
    if codec.emitted != declared_frames {
        return Err(format!(
            "the staged replay synthesized {} of the {} declared frames",
            codec.emitted, declared_frames
        ));
    }
    let staged_total = staged_start.elapsed();
    let frames = pcm.len() / channels;

    // --- The shipped session over the same bytes ----------------------------
    let one_chunk = session_run(wem, 0)?;
    let chunked = session_run(wem, chunk_bytes.max(1))?;
    let matches_one_chunk = same_samples(&pcm, &one_chunk.pcm);
    let matches_chunked = same_samples(&pcm, &chunked.pcm);

    Ok(StagedRun {
        stages,
        staged_total,
        one_chunk,
        chunked,
        matches_one_chunk,
        matches_chunked,
        header: header.ok_or_else(|| "the header was never announced".to_string())?,
        frames,
        declared_frames,
        wem_bytes: wem.len(),
        audio_packets,
        chunk_bytes,
    })
}

// ---------------------------------------------------------------------------
// Reporting
// ---------------------------------------------------------------------------

fn print_input(label: &str, run: &StagedRun) {
    println!(
        "wem_decode_perf_input corpus={label} channels={} sample_rate={} declared_frames={} \
         wem_bytes={} audio_packets={} chunk_bytes={}",
        run.header.channels,
        run.header.sample_rate,
        run.declared_frames,
        run.wem_bytes,
        run.audio_packets,
        run.chunk_bytes,
    );
}

fn print_staged(label: &str, repetition: usize, run: &StagedRun) {
    let stages = run.stages;
    let mut line = format!("wem_decode_perf corpus={label} run={repetition}");
    line += &field("framing_ms", ms(stages.framing));
    line += &field("setup_ms", ms(stages.setup));
    line += &field("header_ms", ms(stages.header));
    line += &field("floors_ms", ms(stages.floors));
    line += &field("residue_ms", ms(stages.residue));
    line += &field("coupling_ms", ms(stages.coupling));
    line += &field("envelope_ms", ms(stages.envelope));
    line += &field("ola_ms", ms(stages.ola));
    line += &field("interleave_ms", ms(stages.interleave));
    line += &field("staged_total_ms", ms(run.staged_total));
    line += &field("stages_sum_ms", ms(stages.total()));
    line += &field("session_push_ms", ms(run.one_chunk.push));
    line += &field("session_finish_ms", ms(run.one_chunk.finish));
    line += &field("session_total_ms", ms(run.one_chunk.total));
    line += &field("chunked_push_ms", ms(run.chunked.push));
    line += &field("chunked_finish_ms", ms(run.chunked.finish));
    line += &field("chunked_total_ms", ms(run.chunked.total));
    line += &format!(" frames={}", run.frames);
    line += &format!(" declared_frames={}", run.declared_frames);
    line += &format!(" audio_packets={}", run.audio_packets);
    line += &format!(" pcm_steps={}", run.one_chunk.pcm_steps);
    line += &format!(" staged_matches_session={}", run.matches_one_chunk);
    line += &format!(" chunked_matches_session={}", run.matches_chunked);
    println!("{line}");
}

/// The corpora the staged split is not run over: the geometry change is
/// confirmed by the session's own totals only, the way the encode harness'
/// 2-channel block does.
fn print_session_only(label: &str, repetition: usize, run: &SessionRun, declared_frames: u64) {
    let mut line = format!("wem_decode_perf corpus={label} run={repetition}");
    line += &field("session_push_ms", ms(run.push));
    line += &field("session_finish_ms", ms(run.finish));
    line += &field("session_total_ms", ms(run.total));
    line += &format!(" frames={}", run.frames);
    line += &format!(" declared_frames={declared_frames}");
    line += &format!(" pcm_steps={}", run.pcm_steps);
    println!("{line}");
}

/// The container's declared frame count, read from its own `fmt ` chunk.
///
/// A prefix of the file is enough — the `fmt ` chunk sits in the header region
/// — so the child can learn the contract without reading the stream twice.
fn declared_frames(wem: &[u8]) -> Result<u64, String> {
    let (_, chunks) = parse_chunks(wem).map_err(|error| error.to_string())?;
    let fmt_chunk = chunks
        .iter()
        .find(|chunk| chunk.id == *b"fmt " && chunk.payload.len() == chunk.size as usize)
        .ok_or_else(|| "no complete fmt chunk".to_string())?;
    let fmt = VorbisFmtFields::parse(&fmt_chunk.payload).map_err(|error| error.to_string())?;
    Ok(u64::from(fmt.dw_total_pcm_frames))
}

// ---------------------------------------------------------------------------
// Corpora
// ---------------------------------------------------------------------------

fn wem_files(directory: &Path) -> Vec<PathBuf> {
    let Ok(entries) = std::fs::read_dir(directory) else {
        return Vec::new();
    };
    let mut files: Vec<PathBuf> = entries
        .filter_map(|entry| entry.ok().map(|entry| entry.path()))
        .filter(|path| path.extension().is_some_and(|extension| extension == "wem"))
        .collect();
    files.sort();
    files
}

/// Read the tracked fixture and the two 2-channel corpora — the material this
/// repository commits. `corpus/` is ignored and is never read here.
fn corpora() -> Result<Vec<(String, Vec<u8>)>, String> {
    let mut material = vec![("fixture".to_string(), common::read_fixture("reference.wem"))];
    for directory in [
        repo_root().join("tests/data/2ch-reference"),
        repo_root().join("tests/data/2ch-stress"),
    ] {
        if !directory.is_dir() {
            return Err(format!("corpus directory missing: {}", directory.display()));
        }
        for path in wem_files(&directory) {
            let stem = path
                .file_stem()
                .map(|stem| stem.to_string_lossy().into_owned())
                .unwrap_or_else(|| "?".to_string());
            let bytes =
                std::fs::read(&path).map_err(|error| format!("{}: {error}", path.display()))?;
            material.push((format!("2ch/{stem}"), bytes));
        }
    }
    Ok(material)
}

/// The fixture, stage by stage. The comparison guard lives in
/// `staged_decode` and is reported per repetition and per chunking; if it ever
/// fails, every number above it describes a pipeline that is not the shipped
/// one.
fn measure_fixture(runs: usize, chunk_bytes: usize) -> Result<(), String> {
    let label = "fixture";
    let wem = common::read_fixture("reference.wem");
    let mut matched = true;
    for repetition in 0..runs {
        let run = staged_decode(&wem, chunk_bytes)?;
        if repetition == 0 {
            print_input(label, &run);
        }
        matched &= run.matches_one_chunk && run.matches_chunked;
        print_staged(label, repetition, &run);
    }
    if !matched {
        return Err(
            "the staged replay produced PCM the decode session did not; the stage split \
             would describe a different pipeline"
                .to_string(),
        );
    }
    Ok(())
}

/// The 2-channel corpora: session totals only.
fn measure_two_channel(runs: usize) -> Result<(), String> {
    for (label, wem) in corpora()? {
        if label == "fixture" {
            continue;
        }
        let declared = declared_frames(&wem)?;
        for repetition in 0..runs {
            let run = session_run(&wem, 0)?;
            if run.frames as u64 != declared {
                return Err(format!(
                    "{label}: the session delivered {} of {declared} declared frames",
                    run.frames
                ));
            }
            print_session_only(&label, repetition, &run, declared);
        }
    }
    Ok(())
}

#[test]
#[ignore = "measurement harness: scripts/measure_decode_perf.py runs it with --ignored --nocapture"]
fn decode_stage_timings_report() {
    if cfg!(debug_assertions) {
        eprintln!(
            "decode_stage_timings: skipped in a debug build; timings require \
             `cargo test --release -p wem-core --test decode_stage_timings`"
        );
        return;
    }
    let runs = env_usize("WEM_DECODE_RUNS", DEFAULT_RUNS).max(1);
    let chunk_bytes = env_usize("WEM_DECODE_CHUNK", DEFAULT_CHUNK_BYTES);
    measure_fixture(runs, chunk_bytes).expect("fixture stage measurement");
    measure_two_channel(runs).expect("2-channel corpus measurement");
}

// ---------------------------------------------------------------------------
// Child entry: one process per measurement point
// ---------------------------------------------------------------------------

/// The child's own view of the session it is driving: the counters a
/// measurement point reports, gathered without a second pass over the PCM.
struct Child {
    session: DecodeSession,
    channels: u32,
    sample_rate: u32,
    frames: u64,
    checksum: u64,
}

impl Child {
    fn new() -> Self {
        Self {
            session: DecodeSession::new(),
            channels: 0,
            sample_rate: 0,
            frames: 0,
            checksum: 0,
        }
    }

    fn observe(&mut self, step: wem_core::decoder::DecodeStep) {
        if let Some(header) = step.header.as_ref() {
            self.channels = header.channels;
            self.sample_rate = header.sample_rate;
        }
        if self.channels != 0 {
            self.frames += (step.pcm.len() / self.channels as usize) as u64;
        }
        // Strided rather than per-sample: the checksum proves real PCM came
        // out without adding a per-sample pass to the thing being measured.
        for index in (0..step.pcm.len()).step_by(4096) {
            self.checksum ^= u64::from(step.pcm[index].to_bits());
        }
        step.outcome.expect("the child's session accepts its input");
    }

    fn push(&mut self, data: &[u8]) -> Duration {
        let start = Instant::now();
        let step = self.session.push_bytes(data);
        let elapsed = start.elapsed();
        self.observe(step);
        elapsed
    }

    fn finish(&mut self) -> Duration {
        let start = Instant::now();
        let step = self.session.finish();
        let elapsed = start.elapsed();
        self.observe(step);
        elapsed
    }
}

/// The child the measurement scripts spawn, one process per point.
///
/// It decodes exactly one WEM — named by `WEM_DECODE_WEM`, pushed in
/// `WEM_DECODE_CHUNK`-sized reads (`0` = the whole file in one push) — through
/// the shipped `DecodeSession`, discards each step's PCM after folding a
/// strided checksum out of it, and prints one `wem_decode_child` line. It
/// exists so a measurement point is a process: the driver reaps it with
/// `os.wait4` and reads *that child's own* rusage, so the resident figure
/// belongs to the decode and not to the driver.
///
/// A failure is a panic (a non-zero exit), never a quiet short line: a
/// measurement of a decode that did not decode is not a measurement.
#[test]
#[ignore = "child entry: scripts/measure_decode_perf.py spawns the test binary directly, one process per point"]
fn decode_process_point() {
    if cfg!(debug_assertions) {
        eprintln!(
            "decode_process_point: skipped in a debug build; a debug decode says nothing \
             about the shipped artifact"
        );
        return;
    }
    let path = std::env::var("WEM_DECODE_WEM")
        .expect("WEM_DECODE_WEM names the WEM file this child decodes");
    let chunk_bytes = env_usize("WEM_DECODE_CHUNK", DEFAULT_CHUNK_BYTES);
    let label = std::env::var("WEM_DECODE_LABEL").unwrap_or_else(|_| {
        Path::new(&path)
            .file_stem()
            .map(|stem| stem.to_string_lossy().into_owned())
            .unwrap_or_else(|| "?".to_string())
    });

    let mut file = std::fs::File::open(&path).expect("the WEM file opens");
    // The declared frame count lives in the header region, so a prefix is
    // enough and the stream is still read exactly once.
    let mut head = vec![0u8; 64 * 1024];
    let head_len = file.read(&mut head).expect("the WEM file reads");
    let declared =
        declared_frames(&head[..head_len]).expect("the container declares its frame count");
    file.rewind().expect("the WEM file rewinds");

    let mut child = Child::new();
    let mut bytes = 0u64;
    let mut total_push = Duration::ZERO;
    if chunk_bytes == 0 {
        let whole = std::fs::read(&path).expect("the WEM file reads");
        bytes = whole.len() as u64;
        total_push += child.push(&whole);
    } else {
        let mut buffer = vec![0u8; chunk_bytes];
        loop {
            let read = file.read(&mut buffer).expect("the WEM file reads");
            if read == 0 {
                break;
            }
            bytes += read as u64;
            total_push += child.push(&buffer[..read]);
        }
    }
    let total_finish = child.finish();

    assert_eq!(
        child.frames, declared,
        "the child delivered {} of {declared} declared frames",
        child.frames
    );

    println!(
        "wem_decode_child corpus={label} bytes={bytes} chunk_bytes={chunk_bytes} \
         channels={} sample_rate={} frames={} declared_frames={declared} \
         push_ms={:.3} finish_ms={:.3} total_ms={:.3} checksum={}",
        child.channels,
        child.sample_rate,
        child.frames,
        ms(total_push),
        ms(total_finish),
        ms(total_push + total_finish),
        child.checksum,
    );
}
