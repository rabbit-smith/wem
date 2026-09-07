//! Temporal weighting and history relaxation for psychoacoustic analysis.
//!
//! Mirrors Python `wwise_wem/analysis/psychoacoustics/temporal.py`. Float32
//! storage boundaries are `f32_of`; the width table is reconstructed exactly.

use crate::config::{f32_of, AnalysisError};

/// Sliding relaxation widths for the 128-bin short-block history recurrence
/// (Python `SHORT_HISTORY_RELAXATION_WIDTHS`). The repeated pushes are
/// intentional: they reconstruct the frozen width table exactly.
#[allow(clippy::same_item_push)]
pub fn short_history_relaxation_widths() -> Vec<i64> {
    let mut widths: Vec<i64> = Vec::with_capacity(128);
    for _ in 0..8 {
        widths.push(1);
    }
    for width in 2..25 {
        for _ in 0..4 {
            widths.push(width);
        }
    }
    for _ in 0..3 {
        widths.push(25);
    }
    for width in (1..=24).rev() {
        widths.push(width);
    }
    widths.push(1);
    debug_assert_eq!(widths.len(), 128);
    widths
}

/// Inputs to the temporal kernel (Python `TemporalKernelInputs`).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct TemporalKernelInputs {
    pub bins: i64,
    pub run_count: i64,
    pub tail_count: i64,
    pub high_rate: bool,
    pub curve_bias: f64,
    pub transition_code: i64,
    pub previous_transition: i64,
    pub update_gate: i64,
    pub analysis_mode: i64,
}

/// Result of the temporal kernel (Python `TemporalKernelResult`).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct TemporalKernelResult {
    pub active: i64,
    pub update_state: i64,
    pub lower_weight: f64,
    pub upper_weight: f64,
    pub state_bias: f64,
    pub chase_seed: f64,
}

impl Default for TemporalKernelResult {
    fn default() -> Self {
        Self {
            active: 0,
            update_state: 0,
            lower_weight: 0.0,
            upper_weight: 0.0,
            state_bias: 0.0,
            chase_seed: 0.0,
        }
    }
}

impl TemporalKernelResult {
    /// The packed word tuple (Python `words`).
    pub fn words(&self) -> (i64, i64, f64, f64, f64, f64) {
        (
            self.active,
            self.update_state,
            self.lower_weight,
            self.upper_weight,
            self.state_bias,
            self.chase_seed,
        )
    }
}

/// Compute temporal weights for the active block geometry and transition
/// (Python `compute_temporal_kernel`).
pub fn compute_temporal_kernel(inputs: &TemporalKernelInputs) -> TemporalKernelResult {
    if !inputs.high_rate {
        return TemporalKernelResult::default();
    }

    let update = if inputs.update_gate == 0 || inputs.analysis_mode == 2 {
        1
    } else {
        0
    };
    if update == 0 && inputs.analysis_mode == 0 {
        return TemporalKernelResult::default();
    }
    if inputs.transition_code != 0 {
        return TemporalKernelResult {
            update_state: update,
            ..TemporalKernelResult::default()
        };
    }

    if inputs.bins == 128 {
        let multiplier = if inputs.curve_bias >= 3.0 { 3 } else { 2 };
        let result = if inputs.previous_transition != 0 {
            TemporalKernelResult {
                active: 1,
                update_state: update,
                lower_weight: 0.7,
                upper_weight: 0.0,
                state_bias: 0.0,
                chase_seed: 8.0,
            }
        } else if inputs.run_count >= 8 {
            TemporalKernelResult {
                active: 1,
                update_state: update,
                lower_weight: 0.3,
                upper_weight: 0.0,
                state_bias: 25.0,
                chase_seed: 0.0,
            }
        } else {
            TemporalKernelResult {
                active: 1,
                update_state: update,
                lower_weight: 0.7 - (inputs.run_count - 1) as f64 / 17.0,
                upper_weight: 0.0,
                state_bias: (inputs.run_count * multiplier) as f64,
                chase_seed: (8 - inputs.run_count) as f64,
            }
        };
        let divisor = 8.0;

        let mut lower = result.lower_weight;
        if inputs.tail_count != 0 {
            lower *= inputs.tail_count as f64 / divisor;
        }
        if inputs.update_gate != 0 && inputs.analysis_mode == 0 {
            lower *= 0.2;
        }
        TemporalKernelResult {
            active: result.active,
            update_state: result.update_state,
            lower_weight: lower,
            upper_weight: result.upper_weight,
            state_bias: result.state_bias,
            chase_seed: result.chase_seed,
        }
    } else if inputs.bins == 256 {
        if inputs.previous_transition != 0 {
            TemporalKernelResult {
                active: 1,
                update_state: update,
                lower_weight: 0.6,
                upper_weight: 0.0,
                state_bias: 12.0,
                chase_seed: 8.0,
            }
        } else if inputs.run_count >= 4 {
            TemporalKernelResult {
                active: 1,
                update_state: update,
                lower_weight: 0.2,
                upper_weight: 0.0,
                state_bias: 30.0,
                chase_seed: 0.0,
            }
        } else {
            let lower_weight = 0.4 - (inputs.run_count - 1) as f64 / 11.0;
            let state_bias = 2.0 * (3.0 * inputs.run_count as f64 + 6.0);
            let chase_seed = 2.0 * (4.0 - inputs.run_count as f64);
            TemporalKernelResult {
                active: 1,
                update_state: update,
                lower_weight,
                upper_weight: 0.0,
                state_bias,
                chase_seed,
            }
        }
    } else {
        // Long blocks do not run the short peak-continuation branch.
        TemporalKernelResult {
            update_state: update,
            ..TemporalKernelResult::default()
        }
    }
}

/// Apply the history rebase that precedes width relaxation
/// (Python `rebase_history`).
pub fn rebase_history(
    inputs: &TemporalKernelInputs,
    result: &TemporalKernelResult,
    state: &[f64],
    history: &[f64],
) -> Result<Vec<f64>, AnalysisError> {
    if state.len() as i64 != inputs.bins || history.len() as i64 != inputs.bins {
        return Err(AnalysisError::RebaseLengthMismatch {
            want: inputs.bins,
        });
    }
    if result.active == 0 || result.update_state == 0 {
        return Ok(history.to_vec());
    }
    let delta = if inputs.bins == 128 {
        5.0
    } else if inputs.bins == 256 {
        10.0
    } else {
        return Ok(history.to_vec());
    };
    let source = if inputs.previous_transition != 0 { state } else { history };
    Ok(source.iter().map(|value| f32_of(*value - delta)).collect())
}

/// Apply sliding width-table relaxation to one history curve
/// (Python `relax_history`).
pub fn relax_history(
    history: &[f64],
    raw: &[f64],
    widths: &[i64],
    delta: f64,
) -> Result<Vec<f64>, AnalysisError> {
    if !(history.len() == raw.len() && history.len() == widths.len()) {
        return Err(AnalysisError::RelaxLengthMismatch);
    }
    let mut out: Vec<f64> = history.iter().map(|value| f32_of(*value)).collect();
    let n = out.len();
    for (k, &width) in widths.iter().enumerate() {
        if width <= 1 {
            continue;
        }
        for j in 1..width {
            let index = k + j as usize;
            if index >= n {
                break;
            }
            if raw[k] - j as f64 * 75.0 / width as f64 > out[index] {
                let denominator = widths[index];
                if denominator <= 0 {
                    return Err(AnalysisError::RelaxWidthNonPositive);
                }
                // Each relaxation is stored as float32 before later comparisons.
                out[index] = f32_of(out[index] + delta / denominator as f64);
            }
        }
    }
    Ok(out)
}

/// Apply the complete 128-bin short-history relaxation table
/// (Python `relax_short_history`).
pub fn relax_short_history(history: &[f64], raw: &[f64]) -> Result<Vec<f64>, AnalysisError> {
    relax_history(history, raw, &short_history_relaxation_widths(), 5.0)
}

/// Update short-block temporal history from local psychoacoustic surfaces
/// (Python `update_short_history`).
#[allow(clippy::too_many_arguments)]
pub fn update_short_history(
    inputs: &TemporalKernelInputs,
    state: &[f64],
    history: &[f64],
    raw: &[f64],
    seed: &[f64],
    remap: &[f64],
    mask_curve: &[f64],
    curve_cap: f64,
    q: f64,
    candidate_bound: i64,
) -> Result<Vec<f64>, AnalysisError> {
    if inputs.bins != 128 {
        return Err(AnalysisError::ShortTemporalBins { want: 128 });
    }
    let arrays = [state, history, raw, seed, remap, mask_curve];
    if arrays.iter().any(|values| values.len() != 128) {
        return Err(AnalysisError::ShortTemporalLength { want: 128 });
    }
    let result = compute_temporal_kernel(inputs);
    let mut baseline = rebase_history(inputs, &result, state, history)?;
    if result.active != 0 && result.update_state != 0 {
        baseline = relax_short_history(&baseline, raw)?;
    } else {
        for value in baseline.iter_mut() {
            *value = f32_of(*value);
        }
    }

    if result.active == 0 {
        return Ok(baseline);
    }
    let mut out = baseline.clone();
    let mut subtract = 0.0;
    if q >= 0.0 && inputs.curve_bias >= 25.0 {
        subtract = q * (inputs.curve_bias - 25.0);
    }
    for index in 0..128usize {
        let cap = f32_of(mask_curve[index] + remap[index]).min(curve_cap);
        let mut candidate = f32_of(seed[index] + inputs.curve_bias);
        if index as i64 <= candidate_bound {
            candidate = f32_of(candidate - subtract);
        }
        if candidate < cap
            && state[index] < cap
            && out[index] + result.state_bias < raw[index]
            && result.update_state != 0 {
                out[index] = f32_of(raw[index]);
            }
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn width_table_length() {
        assert_eq!(short_history_relaxation_widths().len(), 128);
    }

    #[test]
    fn kernel_low_rate_defaults() {
        let inputs = TemporalKernelInputs {
            bins: 128,
            run_count: 1,
            tail_count: 0,
            high_rate: false,
            curve_bias: 0.0,
            transition_code: 0,
            previous_transition: 0,
            update_gate: 0,
            analysis_mode: 0,
        };
        let result = compute_temporal_kernel(&inputs);
        assert_eq!(result.active, 0);
    }
}
