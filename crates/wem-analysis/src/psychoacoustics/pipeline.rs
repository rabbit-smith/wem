//! Long and short frame psychoacoustic orchestration.
//!
//! Mirrors Python `wwise_wem/analysis/psychoacoustics/pipeline.py`. This is the
//! orchestration that ties the transform, remap, seed, and envelope stages into
//! the per-frame surfaces used by the golden dump.

use crate::config::{AnalysisError, AnalysisProfileResources};
use crate::dsp::spectrum::{wwise_log_curve, wwise_mdct_log_curve};
use crate::dsp::transform::mdct_forward;
use crate::psychoacoustics::envelope::{
    make_channel_floor_envelope_scratch, shape_first_long_floor_envelope, FloorEnvelopeScratch,
};
use crate::psychoacoustics::remap::{build_long_psy_remap_variant, build_psy_remap};
use crate::psychoacoustics::seed::{
    build_long_floor_seed, compute_spectrum_peak, update_frame_spectrum_peak,
    wwise_seed_floor_from_look, SpectrumPeakState,
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
    pub state_result: ShortPsyFrameResult,
}

/// Execute the local fresh long-block floor-analysis chain
/// (Python `analyze_long_frame`).
#[allow(clippy::too_many_arguments)]
pub fn analyze_long_frame(
    windowed_frames: &[Vec<f64>],
    carried_global_specmax: f64,
    resources: &AnalysisProfileResources,
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
    if windowed_frames.iter().any(|frame| frame.len() as i64 != table.n * 2) {
        return Err(LongAnalysisFrameSize {
            want: table.n * 2,
        });
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
    let analysis_table = resources
        .long_variants
        .get(&analysis_mode)
        .ok_or(IncompleteResources {
            reason: "analysis resources lack psychoacoustic variants",
        })?;
    let mdct_look = resources.mdct_looks.get(&(table.n * 2)).ok_or(IncompleteResources {
        reason: "analysis resources lack required MDCT looks",
    })?;
    if mdct_look.n != table.n * 2 {
        return Err(LongMdctLookGeometry { want: table.n * 2 });
    }
    let coefficients: Vec<Vec<f64>> = windowed_frames
        .iter()
        .map(|frame| {
            let f32_frame: Vec<f64> = frame.iter().map(|value| f32_of_local(*value)).collect();
            mdct_forward(mdct_look, &f32_frame)
        })
        .collect::<Result<Vec<Vec<f64>>, AnalysisError>>()?;
    let raw_mdct: Vec<Vec<f64>> = coefficients.iter().map(|c| wwise_mdct_log_curve(c)).collect();
    let frozen = resources.frozen_twiddles()?;
    let fft: Vec<Vec<f64>> = windowed_frames
        .iter()
        .map(|frame| {
            let f32_frame: Vec<f64> = frame.iter().map(|value| f32_of_local(*value)).collect();
            wwise_log_curve(&f32_frame, frozen)
        })
        .collect::<Result<Vec<Vec<f64>>, AnalysisError>>()?;

    let (channel_specmax, global_specmax) = match specmax_state {
        None => compute_spectrum_peak(&fft, carried_global_specmax)?,
        Some(state) => {
            update_frame_spectrum_peak(&fft, table.n, table.sample_rate, state)?
        }
    };

    // Resolve shared scratch from stream if provided.
    let mut stream_scratch: Vec<FloorEnvelopeScratch> = Vec::new();
    if let Some(stream) = &stream {
        for channel in &stream.channels {
            stream_scratch.push(FloorEnvelopeScratch {
                current_curve: channel.state.clone(),
                history_curve: channel.history.clone(),
            });
        }
    }

    let long_floor_envelope = resources.long_floor_looks.get(&analysis_mode).ok_or(
        IncompleteResources {
            reason: "analysis resources lack long floor looks",
        },
    )?;

    // Snapshot the scratch contents once to avoid moving the Option per iteration.
    let scratch_snapshot: Option<Vec<FloorEnvelopeScratch>> =
        scratch.as_ref().map(|v| v.to_vec());

    let mut remap: Vec<Vec<f64>> = Vec::new();
    let mut seed: Vec<Vec<f64>> = Vec::new();
    let mut post: Vec<Vec<f64>> = Vec::new();
    let mut side: Vec<Vec<f64>> = Vec::new();
    let mut scratches: Vec<FloorEnvelopeScratch> = Vec::new();

    for channel_index in 0..raw_mdct.len() {
        let raw = &raw_mdct[channel_index];
        let logfft = &fft[channel_index];
        let specmax = channel_specmax[channel_index];
        let coeff = &coefficients[channel_index];

        let local_remap = build_long_psy_remap_variant(raw, analysis_mode, analysis_table)?.remap;
        let local_seed = build_long_floor_seed(
            table,
            logfft,
            specmax,
            global_specmax,
        )?;

        let local_scratch = if !stream_scratch.is_empty() {
            stream_scratch[channel_index].clone()
        } else if let Some(ref s) = scratch_snapshot {
            s[channel_index].clone()
        } else {
            make_channel_floor_envelope_scratch()
        };
        let mut local_scratch = local_scratch;
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

        remap.push(local_remap);
        seed.push(local_seed);
        post.push(local_post_side.0);
        side.push(local_post_side.1);
        scratches.push(local_scratch);
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
                .commit_long_state(
                    &raw_mdct,
                    long_variant,
                    following_mode,
                    0,
                )
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
    update_gate: i64,
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
    let coefficients: Vec<Vec<f64>> = windowed_frames
        .iter()
        .map(|frame| {
            let f32_frame: Vec<f64> = frame.iter().map(|value| f32_of_local(*value)).collect();
            mdct_forward(mdct_look, &f32_frame)
        })
        .collect::<Result<Vec<Vec<f64>>, AnalysisError>>()?;
    let raw_mdct: Vec<Vec<f64>> = coefficients.iter().map(|c| wwise_mdct_log_curve(c)).collect();
    let frozen = resources.frozen_twiddles()?;
    let fft: Vec<Vec<f64>> = windowed_frames
        .iter()
        .map(|frame| {
            let f32_frame: Vec<f64> = frame.iter().map(|value| f32_of_local(*value)).collect();
            wwise_log_curve(&f32_frame, frozen)
        })
        .collect::<Result<Vec<Vec<f64>>, AnalysisError>>()?;

    let (channel_specmax, global_specmax) = match specmax_state {
        None => compute_spectrum_peak(&fft, carried_global_specmax)?,
        Some(state) => {
            update_frame_spectrum_peak(&fft, 128, 44100, state)?
        }
    };

    let look = &resources.short_look;
    let remap: Vec<Vec<f64>> = raw_mdct
        .iter()
        .map(|raw| {
            build_psy_remap(
                raw,
                q,
                look,
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
        update_gate,
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
        state_result,
    })
}

/// Round at the algorithm's float32 storage boundary.
fn f32_of_local(value: f64) -> f64 {
    crate::config::f32_of(value)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn f32_of_roundtrip_is_identity_for_representable() {
        assert_eq!(f32_of_local(1.5), 1.5);
        assert_eq!(f32_of_local(0.0), 0.0);
    }
}
