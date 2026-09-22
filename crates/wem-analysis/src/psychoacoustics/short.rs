//! Short-block psychoacoustic floor-envelope analysis.
//!
//! Mirrors Python `wwise_wem/analysis/psychoacoustics/short.py`. Float32
//! storage boundaries are `f32_of`; tuning literals retain their exact double
//! values so a single-ULP loss is not introduced at final stores.

use crate::config::{f32_of, u32_to_f32, AnalysisError, ShortPsyProfile};
use crate::psychoacoustics::temporal::{
    compute_temporal_kernel, relax_short_history, TemporalKernelInputs,
};

const N: i64 = 128;
const NEG_17_2: f64 = -17.20000076293945;
// Retain exact double values of the source float spellings.
const F64_0_1: f64 = 0.10000000149011612;
const F64_0_2: f64 = 0.20000000298023224;
const F64_0_3: f64 = 0.30000001192092896;

/// Temporal weights and state controls for short envelope analysis
/// (Python `ShortPsyKernel`).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct ShortPsyKernel {
    pub active: i64,
    pub update_history: i64,
    pub lower_weight: f64,
    pub upper_weight: f64,
    pub state_bias: f64,
    pub chase_seed: f64,
}

impl ShortPsyKernel {
    /// Build from six stored words (Python `from_words`).
    pub fn from_words(words: &[u32]) -> Result<Self, AnalysisError> {
        if words.len() != 6 {
            return Err(AnalysisError::ShortKernelWords {
                got: words.len() as i64,
            });
        }
        Ok(Self {
            active: words[0] as i64,
            update_history: words[1] as i64,
            lower_weight: u32_to_f32(words[2]) as f64,
            upper_weight: u32_to_f32(words[3]) as f64,
            state_bias: u32_to_f32(words[4]) as f64,
            chase_seed: u32_to_f32(words[5]) as f64,
        })
    }
}

/// All mutable output surfaces of one 128-bin channel analysis
/// (Python `ShortPsyChannelResult`).
#[derive(Debug, Clone, PartialEq)]
pub struct ShortPsyChannelResult {
    pub post: Vec<f64>,
    pub side: Vec<f64>,
    pub history: Vec<f64>,
    pub groups: Vec<f64>,
    pub peak_bins: i64,
}

/// Persistent state allocation shared by one encoder channel
/// (Python `PsyChannelState`).
#[derive(Debug, Clone, PartialEq)]
pub struct PsyChannelState {
    pub state: Vec<f64>,
    pub history: Vec<f64>,
}

impl PsyChannelState {
    /// Allocate one fresh channel state (Python `fresh`).
    pub fn fresh() -> Self {
        Self {
            state: vec![0.0; 1024],
            history: vec![0.0; N as usize],
        }
    }
}

/// Previous transition, equal-run count, and saturating tail count
/// (Python `PsyTemporalState`).
#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub struct PsyTemporalState {
    pub previous_transition: i64,
    pub run_count: i64,
    pub tail_count: i64,
}

/// Scheduler-derived controls used for one short six-channel group
/// (Python `PsyFrameControls`).
#[derive(Debug, Clone, PartialEq)]
pub struct PsyFrameControls {
    pub transition: i64,
    pub previous_transition: i64,
    pub run_count: i64,
    pub tail_count: i64,
    pub following_mode: i64,
    pub profile_key: String,
}

/// Post/side outputs plus the exact controls consumed by a frame
/// (Python `ShortPsyFrameResult`).
#[derive(Debug, Clone, PartialEq)]
pub struct ShortPsyFrameResult {
    pub info: PsyFrameControls,
    pub channels: Vec<ShortPsyChannelResult>,
}

/// Own active-short psychoacoustic state for every channel
/// (Python `ShortPsyAnalyzer`).
pub struct ShortPsyAnalyzer {
    pub profiles: Vec<ShortPsyProfile>,
    pub channels: Vec<PsyChannelState>,
    pub temporal: PsyTemporalState,
}

impl ShortPsyAnalyzer {
    /// Construct the analyzer (Python `__init__`).
    pub fn new(
        channel_count: i64,
        profiles: Vec<ShortPsyProfile>,
        channels: Option<Vec<PsyChannelState>>,
        temporal: Option<PsyTemporalState>,
    ) -> Result<Self, AnalysisError> {
        if channel_count <= 0 {
            return Err(AnalysisError::ShortAnalyzerChannelsNonPositive {
                channels: channel_count,
            });
        }
        if profiles.len() != 2 {
            return Err(AnalysisError::ShortAnalyzerProfiles {
                want: 2,
                got: profiles.len() as i64,
            });
        }
        let channels = match channels {
            Some(channels) => channels,
            None => (0..channel_count)
                .map(|_| PsyChannelState::fresh())
                .collect(),
        };
        if channels.len() as i64 != channel_count {
            return Err(AnalysisError::ShortAnalyzerChannelStateCount {
                want: channel_count,
            });
        }
        for channel in &channels {
            if channel.state.len() as i64 != 1024 || channel.history.len() as i64 != N {
                return Err(AnalysisError::ShortChannelStateGeometry);
            }
        }
        let temporal = temporal.unwrap_or_default();
        Ok(Self {
            profiles,
            channels,
            temporal,
        })
    }

    /// Derive frame controls for a short variant (Python `_frame_controls`).
    fn frame_controls(
        &self,
        short_variant: i64,
        following_mode: i64,
    ) -> Result<PsyFrameControls, AnalysisError> {
        if !(0..=1).contains(&short_variant) {
            return Err(AnalysisError::ShortVariantInvalid {
                variant: short_variant,
            });
        }
        if !(0..=1).contains(&following_mode) {
            return Err(AnalysisError::ShortFollowingModeInvalid {
                following: following_mode,
            });
        }
        let transition = short_variant;
        let profile = &self.profiles[transition as usize];
        Ok(PsyFrameControls {
            transition,
            previous_transition: self.temporal.previous_transition,
            run_count: self.temporal.run_count,
            tail_count: self.temporal.tail_count,
            following_mode,
            profile_key: profile.key.clone(),
        })
    }

    /// Advance temporal counters once after the channel group
    /// (Python `_advance_temporal_state`).
    fn advance_temporal_state(&mut self, transition: i64) {
        let previous = self.temporal.previous_transition;
        let mut tail = self.temporal.tail_count;
        if transition >= 2 {
            tail = 0;
        }
        if previous != 0 || transition != 1 {
            if tail != 0 && tail < 8 {
                tail += 1;
            }
        } else {
            tail = 1;
        }
        let run = if previous == transition {
            self.temporal.run_count + 1
        } else {
            1
        };
        self.temporal.previous_transition = transition;
        self.temporal.run_count = run;
        self.temporal.tail_count = tail;
    }

    /// Commit the short state curve for the following block geometry
    /// (Python `_commit_state_transition`).
    fn commit_state_transition(
        channel: &mut PsyChannelState,
        raw: &[f64],
        info: &PsyFrameControls,
        update_state: i64,
    ) -> Result<(), AnalysisError> {
        if update_state == 0 {
            return Ok(());
        }
        if !(0..=1).contains(&info.transition) {
            return Err(AnalysisError::ShortAnalysisCannotApplyLongTransition {
                transition: info.transition,
            });
        }
        if info.following_mode != 0 {
            let mut expanded = Vec::with_capacity(1024);
            for value in raw {
                let value = f32_of(*value);
                for _ in 0..8 {
                    expanded.push(value);
                }
            }
            channel.state = expanded;
        } else {
            for (slot, value) in channel.state.iter_mut().take(N as usize).zip(raw.iter()) {
                *slot = f32_of(*value);
            }
        }
        Ok(())
    }

    /// Run all channel calls of one short packet/frame (Python `process_frame`).
    #[allow(clippy::too_many_arguments)]
    pub fn process_frame(
        &mut self,
        remaps: &[Vec<f64>],
        seeds: &[Vec<f64>],
        sides: &[Vec<f64>],
        raws: &[Vec<f64>],
        short_variant: i64,
        following_mode: i64,
        q: f64,
        hold_update: i64,
        groups: Option<&[Vec<f64>]>,
    ) -> Result<ShortPsyFrameResult, AnalysisError> {
        let count = self.channels.len() as i64;
        for values in [remaps, seeds, sides, raws].iter() {
            if values.len() as i64 != count {
                return Err(AnalysisError::ShortFrameChannelCountMismatch);
            }
        }
        if let Some(groups) = groups {
            if groups.len() as i64 != count {
                return Err(AnalysisError::ShortGroupWorkLength {
                    want: count,
                    got: groups.len() as i64,
                });
            }
        }
        let info = self.frame_controls(short_variant, following_mode)?;
        let profile = self.profiles[info.transition as usize].clone();
        let kernel_state = compute_temporal_kernel(&TemporalKernelInputs {
            bins: N,
            run_count: info.run_count,
            tail_count: info.tail_count,
            high_rate: true,
            curve_bias: profile.candidate_bias_by_mode[1],
            transition_code: info.transition,
            previous_transition: info.previous_transition,
            hold_update,
            analysis_mode: 1,
        });
        let kernel = ShortPsyKernel {
            active: kernel_state.active,
            update_history: kernel_state.update_state,
            lower_weight: f32_of(kernel_state.lower_weight),
            upper_weight: f32_of(kernel_state.upper_weight),
            state_bias: f32_of(kernel_state.state_bias),
            chase_seed: f32_of(kernel_state.chase_seed),
        };
        let group_count = (N + profile.group_span - 1) / profile.group_span;
        let mut results: Vec<ShortPsyChannelResult> = Vec::new();
        for index in 0..self.channels.len() {
            let remap = &remaps[index];
            let seed = &seeds[index];
            let side = &sides[index];
            let raw = &raws[index];
            let channel = &mut self.channels[index];
            let work: Vec<f64> = if let Some(groups) = groups {
                groups[index].clone()
            } else {
                vec![0.0; group_count as usize]
            };
            let result = shape_short_floor_envelope(
                &profile,
                remap,
                seed,
                side,
                raw,
                &channel.state[..N as usize],
                &channel.history,
                &work,
                kernel,
                1,
                q,
                info.tail_count,
                info.previous_transition,
            )?;
            channel.history.clone_from(&result.history);
            Self::commit_state_transition(channel, raw, &info, kernel.update_history)?;
            results.push(result);
        }
        self.advance_temporal_state(info.transition);
        Ok(ShortPsyFrameResult {
            info,
            channels: results,
        })
    }

    /// Advance the shared state allocation through a 1024-bin frame
    /// (Python `commit_long_state`).
    pub fn commit_long_state(
        &mut self,
        raws: &[Vec<f64>],
        long_variant: i64,
        following_mode: i64,
        hold_update: i64,
    ) -> Result<PsyFrameControls, AnalysisError> {
        if !(0..=1).contains(&long_variant) {
            return Err(AnalysisError::LongFrameVariantInvalid {
                variant: long_variant,
            });
        }
        if !(0..=1).contains(&following_mode) {
            return Err(AnalysisError::LongFrameVariantInvalid {
                variant: following_mode,
            });
        }
        if raws.len() as i64 != self.channels.len() as i64
            || raws.iter().any(|raw| raw.len() as i64 != 1024)
        {
            return Err(AnalysisError::LongAnalysisFrameSize { want: 1024 });
        }
        let transition = 2 | long_variant;
        let info = PsyFrameControls {
            transition,
            previous_transition: self.temporal.previous_transition,
            run_count: self.temporal.run_count,
            tail_count: self.temporal.tail_count,
            following_mode,
            profile_key: "long_state_only".to_string(),
        };
        // Long-frame peak continuation is inactive; the hold flag only
        // controls whether the state curve is committed.
        if hold_update == 0 {
            for (raw, channel) in raws.iter().zip(self.channels.iter_mut()) {
                if following_mode != 0 {
                    channel.state = raw.iter().map(|v| f32_of(*v)).collect();
                } else {
                    for index in 0..N as usize {
                        let group = &raw[index * 8..index * 8 + 8];
                        let minimum = group.iter().fold(f64::INFINITY, |a, b| a.min(*b));
                        channel.state[index] = f32_of(minimum);
                    }
                }
            }
        }
        self.advance_temporal_state(transition);
        Ok(info)
    }
}

/// Prepare the rebased and relaxed 128-bin history curve
/// (Python `_prepare_history`).
fn prepare_history(
    raw: &[f64],
    state: &[f64],
    history: &[f64],
    kernel: ShortPsyKernel,
    previous_transition: i64,
) -> Result<Vec<f64>, AnalysisError> {
    if kernel.active == 0 || kernel.update_history == 0 {
        return Ok(history.iter().map(|value| f32_of(*value)).collect());
    }
    let source = if previous_transition != 0 {
        state
    } else {
        history
    };
    let rebased: Vec<f64> = source.iter().map(|value| f32_of(*value - 5.0)).collect();
    relax_short_history(&rebased, raw)
}

/// Shape the active 128-bin floor envelope for one channel
/// (Python `shape_short_floor_envelope`).
#[allow(clippy::too_many_arguments)]
pub fn shape_short_floor_envelope(
    profile: &ShortPsyProfile,
    remap: &[f64],
    seed: &[f64],
    side: &[f64],
    raw: &[f64],
    state: &[f64],
    history: &[f64],
    groups: &[f64],
    kernel: ShortPsyKernel,
    mode: i64,
    q: f64,
    tail_count: i64,
    previous_transition: i64,
) -> Result<ShortPsyChannelResult, AnalysisError> {
    let vectors = [remap, seed, side, raw, state, history];
    if vectors.iter().any(|values| values.len() as i64 != N) {
        return Err(AnalysisError::ShortVectorsLength { want: N });
    }
    if mode < 0 || mode >= profile.mask_curves.len() as i64 {
        return Err(AnalysisError::ShortModeOutOfRange {
            mode,
            curves: profile.mask_curves.len() as i64,
        });
    }
    if mode >= profile.candidate_bias_by_mode.len() as i64 {
        return Err(AnalysisError::ShortModeBiasOutOfRange { mode });
    }
    let expected_groups = (N + profile.group_span - 1) / profile.group_span;
    if groups.len() as i64 != expected_groups {
        return Err(AnalysisError::ShortGroupWorkLength {
            want: expected_groups,
            got: groups.len() as i64,
        });
    }

    let curve = &profile.mask_curves[mode as usize];
    let candidate_bias = profile.candidate_bias_by_mode[mode as usize];
    let mut subtract = 0.0;
    if q >= 0.0 && candidate_bias >= 25.0 {
        // Preserve extended precision until the product is stored as float32.
        subtract = f32_of(q * (candidate_bias - 25.0));
    }

    // Prepare history before the envelope loop.
    let mut history_out = prepare_history(raw, state, history, kernel, previous_transition)?;
    let mut post = vec![0.0f64; N as usize];
    let mut side_out: Vec<f64> = side.iter().map(|value| f32_of(*value)).collect();
    let mut groups_out: Vec<f64> = groups.iter().map(|value| f32_of(*value)).collect();

    // The chase minimum remains zero throughout this path.
    let chase_min = 0.0;
    let mut peak_bins = 0;
    let low_band = profile.band_limits[0];
    let middle_band = profile.band_limits[1];
    let fine_band = profile.band_limits[2];

    for index in 0..N as usize {
        // Materialize both the cap and candidate as float32 before comparing.
        let mut cap = f32_of(curve[index] + remap[index]);
        if profile.curve_cap < cap {
            cap = f32_of(profile.curve_cap);
        }
        let mut candidate = f32_of(seed[index] + candidate_bias);
        if index as i64 <= profile.candidate_bound {
            candidate = f32_of(candidate - subtract);
        }

        let peak = kernel.active != 0
            && candidate < cap
            && state[index] < cap
            && f32_of(history_out[index] + kernel.state_bias) < raw[index];
        let mut marked_peak = false;

        // The intermediate envelope value, used by both the group-work update
        // and the side attenuation (not the final post value).
        let value;
        if peak {
            peak_bins += 1;
            if kernel.update_history != 0 {
                history_out[index] = f32_of(raw[index]);
            }

            let mut weight = if state[index] >= raw[index] {
                kernel.upper_weight
            } else {
                kernel.lower_weight
            };

            // Optionally chase strong peaks in the low-frequency region.
            if tail_count == 0
                && (index as i64) < profile.peak_cutoff
                && (cap - state[index]) > 20.0
                && (raw[index] - state[index]) > 25.0
            {
                marked_peak = true;
                if candidate > -100.0 && (raw[index] - candidate) < 48.0 {
                    let delta_state = raw[index] - state[index];
                    let reduction = if delta_state >= 35.0 {
                        kernel.chase_seed
                    } else {
                        f32_of((35.0 - delta_state) * F64_0_1 * kernel.chase_seed)
                    };
                    candidate = f32_of(f32_of(candidate - reduction).max(-100.0));
                    if (raw[index] - candidate) > 48.0 {
                        candidate = f32_of(raw[index] - 48.0);
                    }
                }
            }

            // The nesting matters: bins 8 and 9 use width 20.
            let width = if index as i64 <= low_band {
                if index as i64 <= middle_band {
                    weight = f32_of(
                        weight
                            * (if index as i64 <= fine_band {
                                F64_0_3
                            } else {
                                0.5
                            }),
                    );
                    10.0
                } else {
                    20.0
                }
            } else {
                30.0
            };

            // Keep the cap/candidate difference in extended precision.
            let mut gap = cap - candidate;
            if gap > width {
                gap = width + (gap - width) * F64_0_1;
            }
            let ceiling = f32_of(cap - f32_of(gap * weight));
            let mut value_local = if state[index] >= ceiling {
                state[index]
            } else {
                ceiling
            };

            // Peak marks restrain a curve more than 20 dB above the state curve.
            if marked_peak {
                let state_floor = state[index].max(-140.0);
                let separation = f32_of(value_local - state_floor);
                if separation > 20.0 {
                    value_local = f32_of(value_local - (separation - 20.0) * F64_0_2);
                }
            }
            value = value_local;
        } else {
            // Inactive bins use the cap directly.
            value = cap;
        }

        // Only active peaks update caller-owned group work.
        if peak {
            let group_index = index / profile.group_span as usize;
            if marked_peak {
                groups_out[group_index] = -1.0;
            } else if chase_min < groups_out[group_index] {
                groups_out[group_index] = f32_of(chase_min);
            }
        }

        // The middle range is dormant for installed profiles.
        let output;
        if value <= candidate {
            if index as i64 <= profile.candidate_bound || index as i64 >= profile.peak_cutoff {
                output = candidate;
            } else if raw[index] < candidate {
                if raw[index] >= value {
                    output = f32_of(raw[index]);
                } else {
                    output = f32_of(candidate - f32_of(candidate - value) * profile.blend_weight);
                }
            } else {
                output = candidate;
            }
        } else {
            output = value;
        }
        post[index] = f32_of(output);

        // Side attenuation references the intermediate envelope value rather
        // than the post value.
        let delta = f32_of(value - raw[index]);
        let distance = delta - NEG_17_2;
        let factor = if delta <= -17.200001 {
            f32_of(1.0 - distance * 0.0003 * profile.side_gain)
        } else {
            let mut factor = f32_of(1.0 - distance * 0.005 * profile.side_gain);
            if delta > -17.200001 && chase_min > factor {
                factor = 0.0001;
            }
            factor
        };
        side_out[index] = f32_of(side_out[index] * factor);
    }

    Ok(ShortPsyChannelResult {
        post,
        side: side_out,
        history: history_out,
        groups: groups_out,
        peak_bins,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn kernel_from_words() {
        let kernel = ShortPsyKernel::from_words(&[1, 0, 0, 0, 0, 0]).unwrap();
        assert_eq!(kernel.active, 1);
        assert_eq!(kernel.update_history, 0);
    }
}
