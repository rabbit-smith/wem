//! Materialize hybrid-windowed PCM frames from immutable frame decisions.
//!
//! Mirrors Python `wwise_wem/analysis/preprocessing/windowing.py`.

use std::collections::HashMap;

use wem_scheduling::{plan_mode_sequence, validate_modes, FramePlan};

use crate::config::f32_of;
use crate::config::AnalysisError;
use crate::dsp::lpc::{wwise_first_frame_lpc_prime, wwise_lpc_from_data, wwise_lpc_predict};
use crate::dsp::transform::apply_vorbis_window_in_place;

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
///
/// Collecting form: it is what a caller that wants the whole sequence in
/// hand asks for. A conversion uses [`PlannedWindowSource::materialize`]
/// directly and holds one frame at a time.
pub fn iter_planned_pcm_windows(
    pcm: &[Vec<f64>],
    plans: &[FramePlan],
    blocksizes: &[i64],
    frozen_windows: Option<&HashMap<i64, Vec<f32>>>,
    tail_training: Option<i64>,
) -> Result<Vec<WindowedFrame>, AnalysisError> {
    let source = PlannedWindowSource::new(pcm, plans, blocksizes, tail_training)?;
    source
        .plans()
        .iter()
        .map(|plan| source.materialize(plan, frozen_windows, None))
        .collect()
}

/// One conversion's complete analysis input, handing out one windowed
/// frame at a time into caller-owned scratch.
///
/// This is the batch counterpart of
/// [`StreamingPcmFeeder`](crate::preprocessing::streaming::StreamingPcmFeeder):
/// both own the PCM-derived state one analysis session reads — this one
/// the complete source the one-shot caller already holds, the streaming
/// feeder a bounded ring — and both materialize exactly one frame per
/// request. Nothing here holds a frame sequence: a frame's rows belong to
/// the caller and are refilled in place, so peak frame storage is one
/// frame regardless of stream length (the crate rule: no `Vec` returns
/// from an inner stage where a scratch buffer exists).
pub struct PlannedWindowSource {
    plans: Vec<FramePlan>,
    blocksizes: Vec<i64>,
    /// f32-rounded source rows: the LPC priming/tail model input, and the
    /// values every frame's raw row is gathered from.
    channels: Vec<Vec<f64>>,
    primes: Vec<Vec<f64>>,
    tails: Vec<Vec<f64>>,
    source_len: i64,
    source_origin: i64,
}

impl PlannedWindowSource {
    /// Validate the plans and the source PCM, and derive the priming/tail
    /// rows every frame reads. These are the checks, in the order, the
    /// batch materializer has always made; every rejection names the same
    /// error it always did.
    pub fn new(
        pcm: &[Vec<f64>],
        plans: &[FramePlan],
        blocksizes: &[i64],
        tail_training: Option<i64>,
    ) -> Result<Self, AnalysisError> {
        if plans.is_empty() {
            return Ok(Self {
                plans: Vec::new(),
                blocksizes: blocksizes.to_vec(),
                channels: Vec::new(),
                primes: Vec::new(),
                tails: Vec::new(),
                source_len: 0,
                source_origin: 0,
            });
        }
        for (index, plan) in plans.iter().enumerate() {
            validate_modes(blocksizes, &[plan.previous, plan.current, plan.following])
                .map_err(|_| AnalysisError::state("frame plans not contiguous"))?;
            if plan.index != index as i64 {
                return Err(AnalysisError::state("frame plans not contiguous"));
            }
            let expected_size = blocksizes[plan.current as usize];
            if plan.sample_end - plan.sample_start != expected_size {
                return Err(AnalysisError::state("frame plan interval mismatch"));
            }
            if index > 0 {
                let prev = &plans[index - 1];
                if prev.current != plan.previous || prev.following != plan.current {
                    return Err(AnalysisError::state("frame plan transitions differ"));
                }
            }
        }
        if pcm.is_empty() {
            return Err(AnalysisError::input("pcm feeder empty"));
        }
        let source_len = pcm[0].len() as i64;
        if source_len < 4096 {
            return Err(AnalysisError::input(format!(
                "pcm feeder short (frames={:?})",
                source_len
            )));
        }
        if let Some((channel, samples)) = pcm
            .iter()
            .enumerate()
            .find(|(_, samples)| samples.len() as i64 != source_len)
        {
            return Err(AnalysisError::input(format!(
                "PCM channel {channel} has {} frames, expected {source_len}",
                samples.len()
            )));
        }

        // LPC priming before packet zero.
        let channels: Vec<Vec<f64>> = pcm
            .iter()
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
        let tail_count =
            tail_training.unwrap_or_else(|| blocksizes.iter().copied().max().unwrap_or(0));
        if tail_count <= 32 || tail_count > source_len {
            return Err(AnalysisError::input(format!(
                "pcm feeder short (frames={:?})",
                source_len
            )));
        }
        let tail_training = tail_count as usize;
        let tails: Vec<Vec<f64>> = channels
            .iter()
            .map(|channel| {
                let len = channel.len();
                let coeffs = wwise_lpc_from_data(&channel[len - tail_training..], 32)
                    .expect("tail LPC coefficients (validated above)");
                wwise_lpc_predict(&coeffs, &channel[len - 32..], tail_count)
                    .expect("tail LPC prediction (validated above)")
            })
            .collect();

        Ok(Self {
            plans: plans.to_vec(),
            blocksizes: blocksizes.to_vec(),
            channels,
            primes,
            tails,
            source_len,
            source_origin: blocksizes[1] / 2,
        })
    }

    /// The plans this source materializes, in analysis order.
    pub fn plans(&self) -> &[FramePlan] {
        &self.plans
    }

    /// Materialize `plan`'s windowed rows, reusing `scratch`'s row buffers
    /// when a finished analysis handed them back and allocating them when
    /// it did not.
    ///
    /// Every field of the returned frame is written here, so a recycled
    /// frame carries nothing over: the plan and the center are overwritten
    /// and each row is refilled from index 0. What survives is the rows'
    /// capacity — the widest block size they have held — so an alternation
    /// of short and long frames reallocates at most once per channel.
    pub fn materialize(
        &self,
        plan: &FramePlan,
        frozen_windows: Option<&HashMap<i64, Vec<f32>>>,
        scratch: Option<WindowedFrame>,
    ) -> Result<WindowedFrame, AnalysisError> {
        if plan.index < 0 || plan.index as usize >= self.plans.len() {
            return Err(AnalysisError::state("frame plans not contiguous"));
        }
        let current = plan.current;
        let previous = plan.previous;
        let following = plan.following;
        let n = self.blocksizes[current as usize];
        let start = plan.sample_start - self.source_origin;
        let center = start + n / 2;
        // Short blocks do not use pending long-transition flags; their
        // effective window is always the short one.
        let window_modes = if current == 0 {
            (0, 0, 0)
        } else {
            (previous, current, following)
        };

        let mut frame = scratch.unwrap_or(WindowedFrame {
            plan: *plan,
            center,
            samples: Vec::new(),
        });
        frame.plan = *plan;
        frame.center = center;
        frame.samples.truncate(self.channels.len());
        while frame.samples.len() < self.channels.len() {
            frame.samples.push(Vec::new());
        }
        for (channel_idx, row) in frame.samples.iter_mut().enumerate() {
            let channel = &self.channels[channel_idx];
            let prime = &self.primes[channel_idx];
            let tail = &self.tails[channel_idx];
            row.clear();
            for sample_index in start..(start + n) {
                let value = if sample_index < 0 {
                    let prime_index = sample_index + (prime.len() as i64);
                    if prime_index >= 0 {
                        prime[prime_index as usize]
                    } else {
                        0.0
                    }
                } else if sample_index < self.source_len {
                    channel[sample_index as usize]
                } else {
                    let tail_index = sample_index - self.source_len;
                    if tail_index < (tail.len() as i64) {
                        tail[tail_index as usize]
                    } else {
                        0.0
                    }
                };
                row.push(value);
            }
            apply_vorbis_window_in_place(
                row,
                &self.blocksizes,
                window_modes.0,
                window_modes.1,
                window_modes.2,
                frozen_windows,
            )?;
        }
        Ok(frame)
    }
}

/// Yield hybrid-windowed PCM blocks for an already-decided mode stream
/// (Python `iter_pcm_windows`).
pub fn iter_pcm_windows(
    pcm: &[Vec<f64>],
    modes: &[i64],
    blocksizes: &[i64],
    terminal_following: i64,
    frozen_windows: Option<&HashMap<i64, Vec<f32>>>,
    tail_training: Option<i64>,
) -> Result<Vec<WindowedFrame>, AnalysisError> {
    let plans = plan_mode_sequence(modes, blocksizes, terminal_following).map_err(|e| match e {
        wem_scheduling::PlannerError::BlockSizeCount => {
            AnalysisError::geometry("window block size invalid")
        }
        _ => AnalysisError::state("frame plan interval mismatch"),
    })?;
    iter_planned_pcm_windows(pcm, &plans, blocksizes, frozen_windows, tail_training)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rejects_empty_plans_and_pcm() {
        assert!(iter_planned_pcm_windows(&[], &[], &[256, 2048], None, None).is_ok());
        let plans = wem_scheduling::plan_mode_sequence(&[0], &[256, 2048], 1).expect("plan");
        assert!(iter_planned_pcm_windows(&[], &plans, &[256, 2048], None, None).is_err());
    }

    #[test]
    fn rejects_short_pcm() {
        let pcm: Vec<Vec<f64>> = vec![vec![0.0f64; 100]];
        let plans = wem_scheduling::plan_mode_sequence(&[0], &[256, 2048], 1).expect("plan");
        assert!(iter_planned_pcm_windows(&pcm, &plans, &[256, 2048], None, None).is_err());
    }
}
