//! Long and short frame psychoacoustic orchestration.
//!
//! Mirrors Python `wwise_wem/analysis/psychoacoustics/pipeline.py`. This is the
//! orchestration that ties the transform, remap, seed, and envelope stages into
//! the per-frame surfaces used by the recorded stage dump.

use crate::config::{f32_of, AnalysisError, AnalysisProfileResources, FrozenMathTables, MdctLook};

use crate::dsp::spectrum::{wwise_log_curve, wwise_mdct_log_curve};
use crate::dsp::transform::mdct_forward;
use crate::psychoacoustics::envelope::{
    make_channel_floor_envelope_scratch, shape_first_long_floor_envelope, FloorEnvelopeScratch,
};
use crate::psychoacoustics::remap::{
    build_coupling_peak, build_long_psy_remap_variant, build_psy_remap,
};
use crate::psychoacoustics::seed::{
    compute_spectrum_peak, update_frame_spectrum_peak, wwise_seed_floor_from_look,
    MaterializedLongSeedLook, SpectrumPeakState,
};
use crate::psychoacoustics::short::{ShortPsyAnalyzer, ShortPsyFrameResult};

/// Pure local surface for one fresh long-mode analysis group
/// (Python `LongPsyFrame`).
#[derive(Debug, Clone, PartialEq)]
pub struct LongPsyFrame {
    pub coefficients: Vec<Vec<f64>>,
    pub raw_mdct: Vec<Vec<f64>>,
    pub fft: Vec<Vec<f64>>,
    pub channel_specmax: Vec<f64>,
    pub global_specmax: f64,
    pub remap: Vec<Vec<f64>>,
    pub seed: Vec<Vec<f64>>,
    pub post: Vec<Vec<f64>>,
    pub side: Vec<Vec<f64>>,
    pub coupling_peak: Vec<Vec<f64>>,
    pub scratch: Vec<FloorEnvelopeScratch>,
    pub state_info: Option<crate::psychoacoustics::short::PsyFrameControls>,
}

/// One local short analysis group connected to the shared floor-envelope
/// stage owner (Python `ShortPsyStreamFrame`).
#[derive(Debug, Clone, PartialEq)]
pub struct ShortPsyStreamFrame {
    pub coefficients: Vec<Vec<f64>>,
    pub raw_mdct: Vec<Vec<f64>>,
    pub fft: Vec<Vec<f64>>,
    pub channel_specmax: Vec<f64>,
    pub global_specmax: f64,
    pub remap: Vec<Vec<f64>>,
    pub seed: Vec<Vec<f64>>,
    pub post: Vec<Vec<f64>>,
    pub side: Vec<Vec<f64>>,
    pub coupling_peak: Vec<Vec<f64>>,
    pub state_result: ShortPsyFrameResult,
}

/// Execute the local fresh long-block floor-analysis chain
/// (Python `analyze_long_frame`).
///
/// `seed_look` is the materialized, frame- and channel-invariant long seed
/// look (`MaterializedLongSeedLook`); it must have been materialized from
/// `resources.long_base`, which is what the session does once per encode. The
/// frame-invariant derivation it carries is otherwise rebuilt identically once
/// per channel per frame (see `build_long_floor_seed_from_look`).
#[allow(clippy::too_many_arguments)]
pub fn analyze_long_frame(
    windowed_frames: &[Vec<f64>],
    carried_global_specmax: f64,
    resources: &AnalysisProfileResources,
    seed_look: &MaterializedLongSeedLook,
    scratch: Option<&mut Vec<FloorEnvelopeScratch>>,
    stream: Option<&mut ShortPsyAnalyzer>,
    long_variant: i64,
    following_mode: i64,
    specmax_state: Option<&mut SpectrumPeakState>,
) -> Result<LongPsyFrame, AnalysisError> {
    use AnalysisError::*;
    if windowed_frames.is_empty() {
        return Err(LongAnalysisEmpty);
    }
    if !(0..=1).contains(&long_variant) {
        return Err(LongFrameVariantInvalid {
            variant: long_variant,
        });
    }
    let table = &resources.long_base;
    if table.n != 1024 {
        return Err(LongAnalysisTableBins { want: 1024 });
    }
    if windowed_frames
        .iter()
        .any(|frame| frame.len() as i64 != table.n * 2)
    {
        return Err(LongAnalysisFrameSize { want: table.n * 2 });
    }
    if let Some(scratch) = &scratch {
        if scratch.len() as i64 != windowed_frames.len() as i64 {
            return Err(LongAnalysisScratchCount {
                want: windowed_frames.len() as i64,
            });
        }
    }
    if stream.is_some() {
        if scratch.is_some() {
            return Err(LongAnalysisBothScratchAndStream);
        }
        if let Some(stream) = &stream {
            if stream.channels.len() as i64 != windowed_frames.len() as i64 {
                return Err(LongAnalysisStreamChannels {
                    want: windowed_frames.len() as i64,
                });
            }
        }
    }

    // Two scheduler variants are distinct psycho looks (mode 2 vs 3).
    let analysis_mode = 2 + long_variant;
    let analysis_table =
        resources
            .long_variants
            .get(&analysis_mode)
            .ok_or(IncompleteResources {
                reason: "analysis resources lack psychoacoustic variants",
            })?;
    let mdct_look = resources
        .mdct_looks
        .get(&(table.n * 2))
        .ok_or(IncompleteResources {
            reason: "analysis resources lack required MDCT looks",
        })?;
    if mdct_look.n != table.n * 2 {
        return Err(LongMdctLookGeometry { want: table.n * 2 });
    }
    let frozen = resources.frozen_twiddles()?;
    // SAFETY (per-channel partition, `parallel` feature): each spawned job
    // owns exactly one channel index. It reads only that channel's input
    // slice plus immutable shared resources (mdct_look, frozen twiddles,
    // resource tables) and writes only its own slot of the result vector,
    // which `iter_mut` handed it as a distinct `&mut` — so there is no
    // cross-channel shared mutable state, no interior mutability and no
    // lock, `f32_of` is a pure, order-free rounding, and slots are collected
    // in index order, so the results are bit-identical to the sequential
    // loop. `scope` joins every job it spawned before returning, so a slot
    // is always published by the time it is read, at any pool size.
    // Guarded end-to-end by the complete_wem_bytes / frame_pipeline_parity /
    // vorbis_oracle_values byte-parity tests.
    //
    // Without `parallel` (threadless targets such as wasm32), run the same
    // work sequentially.
    #[cfg_attr(not(feature = "parallel"), allow(unused_mut))]
    let transformed: Vec<(Vec<f64>, Vec<f64>, Vec<f64>)> =
        transform_channel_rows(windowed_frames, mdct_look, frozen)?;

    let mut coefficients: Vec<Vec<f64>> = Vec::with_capacity(transformed.len());
    let mut raw_mdct: Vec<Vec<f64>> = Vec::with_capacity(transformed.len());
    let mut fft: Vec<Vec<f64>> = Vec::with_capacity(transformed.len());
    for (coeffs, raw, spectrum) in transformed {
        coefficients.push(coeffs);
        raw_mdct.push(raw);
        fft.push(spectrum);
    }

    let (channel_specmax, global_specmax) = match specmax_state {
        None => compute_spectrum_peak(&fft, carried_global_specmax)?,
        Some(state) => update_frame_spectrum_peak(&fft, table.n, table.sample_rate, state)?,
    };

    // Resolve per-channel scratch from the stream when present (single
    // direct clone; an earlier intermediate Vec doubled copy traffic).

    let long_floor_envelope =
        resources
            .long_floor_looks
            .get(&analysis_mode)
            .ok_or(IncompleteResources {
                reason: "analysis resources lack long floor looks",
            })?;

    // Snapshot the scratch contents once to avoid moving the Option per iteration.
    let scratch_snapshot: Option<Vec<FloorEnvelopeScratch>> = scratch.as_ref().map(|v| v.to_vec());

    // Resolve per-channel scratch up front (single direct clone each), so
    // the parallel region below needs no shared mutable state.
    let local_scratches: Vec<FloorEnvelopeScratch> = (0..raw_mdct.len())
        .map(|ci| {
            stream
                .as_ref()
                .map(|stream| {
                    let channel = &stream.channels[ci];
                    FloorEnvelopeScratch {
                        current_curve: channel.state.clone(),
                        history_curve: channel.history.clone(),
                    }
                })
                .or_else(|| scratch_snapshot.as_ref().map(|s| s[ci].clone()))
                .unwrap_or_else(make_channel_floor_envelope_scratch)
        })
        .collect();

    let mut remap: Vec<Vec<f64>> = Vec::new();
    let mut seed: Vec<Vec<f64>> = Vec::new();
    let mut post: Vec<Vec<f64>> = Vec::new();
    let mut side: Vec<Vec<f64>> = Vec::new();
    let mut coupling_peak: Vec<Vec<f64>> = Vec::new();
    let mut scratches: Vec<FloorEnvelopeScratch> = Vec::new();
    let coupling_tone_end = table
        .seed_outer_u32
        .get(17)
        .copied()
        .ok_or(IncompleteResources {
            reason: "long psychoacoustic table lacks coupling tone limit",
        })? as usize;

    // SAFETY (per-channel partition, `parallel` feature): same argument as
    // the transform region above — one spawned job per channel, each reading
    // only its own channel's rows (plus the specmax values computed before
    // this region, which are immutable inputs here) and writing only the
    // slot it was handed. Scratch copies are per-channel owned. The
    // materialized seed look is shared read-only across the jobs: a plain
    // `&` with no lock and no interior mutability, since `build_seed` only
    // reads it.
    type PsychChannel = (
        Vec<f64>,
        Vec<f64>,
        Vec<f64>,
        Vec<f64>,
        Vec<f64>,
        FloorEnvelopeScratch,
    );
    let channel_count = raw_mdct.len();
    let psych: Vec<PsychChannel> = channel_psych_map(channel_count, |channel_index| {
        let raw = &raw_mdct[channel_index];
        let logfft = &fft[channel_index];
        let specmax = channel_specmax[channel_index];
        let coeff = &coefficients[channel_index];

        let remap_result = build_long_psy_remap_variant(raw, analysis_mode, analysis_table)?;
        let local_remap = remap_result.remap.clone();
        let local_coupling_peak = build_coupling_peak(
            raw,
            &remap_result.selector,
            &remap_result.base,
            &local_scratches[channel_index].current_curve,
            coupling_tone_end,
            windowed_frames.len() == 2,
        )?;
        let local_seed = seed_look.build_seed(logfft, specmax, global_specmax)?;

        let mut local_scratch = local_scratches[channel_index].clone();
        let local_post_side = shape_first_long_floor_envelope(
            &local_seed,
            &local_remap,
            raw,
            coeff,
            &mut local_scratch,
            -1.0,
            Some(long_floor_envelope),
            None,
            2,
            1,
        )?;

        Ok::<(_, _, _, _, _, _), AnalysisError>((
            local_remap,
            local_seed,
            local_post_side.0,
            local_post_side.1,
            local_coupling_peak,
            local_scratch,
        ))
    })?;

    for (r, s, p, sd, cp, sc) in psych {
        remap.push(r);
        seed.push(s);
        post.push(p);
        side.push(sd);
        coupling_peak.push(cp);
        scratches.push(sc);
    }

    // Write back scratch mutations into the stream channels, then commit state.
    let mut state_info = None;
    if let Some(stream) = stream {
        // Reflect the inactive long branch's state=raw write.
        for (i, scratch_item) in scratches.iter().enumerate() {
            stream.channels[i].state = scratch_item.current_curve.clone();
            stream.channels[i].history = scratch_item.history_curve.clone();
        }
        state_info = Some(
            stream
                .commit_long_state(&raw_mdct, long_variant, following_mode, 0)
                .map_err(|_| LongAnalysisStreamChannels {
                    want: windowed_frames.len() as i64,
                })?,
        );
    }

    Ok(LongPsyFrame {
        coefficients,
        raw_mdct,
        fft,
        channel_specmax,
        global_specmax,
        remap,
        seed,
        post,
        side,
        coupling_peak,
        scratch: scratches,
        state_info,
    })
}

/// Run one local short frame through the persistent active floor-envelope
/// stage path (Python `analyze_short_frame`).
#[allow(clippy::too_many_arguments)]
pub fn analyze_short_frame(
    windowed_frames: &[Vec<f64>],
    stream: &mut ShortPsyAnalyzer,
    resources: &AnalysisProfileResources,
    short_variant: i64,
    following_mode: i64,
    q: f64,
    hold_update: i64,
    carried_global_specmax: f64,
    specmax_state: Option<&mut SpectrumPeakState>,
    groups: Option<&[Vec<f64>]>,
) -> Result<ShortPsyStreamFrame, AnalysisError> {
    use AnalysisError::*;
    if windowed_frames.is_empty() {
        return Err(ShortAnalysisEmpty);
    }
    if windowed_frames.len() as i64 != stream.channels.len() as i64 {
        return Err(ShortAnalysisChannelCount {
            want: stream.channels.len() as i64,
        });
    }
    if windowed_frames.iter().any(|frame| frame.len() != 256) {
        return Err(ShortAnalysisFrameSize { want: 256 });
    }
    if let Some(groups) = groups {
        if groups.len() as i64 != windowed_frames.len() as i64 {
            return Err(ShortAnalysisGroupWorkCount {
                want: windowed_frames.len() as i64,
            });
        }
    }

    let mdct_look = resources.mdct_looks.get(&256).ok_or(IncompleteResources {
        reason: "analysis resources lack required MDCT looks",
    })?;
    if mdct_look.n != 256 {
        return Err(IncompleteResources {
            reason: "short MDCT look differs from analysis geometry",
        });
    }
    // Short frames: the 256-sample per-channel work is too small for
    // rayon's per-region sync overhead to pay off, so this stays
    // sequential (the long-frame path above does use per-channel
    // partitioning).
    let frozen = resources.frozen_twiddles()?;
    let mut coefficients: Vec<Vec<f64>> = Vec::with_capacity(windowed_frames.len());
    let mut raw_mdct: Vec<Vec<f64>> = Vec::with_capacity(windowed_frames.len());
    let mut fft: Vec<Vec<f64>> = Vec::with_capacity(windowed_frames.len());
    for frame in windowed_frames {
        let f32_frame: Vec<f64> = frame.iter().map(|value| f32_of(*value)).collect();
        let coeffs = mdct_forward(mdct_look, &f32_frame)?;
        let raw = wwise_mdct_log_curve(&coeffs);
        let spectrum = wwise_log_curve(&f32_frame, frozen)?;
        coefficients.push(coeffs);
        raw_mdct.push(raw);
        fft.push(spectrum);
    }
    // Frame-level sequential state update (cross-frame specmax must keep
    // its exact order; it reads only the completed fft rows).
    let (channel_specmax, global_specmax) = match specmax_state {
        None => compute_spectrum_peak(&fft, carried_global_specmax)?,
        // The short look's own rate, not a literal: this frame's decay feeds the
        // persistent cross-frame peak, so a hardcoded rate silently corrupts
        // every later frame (and breaks parity with the reference port, which
        // reads ``resources.short_look.sample_rate`` here).
        Some(state) => {
            update_frame_spectrum_peak(&fft, 128, resources.short_look.sample_rate, state)?
        }
    };
    let look = &resources.short_look;
    // The two transient variants share tone curves but have distinct peak caps.
    let cap_curve = &resources.short_profiles[short_variant as usize].mask_curves[1];
    let remap: Vec<Vec<f64>> = raw_mdct
        .iter()
        .map(|raw| {
            build_psy_remap(
                raw,
                q,
                look,
                cap_curve,
                &resources.short_surface.remap_curve_offsets,
            )
            .map(|(_, _, _, noise_mask, _)| noise_mask)
        })
        .collect::<Result<Vec<Vec<f64>>, AnalysisError>>()?;
    let seed: Vec<Vec<f64>> = fft
        .iter()
        .zip(channel_specmax.iter())
        .map(|(logfft, channel_max)| {
            wwise_seed_floor_from_look(look, logfft, *channel_max, global_specmax)
        })
        .collect::<Result<Vec<Vec<f64>>, AnalysisError>>()?;
    let state_result = stream.process_frame(
        &remap,
        &seed,
        &coefficients,
        &raw_mdct,
        short_variant,
        following_mode,
        q,
        hold_update,
        groups,
    )?;

    let post: Vec<Vec<f64>> = state_result
        .channels
        .iter()
        .map(|row| row.post.clone())
        .collect();
    let side: Vec<Vec<f64>> = state_result
        .channels
        .iter()
        .map(|row| row.side.clone())
        .collect();

    Ok(ShortPsyStreamFrame {
        coefficients,
        raw_mdct,
        fft,
        channel_specmax,
        global_specmax,
        remap,
        seed,
        post,
        side,
        coupling_peak: vec![vec![0.0; 128]; windowed_frames.len()],
        state_result,
    })
}

/// Per-channel long-frame transform result: (MDCT coefficients, raw log
/// curve, FFT log curve).
type LongChannelTransform = (Vec<f64>, Vec<f64>, Vec<f64>);

/// Compute per-channel (MDCT coefficients, raw log curve, FFT log curve)
/// for a long-frame window. Each channel is an independent transformation of
/// its own slice plus immutable shared looks, so the `parallel` feature may
/// run channels concurrently without changing any bit of output; without it
/// the same work runs sequentially (threadless targets).
///
/// `parallel` runs one explicitly spawned job per channel inside a single
/// scope rather than an adaptive `par_iter` range. The difference is
/// scheduling only — the same jobs, the same order of results, the same
/// arithmetic — but a range over one job per channel is handed out by
/// recursive halving on demand, so a region whose whole point is six jobs of
/// ~60us reaches six workers only through a chain of steals. Spawning the six
/// jobs up front makes every one of them visible to the pool at once; the
/// scope then joins them all before returning, so the collected vector is
/// complete regardless of how many threads the pool has.
fn transform_channel_rows(
    windowed_frames: &[Vec<f64>],
    mdct_look: &MdctLook,
    frozen: &FrozenMathTables,
) -> Result<Vec<LongChannelTransform>, AnalysisError> {
    #[cfg(feature = "parallel")]
    {
        let mut rows: Vec<Option<Result<LongChannelTransform, AnalysisError>>> =
            windowed_frames.iter().map(|_| None).collect();
        rayon::scope(|scope| {
            for (frame, row) in windowed_frames.iter().zip(rows.iter_mut()) {
                scope.spawn(move |_| {
                    *row = Some(transform_one_channel(frame, mdct_look, frozen));
                });
            }
        });
        rows.into_iter()
            .map(|row| row.expect("every spawned channel job publishes before the scope returns"))
            .collect()
    }
    #[cfg(not(feature = "parallel"))]
    {
        (0..windowed_frames.len())
            .map(|ci| transform_one_channel(&windowed_frames[ci], mdct_look, frozen))
            .collect()
    }
}

fn transform_one_channel(
    frame: &[f64],
    mdct_look: &MdctLook,
    frozen: &FrozenMathTables,
) -> Result<LongChannelTransform, AnalysisError> {
    // Single f32-rounded copy reused by MDCT and FFT; rounding is idempotent,
    // so this is bit-identical to rounding twice.
    let f32_frame: Vec<f64> = frame.iter().map(|value| f32_of(*value)).collect();
    let coeffs = mdct_forward(mdct_look, &f32_frame)?;
    let raw = wwise_mdct_log_curve(&coeffs);
    let spectrum = wwise_log_curve(&f32_frame, frozen)?;
    Ok((coeffs, raw, spectrum))
}

/// Map a per-channel psych closure over `count` channels, using rayon when
/// the `parallel` feature is on and a sequential loop otherwise. Output
/// order is preserved either way.
///
/// The `parallel` branch spawns one job per channel into one scope and
/// collects by slot index, exactly as `transform_channel_rows` does — see
/// there for why an explicit job per channel beats an adaptive range here.
/// The closure is shared, not cloned: it captures the frame's already
/// computed rows by reference, and `F: Sync` is what makes that sound.
fn channel_psych_map<T, F>(count: usize, f: F) -> Result<Vec<T>, AnalysisError>
where
    F: Fn(usize) -> Result<T, AnalysisError> + Send + Sync,
    T: Send,
{
    #[cfg(feature = "parallel")]
    {
        let f = &f;
        let mut channels: Vec<Option<Result<T, AnalysisError>>> =
            (0..count).map(|_| None).collect();
        rayon::scope(|scope| {
            for (channel_index, channel) in channels.iter_mut().enumerate() {
                scope.spawn(move |_| {
                    *channel = Some(f(channel_index));
                });
            }
        });
        channels
            .into_iter()
            .map(|channel| {
                channel.expect("every spawned channel job publishes before the scope returns")
            })
            .collect()
    }
    #[cfg(not(feature = "parallel"))]
    {
        (0..count).map(f).collect::<Result<Vec<_>, _>>()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn f32_of_roundtrip_is_identity_for_representable() {
        assert_eq!(f32_of(1.5), 1.5);
        assert_eq!(f32_of(0.0), 0.0);
    }
}
