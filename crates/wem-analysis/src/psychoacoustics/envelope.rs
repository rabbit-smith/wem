//! Temporal floor-envelope shaping and history transitions.
//!
//! Mirrors Python `wwise_wem/analysis/psychoacoustics/envelope.py`. Float32
//! storage boundaries are `f32_of`; the flattened no-peak branch preserves
//! the reference encoder's arithmetic order.

use crate::config::{
    f32_of, AnalysisError, LongFloorEnvelopeLook, WwisePsyLongTables, WwisePsyLook,
};

/// Per-channel floor-envelope scratch (Python `FloorEnvelopeScratch`).
#[derive(Debug, Clone, PartialEq)]
pub struct FloorEnvelopeScratch {
    pub current_curve: Vec<f64>,
    pub history_curve: Vec<f64>,
}

impl FloorEnvelopeScratch {
    /// Allocate the observed cleared first-call scratches (Python
    /// `make_floor_envelope_scratch`).
    pub fn new_same_size(n: i64) -> Result<Self, AnalysisError> {
        if n <= 0 {
            return Err(AnalysisError::EnvelopeScratchLengthNonPositive { n });
        }
        Ok(Self {
            current_curve: vec![0.0; n as usize],
            history_curve: vec![0.0; n as usize],
        })
    }

    /// Allocate one cross-block floor-envelope state pair (Python
    /// `make_channel_floor_envelope_scratch`).
    pub fn new_channel() -> Self {
        Self {
            current_curve: vec![0.0; 1024],
            history_curve: vec![0.0; 128],
        }
    }
}

/// Allocate one cross-block floor-envelope state pair (Python
/// `make_channel_floor_envelope_scratch`).
pub fn make_channel_floor_envelope_scratch() -> FloorEnvelopeScratch {
    FloorEnvelopeScratch::new_channel()
}

/// Promote an f32 curve to the f64 working domain
/// (f32 values are exactly representable in f64).
fn f64_curve(values: &[f32]) -> Vec<f64> {
    values.iter().map(|v| *v as f64).collect()
}

/// Port the no-peak branch of the floor-envelope shaping routine
/// (Python `shape_floor_envelope`).
///
/// Returns `(output, side_output, break_index)` where `break_index` records
/// entry into the later peak branch.
#[allow(clippy::too_many_arguments, clippy::type_complexity)]
pub fn shape_floor_envelope(
    target_curve: &[f64],
    psycho_curve: &[f64],
    previous_curve: &[f64],
    base_curve: &[f64],
    side_curve: &[f64],
    mask_curve: &[f64],
    curve_bias: f64,
    q: f64,
    curve_cap: f64,
    history_start: i64,
    history_end: i64,
    state_active: bool,
    history_weight: f64,
    side_gain: f64,
    mode: i64,
    source_curve: Option<&[f64]>,
) -> Result<(Vec<f64>, Vec<f64>, Option<i64>), AnalysisError> {
    let n = target_curve.len();
    if [
        psycho_curve,
        previous_curve,
        base_curve,
        side_curve,
        mask_curve,
    ]
    .iter()
    .any(|values| values.len() != n)
    {
        return Err(AnalysisError::EnvelopeStageLengthMismatch { want: n as i64 });
    }
    let source_curve = source_curve.unwrap_or(target_curve);
    if source_curve.len() != n {
        return Err(AnalysisError::EnvelopeStageLengthMismatch { want: n as i64 });
    }
    if n == 0 {
        return Ok((Vec::new(), Vec::new(), None));
    }
    if mode != 1 {
        return Err(AnalysisError::EnvelopeStageModeUnsupported { mode });
    }

    let mut output = vec![0.0f64; n];
    let mut side_output: Vec<f64> = side_curve.iter().map(|value| f32_of(*value)).collect();
    let stop = history_start;
    let blend_end = history_end;
    let mut subtract = 0.0;
    if q >= 0.0 && curve_bias >= 25.0 {
        subtract = q * (curve_bias - 25.0);
    }
    let mut break_index: Option<i64> = None;

    for index in 0..n {
        // remap delta scratch is a float buffer.
        let floor_value = f32_of(mask_curve[index] + base_curve[index]).min(curve_cap);
        let mut target = target_curve[index] + curve_bias;
        if index as i64 <= stop {
            target -= subtract;
        }

        // floor-envelope stage enters the regular path directly unless the
        // active state asks for the peak continuation and the previous curve
        // fails its gate.
        if state_active
            && target < floor_value
            && psycho_curve[index] < floor_value
            && previous_curve[index] < psycho_curve[index]
        {
            break_index = Some(index as i64);
            break;
        }

        if floor_value <= target {
            let mut chosen = target;
            if (index as i64) > stop && (index as i64) < blend_end {
                let current = source_curve[index];
                if current < target {
                    if current >= floor_value {
                        chosen = current;
                    } else {
                        chosen = target - (target - floor_value) * history_weight;
                    }
                }
            }
            output[index] = f32_of(chosen);
        } else {
            output[index] = f32_of(floor_value);
        }

        // The reference encoder scales the side curve from the floor
        // candidate rather than the selected output envelope.
        let delta = f32_of(floor_value - source_curve[index]);
        let distance = delta + 17.20000076293945;
        let factor = if delta <= -17.200001 {
            f32_of(1.0 - distance * 0.0003 * side_gain)
        } else {
            let mut factor = f32_of(1.0 - distance * 0.005 * side_gain);
            if factor < 0.0 {
                factor = 0.0001;
            }
            factor
        };
        side_output[index] = f32_of(side_output[index] * factor);
    }

    Ok((output, side_output, break_index))
}

/// Expand a 128-bin short floor-envelope state curve to 1024 bins
/// (Python `prepare_short_to_long_history`).
pub fn prepare_short_to_long_history(raw_short: &[f64]) -> Result<Vec<f64>, AnalysisError> {
    if raw_short.len() != 128 {
        return Err(AnalysisError::FloorTransitionBins {
            want: 128,
            got: raw_short.len() as i64,
        });
    }
    let mut out = Vec::with_capacity(1024);
    for value in raw_short {
        let value = f32_of(*value);
        for _ in 0..8 {
            out.push(value);
        }
    }
    Ok(out)
}

/// Apply a long→short transition's 8:1 minimum state reduction
/// (Python `prepare_long_to_short_history`).
pub fn prepare_long_to_short_history(
    raw_long: &[f64],
    previous_state: &[f64],
) -> Result<Vec<f64>, AnalysisError> {
    if raw_long.len() != 1024 || previous_state.len() != 1024 {
        return Err(AnalysisError::FloorTransitionBins {
            want: 1024,
            got: raw_long.len() as i64,
        });
    }
    let mut out: Vec<f64> = previous_state.iter().map(|value| f32_of(*value)).collect();
    for index in 0..128 {
        let group = &raw_long[index * 8..index * 8 + 8];
        let minimum = group.iter().fold(f64::INFINITY, |a, b| a.min(*b));
        out[index] = f32_of(minimum);
    }
    Ok(out)
}

/// Run the profile-bound first regular floor-envelope stage call
/// (Python `shape_first_floor_envelope`).
pub fn shape_first_floor_envelope(
    seed: &[f64],
    remap: &[f64],
    raw: &[f64],
    look: &WwisePsyLook,
    scratch: &mut FloorEnvelopeScratch,
    q: f64,
    side: Option<&[f64]>,
) -> Result<(Vec<f64>, Vec<f64>), AnalysisError> {
    let n = raw.len();
    if seed.len() != n
        || remap.len() != n
        || look.mask_curves.1.len() != n
        || scratch.current_curve.len() != n
        || scratch.history_curve.len() != n
    {
        return Err(AnalysisError::FirstEnvelopeBuffersMismatch { want: n as i64 });
    }
    let side = match side {
        Some(side) => side.to_vec(),
        None => vec![0.0; n],
    };
    let mask_curve = f64_curve(&look.mask_curves.1);
    let (post, side_out, break_index) = shape_floor_envelope(
        seed,
        &scratch.current_curve,
        &scratch.history_curve,
        remap,
        &side,
        &mask_curve,
        look.regular_curve_bias as f64,
        q,
        look.regular_curve_cap as f64,
        0,
        0,
        false,
        0.0,
        1.0,
        1,
        Some(raw),
    )?;
    if break_index.is_some() {
        return Err(AnalysisError::FirstEnvelopeEnteredPeakBranch);
    }
    for (slot, value) in scratch.current_curve.iter_mut().zip(raw.iter()) {
        *slot = f32_of(*value);
    }
    for slot in scratch.history_curve.iter_mut() {
        *slot = -5.0;
    }
    Ok((post, side_out))
}

/// Run a fresh long profile's inactive regular floor-envelope call
/// (Python `shape_first_long_floor_envelope`).
#[allow(clippy::too_many_arguments)]
pub fn shape_first_long_floor_envelope(
    seed: &[f64],
    remap: &[f64],
    raw: &[f64],
    side: &[f64],
    scratch: &mut FloorEnvelopeScratch,
    q: f64,
    look: Option<&LongFloorEnvelopeLook>,
    table: Option<&WwisePsyLongTables>,
    transition: i64,
    same_run: i64,
) -> Result<(Vec<f64>, Vec<f64>), AnalysisError> {
    let computed_look = table
        .map(crate::config::make_long_floor_envelope_look)
        .transpose()?;
    let look = match look {
        Some(look) => look,
        None => computed_look
            .as_ref()
            .ok_or(AnalysisError::LongFloorGeometry)?,
    };
    let n = look.n as usize;
    if !(seed.len() == remap.len()
        && seed.len() == raw.len()
        && seed.len() == side.len()
        && seed.len() == n
        && scratch.current_curve.len() == n
        && (scratch.history_curve.len() == 128 || scratch.history_curve.len() == n))
    {
        return Err(AnalysisError::LongEnvelopeStateBins);
    }
    // history inside the generic floor loop.
    let history_curve: Vec<f64> = if scratch.history_curve.len() == n {
        scratch.history_curve.clone()
    } else {
        vec![0.0; n]
    };
    let mask_curve = f64_curve(&look.mask_curve);
    let (post, side_out, break_index) = shape_floor_envelope(
        seed,
        &scratch.current_curve,
        &history_curve,
        remap,
        side,
        &mask_curve,
        look.curve_bias as f64,
        q,
        look.curve_cap as f64,
        look.history_start,
        n as i64,
        false,
        0.0,
        look.side_gain as f64,
        1,
        Some(raw),
    )?;
    if break_index.is_some() {
        return Err(AnalysisError::FirstEnvelopeEnteredPeakBranch);
    }
    if (transition, same_run) == (2, 0) {
        let reduced = prepare_long_to_short_history(raw, &scratch.current_curve)?;
        scratch.current_curve = reduced;
    } else {
        for (slot, value) in scratch.current_curve.iter_mut().zip(raw.iter()) {
            *slot = f32_of(*value);
        }
    }
    // The checked long path does not write the fresh history allocation.
    Ok((post, side_out))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn prepare_short_to_long_history_basic() {
        let raw: Vec<f64> = (0..128).map(|i| i as f64).collect();
        let out = prepare_short_to_long_history(&raw).unwrap();
        assert_eq!(out.len(), 1024);
        assert_eq!(out[0], 0.0);
        assert_eq!(out[8], 1.0);
    }

    #[test]
    fn prepare_long_to_short_history_basic() {
        let raw: Vec<f64> = (0..1024).map(|i| i as f64).collect();
        let prev: Vec<f64> = (0..1024).map(|i| -(i as f64)).collect();
        let out = prepare_long_to_short_history(&raw, &prev).unwrap();
        assert_eq!(out.len(), 1024);
        // First group min = 0.
        assert_eq!(out[0], 0.0);
        // Bins beyond 128 retain previous.
        assert_eq!(out[128], -128.0);
    }
}
