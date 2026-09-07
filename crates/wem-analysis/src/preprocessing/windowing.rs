//! Materialize hybrid-windowed PCM frames from immutable frame decisions.
//!
//! Mirrors Python `wwise_wem/analysis/preprocessing/windowing.py`.

use std::collections::HashMap;

use wem_scheduling::{plan_mode_sequence, validate_modes, FramePlan};

use crate::config::AnalysisError;
use crate::dsp::lpc::{wwise_first_frame_lpc_prime, wwise_lpc_from_data, wwise_lpc_predict};
use crate::config::f32_of;
use crate::dsp::transform::apply_vorbis_window;

/// One scheduler-planned, channel-major PCM window (Python `WindowedFrame`).
///
/// Frame identity and mode transitions are properties of `plan`; the
/// immutable scheduler decision is the only source of scheduling truth.
#[derive(Debug, Clone, PartialEq)]
pub struct WindowedFrame {
    pub plan: FramePlan,
    pub center: i64,
    /// Channel-major windowed samples: `samples[channel][sample]`.
    pub samples: Vec<Vec<f64>>,
}

impl WindowedFrame {
    pub fn index(&self) -> i64 {
        self.plan.index
    }
    pub fn previous(&self) -> i64 {
        self.plan.previous
    }
    pub fn current(&self) -> i64 {
        self.plan.current
    }
    pub fn following(&self) -> i64 {
        self.plan.following
    }
}

/// Materialize PCM exclusively from scheduler-owned frame plans
/// (Python `iter_planned_pcm_windows`), returning all frames.
pub fn iter_planned_pcm_windows(
    pcm: &[Vec<f64>],
    plans: &[FramePlan],
    blocksizes: &[i64],
    frozen_windows: Option<&HashMap<i64, Vec<f32>>>,
) -> Result<Vec<WindowedFrame>, AnalysisError> {
    if plans.is_empty() {
        return Ok(Vec::new());
    }
    for (index, plan) in plans.iter().enumerate() {
        validate_modes(
            blocksizes,
            &[plan.previous, plan.current, plan.following],
        )
        .map_err(|_| AnalysisError::FramePlansNotContiguous)?;
        if plan.index != index as i64 {
            return Err(AnalysisError::FramePlansNotContiguous);
        }
        let expected_size = blocksizes[plan.current as usize];
        if plan.sample_end - plan.sample_start != expected_size {
            return Err(AnalysisError::FramePlanIntervalMismatch);
        }
        if index > 0 {
            let prev = &plans[index - 1];
            if prev.current != plan.previous || prev.following != plan.current {
                return Err(AnalysisError::FramePlanTransitionsDiffer);
            }
        }
    }
    if pcm.is_empty() {
        return Err(AnalysisError::PcmFeederEmpty);
    }
    let source_len = pcm[0].len() as i64;
    if source_len < 4096 {
        return Err(AnalysisError::PcmFeederShort { frames: source_len });
    }
    if pcm
        .iter()
        .any(|channel| channel.len() as i64 != source_len)
    {
        return Err(AnalysisError::PcmChannelsUnequal {
            want: source_len,
            got: 0,
        });
    }

    // LPC priming before packet zero.
    let channels: Vec<Vec<f64>> =
        pcm.iter()
            .map(|channel| channel.iter().map(|v| f32_of(*v)).collect())
            .collect();
    let primes: Vec<Vec<f64>> = channels
        .iter()
        .map(|channel| {
            wwise_first_frame_lpc_prime(channel, 128, 4096, 16)
                .expect("first-frame LPC prime (validated above)")
        })
        .collect();

    // EOF uses the regular-direction 32-tap predictor.
    let tail_count = blocksizes.iter().copied().max().unwrap_or(0);
    let tails: Vec<Vec<f64>> = channels
        .iter()
        .map(|channel| {
            let len = channel.len();
            let coeffs = wwise_lpc_from_data(&channel[len - 4096..], 32)
                .expect("tail LPC coefficients (validated above)");
            wwise_lpc_predict(&coeffs, &channel[len - 32..], tail_count)
                .expect("tail LPC prediction (validated above)")
        })
        .collect();

    let source_origin = blocksizes[1] / 2;
    let mut frames = Vec::with_capacity(plans.len());
    for plan in plans {
        let current = plan.current;
        let previous = plan.previous;
        let following = plan.following;
        let n = blocksizes[current as usize];
        let start = plan.sample_start - source_origin;
        let center = start + n / 2;
        let mut rows: Vec<Vec<f64>> = Vec::with_capacity(channels.len());
        for channel_idx in 0..channels.len() {
            let channel = &channels[channel_idx];
            let prime = &primes[channel_idx];
            let tail = &tails[channel_idx];
            let mut raw: Vec<f64> = Vec::with_capacity(n as usize);
            for sample_index in start..(start + n) {
                let value = if sample_index < 0 {
                    let prime_index = sample_index + (prime.len() as i64);
                    if prime_index >= 0 {
                        prime[prime_index as usize]
                    } else {
                        0.0
                    }
                } else if sample_index < source_len {
                    channel[sample_index as usize]
                } else {
                    let tail_index = sample_index - source_len;
                    if tail_index < (tail.len() as i64) {
                        tail[tail_index as usize]
                    } else {
                        0.0
                    }
                };
                raw.push(value);
            }
            // Short blocks do not use pending long-transition flags; their
            // effective window is always the short one.
            let window_modes = if current == 0 {
                (0, 0, 0)
            } else {
                (previous, current, following)
            };
            let windowed = apply_vorbis_window(
                &raw,
                blocksizes,
                window_modes.0,
                window_modes.1,
                window_modes.2,
                frozen_windows,
            )?;
            rows.push(windowed);
        }
        frames.push(WindowedFrame {
            plan: *plan,
            center,
            samples: rows,
        });
    }
    Ok(frames)
}

/// Yield hybrid-windowed PCM blocks for an already-decided mode stream
/// (Python `iter_pcm_windows`).
pub fn iter_pcm_windows(
    pcm: &[Vec<f64>],
    modes: &[i64],
    blocksizes: &[i64],
    terminal_following: i64,
    frozen_windows: Option<&HashMap<i64, Vec<f32>>>,
) -> Result<Vec<WindowedFrame>, AnalysisError> {
    let plans = plan_mode_sequence(modes, blocksizes, terminal_following)
        .map_err(|e| match e {
            wem_scheduling::PlannerError::BlockSizeCount => AnalysisError::WindowBlockSizeInvalid,
            _ => AnalysisError::FramePlanIntervalMismatch,
        })?;
    iter_planned_pcm_windows(pcm, &plans, blocksizes, frozen_windows)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rejects_empty_plans_and_pcm() {
        assert!(iter_planned_pcm_windows(&[], &[], &[256, 2048], None).is_ok());
        let plans =
            wem_scheduling::plan_mode_sequence(&[0], &[256, 2048], 1).expect("plan");
        assert!(iter_planned_pcm_windows(&[], &plans, &[256, 2048], None).is_err());
    }

    #[test]
    fn rejects_short_pcm() {
        let pcm: Vec<Vec<f64>> = vec![vec![0.0f64; 100]];
        let plans =
            wem_scheduling::plan_mode_sequence(&[0], &[256, 2048], 1).expect("plan");
        assert!(iter_planned_pcm_windows(&pcm, &plans, &[256, 2048], None).is_err());
    }
}
