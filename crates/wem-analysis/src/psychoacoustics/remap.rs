//! Psychoacoustic smoothing, peak suppression, and spectrum remapping.
//!
//! Mirrors Python `wwise_wem/analysis/psychoacoustics/remap.py`. Every
//! float32 storage boundary is a `f32_of` call; line-fit and peak-suppression
//! ordering is preserved verbatim.

use crate::config::{f32_of, u32_to_f32, AnalysisError, WwisePsyLongTables, WwisePsyLook};

const LONG_PSY_N: i64 = 1024;

/// Pure local result of the 1024-bin mode-2 remap front end
/// (Python `LongRemapResult`).
#[derive(Debug, Clone, PartialEq)]
pub struct LongRemapResult {
    pub first_smooth: Vec<f64>,
    pub residual: Vec<f64>,
    pub selector: Vec<f64>,
    pub base_before_peak: Vec<f64>,
    pub base: Vec<f64>,
    pub remap: Vec<f64>,
}

/// Decode the signed high half of a profile interpolation packed interval
/// (Python `_wwise_signed_hi16`).
fn wwise_signed_hi16(value: i64) -> i64 {
    let packed = value as u32;
    let high = (packed >> 16) & 0xFFFF;
    if high & 0x8000 != 0 {
        high as i64 - 0x10000
    } else {
        high as i64
    }
}

fn wwise_lo16(value: i64) -> i64 {
    (value as u32 & 0xFFFF) as i64
}

/// Build the five float prefix arrays used by the reference smoothing
/// routine (Python `_wwise_psy_prefix_moments`).
#[allow(clippy::type_complexity)]
fn wwise_psy_prefix_moments(
    curve: &[f64],
    offset: f64,
) -> (Vec<f64>, Vec<f64>, Vec<f64>, Vec<f64>, Vec<f64>) {
    let mut p0: Vec<f64> = Vec::with_capacity(curve.len());
    let mut p1: Vec<f64> = Vec::with_capacity(curve.len());
    let mut p2: Vec<f64> = Vec::with_capacity(curve.len());
    let mut q0: Vec<f64> = Vec::with_capacity(curve.len());
    let mut q1: Vec<f64> = Vec::with_capacity(curve.len());
    let mut s0 = 0.0f64;
    let mut s1 = 0.0f64;
    let mut s2 = 0.0f64;
    let mut t0 = 0.0f64;
    let mut t1 = 0.0f64;
    for (i, value) in curve.iter().enumerate() {
        // smoothing is compiled from Xiph's float locals on reference.
        // The expression temporaries stay wide, but every assignment to
        // y, w and the five running t* accumulators is a float32 boundary.
        let x = f32_of(f32_of(*value) + f32_of(offset)).max(1.0);
        let x2 = f32_of(x * x);
        let endpoint_weight = if i == 0 { 0.5 } else { 1.0 };
        let weighted = f32_of(endpoint_weight * x2);
        s0 = f32_of(s0 + weighted);
        // bark_noise_hybridmp initializes its X term at one for the
        // half-weight endpoint, then advances to integer coordinates.
        s1 = f32_of(s1 + if i == 0 { weighted } else { i as f64 * x2 });
        // Keep the source expression order w*x*x.
        s2 = f32_of(
            s2 + if i == 0 {
                0.0
            } else {
                x2 * i as f64 * i as f64
            },
        );
        t0 = f32_of(t0 + weighted * x);
        // The source does not add w*y to tXY in its special i=0 setup.
        t1 = f32_of(t1 + if i == 0 { 0.0 } else { x2 * i as f64 * x });
        p0.push(f32_of(s0));
        p1.push(f32_of(s1));
        p2.push(f32_of(s2));
        q0.push(f32_of(t0));
        q1.push(f32_of(t1));
    }
    (p0, p1, p2, q0, q1)
}

/// Fit one Bark interval using the moment prefixes; return the updated
/// (slope_num, intercept_num, denominator) triple.
#[allow(clippy::too_many_arguments)]
fn fit_interval(
    p0: &[f64],
    p1: &[f64],
    p2: &[f64],
    q0: &[f64],
    q1: &[f64],
    start: i64,
    end: i64,
    crossing_zero: bool,
    n: i64,
    slope_num: f64,
    intercept_num: f64,
    denominator: f64,
) -> Result<(f64, f64, f64), AnalysisError> {
    if !(start >= 0 && start < n && end >= 0 && end < n) {
        return Err(AnalysisError::PsyIntervalEndpointOutOfRange { start, end, n });
    }
    let (a, b, c, y, xy) = if crossing_zero {
        (
            f32_of(p0[end as usize] + p0[start as usize]),
            f32_of(p1[end as usize] - p1[start as usize]),
            f32_of(p2[end as usize] + p2[start as usize]),
            f32_of(q0[end as usize] + q0[start as usize]),
            f32_of(q1[end as usize] - q1[start as usize]),
        )
    } else {
        (
            f32_of(p0[end as usize] - p0[start as usize]),
            f32_of(p1[end as usize] - p1[start as usize]),
            f32_of(p2[end as usize] - p2[start as usize]),
            f32_of(q0[end as usize] - q0[start as usize]),
            f32_of(q1[end as usize] - q1[start as usize]),
        )
    };
    // A/B/D/R are source float locals.
    let den = f32_of(a * c - b * b);
    if den.abs() < 1e-20 {
        return Ok((slope_num, intercept_num, denominator));
    }
    // slope=(A*XY-B*Y)/den, intercept=(C*Y-B*XY)/den
    Ok((f32_of(a * xy - b * y), f32_of(c * y - b * xy), den))
}

/// Port the first, active reference smoothing pass
/// (Python `wwise_psy_curve_smooth`).
pub fn wwise_psy_curve_smooth(
    curve: &[f64],
    interval_table: &[i64],
    offset: f64,
    fixed_window: i64,
) -> Result<Vec<f64>, AnalysisError> {
    let n = curve.len() as i64;
    if n == 0 {
        return Ok(Vec::new());
    }
    if (interval_table.len() as i64) < n {
        return Err(AnalysisError::PsyIntervalTableShort {
            want: n,
            got: interval_table.len() as i64,
        });
    }

    let (p0, p1, p2, q0, q1) = wwise_psy_prefix_moments(curve, offset);
    let mut out = vec![0.0f64; n as usize];
    let mut slope_num = 0.0;
    let mut intercept_num = 0.0;
    let mut denominator = 1.0;
    let _floor = 0.0;
    let mut xcoord = 0.0;
    let mut emitted = 0i64;

    for &packed in interval_table.iter() {
        if emitted >= n {
            break;
        }
        let start = wwise_signed_hi16(packed);
        let end = wwise_lo16(packed);
        if end >= n {
            break;
        }
        let crossing_zero = start < 0;
        let start = if crossing_zero { -start } else { start };
        if start >= n {
            break;
        }
        (slope_num, intercept_num, denominator) = fit_interval(
            &p0,
            &p1,
            &p2,
            &q0,
            &q1,
            start,
            end,
            crossing_zero,
            n,
            slope_num,
            intercept_num,
            denominator,
        )?;
        let mut fitted = f32_of((xcoord * slope_num + intercept_num) / denominator);
        fitted = fitted.max(0.0);
        out[emitted as usize] = f32_of(fitted - offset);
        emitted += 1;
        xcoord += 1.0;
    }

    while emitted < n {
        let mut fitted = f32_of((xcoord * slope_num + intercept_num) / denominator);
        fitted = fitted.max(0.0);
        out[emitted as usize] = f32_of(fitted - offset);
        emitted += 1;
        xcoord += 1.0;
    }

    let fixed = fixed_window;
    if fixed <= 0 {
        return Ok(out);
    }

    // Fixed-width tail pass 1 (left edge, crossing zero).
    emitted = 0;
    xcoord = 0.0;
    while emitted < n {
        let end = emitted + fixed / 2;
        let start = end - fixed;
        if end >= n || start >= 0 {
            break;
        }
        (slope_num, intercept_num, denominator) = fit_interval(
            &p0,
            &p1,
            &p2,
            &q0,
            &q1,
            -start,
            end,
            true,
            n,
            slope_num,
            intercept_num,
            denominator,
        )?;
        let candidate =
            f32_of(f32_of((xcoord * slope_num + intercept_num) / denominator) - f32_of(offset));
        if candidate < out[emitted as usize] {
            out[emitted as usize] = candidate;
        }
        emitted += 1;
        xcoord += 1.0;
    }

    // Fixed-width tail pass 2 (interior, not crossing zero).
    while emitted < n {
        let end = emitted + fixed / 2;
        let start = end - fixed;
        if end >= n || start < 0 {
            break;
        }
        (slope_num, intercept_num, denominator) = fit_interval(
            &p0,
            &p1,
            &p2,
            &q0,
            &q1,
            start,
            end,
            false,
            n,
            slope_num,
            intercept_num,
            denominator,
        )?;
        let candidate =
            f32_of(f32_of((xcoord * slope_num + intercept_num) / denominator) - f32_of(offset));
        if candidate < out[emitted as usize] {
            out[emitted as usize] = candidate;
        }
        emitted += 1;
        xcoord += 1.0;
    }

    // Fixed-width tail pass 3 (right edge, current line).
    while emitted < n {
        let candidate =
            f32_of(f32_of((xcoord * slope_num + intercept_num) / denominator) - f32_of(offset));
        if candidate < out[emitted as usize] {
            out[emitted as usize] = candidate;
        }
        emitted += 1;
        xcoord += 1.0;
    }
    Ok(out)
}

/// Port the short/mode-0 branch of reference peak suppression
/// (Python `wwise_psy_peak_suppress`).
#[allow(clippy::needless_range_loop)]
pub fn wwise_psy_peak_suppress(
    original: &[f64],
    difference: &[f64],
    look: &WwisePsyLook,
) -> Result<Vec<f64>, AnalysisError> {
    if original.len() != difference.len() {
        return Err(AnalysisError::PsyCurveLengthMismatch {
            want: original.len() as i64,
            got: difference.len() as i64,
        });
    }
    if original.len() as i64 != look.n {
        return Err(AnalysisError::PsyLookCurveLengthMismatch {
            look_n: look.n,
            got: original.len() as i64,
        });
    }
    let n = look.n as usize;
    let limit = (look.short_limit.min(look.n)) as usize;
    let mut smooth = vec![0.0f64; n];
    let mut peak_floor = vec![0.0f64; n];
    for i in 0..((limit + 4).min(n)) {
        let value = original[i];
        smooth[i] = f32_of(if value >= -70.0 {
            value
        } else {
            (value + 70.0) * 0.1 - 70.0
        });
    }
    let threshold = if (n as i64) != 256 { 9.0 } else { 15.0 };
    let mut center = 4usize;
    while (center as i64) < (limit as i64) {
        if !(original[center - 1] < original[center] && original[center + 1] < original[center]) {
            center += 1;
            continue;
        }
        let mut left = center - 1;
        let left_floor = center - 3;
        let mut cursor = left;
        while cursor > left_floor && original[cursor] <= original[cursor + 1] {
            left = cursor;
            cursor -= 1;
        }
        let mut right = center + 1;
        let right_limit = center + 4;
        while right < right_limit && original[right] <= original[right - 1] {
            right += 1;
        }
        right -= 1;

        let mut delta = (smooth[center] - smooth[left]).max(smooth[center] - smooth[right]);
        if delta <= threshold {
            center = right + 1;
            continue;
        }
        if difference[center] < original[center] {
            // MSVC materializes delta-threshold in a float local, then
            // multiplies by the promoted float literal whose exact double
            // value is 0x3fe3333340000000 (0.6000000238418579).
            delta = f32_of(delta - threshold);
            delta = f32_of(delta * 0.6000000238418579);
        }
        for i in left..=right {
            peak_floor[i] = 0.0f64.max(peak_floor[i].max(f32_of(delta)));
        }
        // The reference encoder resumes after the whole local peak interval
        // rather than testing nested centers inside it a second time.
        center = right + 1;
    }
    let mut out: Vec<f64> = difference.iter().map(|v| f32_of(*v)).collect();
    for i in 3..limit {
        let cap = (look.envelope[i] as f64)
            .min(look.second_envelope[i] as f64 + (look.second_envelope[0] as f64).abs());
        if cap < peak_floor[i] {
            peak_floor[i] = cap;
        }
        out[i] = f32_of(out[i] - peak_floor[i]);
    }
    Ok(out)
}

/// Port the long / mode-2 peak suppression branch
/// (Python `wwise_psy_peak_suppress_long_mode2`).
#[allow(clippy::needless_range_loop)]
pub fn wwise_psy_peak_suppress_long_mode2(
    base_curve: &[f64],
    tables: &WwisePsyLongTables,
) -> Result<Vec<f64>, AnalysisError> {
    if base_curve.len() as i64 != LONG_PSY_N || tables.n != LONG_PSY_N {
        return Err(AnalysisError::LongRemapMode2Bins { want: LONG_PSY_N });
    }
    // In the stored long look this is exactly the active-bin field in peak
    // suppression (the shared long-look word at index 23).
    let active_bins = tables.seed_outer_u32[23] as i64;
    if !(active_bins > 0 && active_bins <= LONG_PSY_N && active_bins % 8 == 0) {
        return Err(AnalysisError::LongActiveSpanInvalid {
            active: active_bins,
        });
    }
    // peak suppression spills each eight-bin mean to a float stack slot.
    let mut means = Vec::with_capacity((active_bins / 8) as usize);
    for offset in (0..active_bins).step_by(8) {
        let mut sum = 0.0f64;
        for value in &base_curve[offset as usize..(offset + 8) as usize] {
            sum += *value;
        }
        means.push(f32_of(sum * 0.125));
    }
    let mut out: Vec<f64> = base_curve.iter().map(|v| f32_of(*v)).collect();
    let cap_curve = &tables.analysis_curves[1];
    let cap_bias = (cap_curve[0] as f64).abs();

    for i in 2..(means.len() - 2) {
        if !(means[i] < means[i + 1] && means[i + 2] < means[i + 1]) {
            continue;
        }
        let (local_floor, start) = if means[i - 1] >= means[i] {
            (means[i], (i - 1) * 8)
        } else {
            (means[i - 1], (i - 2) * 8)
        };
        let mut reduction = means[i + 1] - local_floor;
        if reduction <= 2.0 {
            continue;
        }
        let peak_bin = (i + 1) * 8;
        let cap = (tables.analysis_field_19_curve[peak_bin] as f64)
            .min(cap_curve[peak_bin] as f64 + cap_bias);
        reduction = (reduction - 2.0).min(cap);
        let end = (out.len() - 1).min((i + 4) * 8);
        for bin_index in start..=end {
            out[bin_index] = f32_of(out[bin_index] - reduction);
        }
    }
    Ok(out)
}

/// Build the long remap with the selected immutable analysis profile
/// (Python `build_long_psy_remap_variant`).
pub fn build_long_psy_remap_variant(
    raw: &[f64],
    mode: i64,
    table: &WwisePsyLongTables,
) -> Result<LongRemapResult, AnalysisError> {
    if raw.len() as i64 != 1024 {
        return Err(AnalysisError::LongRemapBins { want: 1024 });
    }
    if table.n != 1024 {
        return Err(AnalysisError::LongRemapBins { want: 1024 });
    }
    let original: Vec<f64> = raw.iter().map(|v| f32_of(*v)).collect();
    let first = wwise_psy_curve_smooth(
        &original,
        &table
            .analysis_interval_u32
            .iter()
            .map(|v| *v as i64)
            .collect::<Vec<i64>>(),
        140.0,
        -1,
    )?;
    let residual: Vec<f64> = original
        .iter()
        .zip(first.iter())
        .map(|(value, smooth)| f32_of(*value - smooth))
        .collect();
    let selector = wwise_psy_curve_smooth(
        &residual,
        &table
            .analysis_interval_u32
            .iter()
            .map(|v| *v as i64)
            .collect::<Vec<i64>>(),
        0.0,
        100,
    )?;
    let base_before: Vec<f64> = original
        .iter()
        .zip(residual.iter())
        .map(|(value, remainder)| f32_of(*value - remainder))
        .collect();
    let base = if mode == 2 {
        wwise_psy_peak_suppress_long_mode2(&base_before, table)?
    } else {
        base_before.clone()
    };
    let lut: Vec<f64> = table
        .analysis_profile_u32
        .get(84..124)
        .ok_or(AnalysisError::LongRemapLutShort {
            got: table.analysis_profile_u32.len() as i64,
        })?
        .iter()
        .map(|word| u32_to_f32(*word) as f64)
        .collect();
    let remap: Vec<f64> = base
        .iter()
        .zip(selector.iter())
        .map(|(value, choice)| {
            let idx = ((*choice + 0.5) as i64).clamp(0, 39) as usize;
            f32_of(*value + lut[idx])
        })
        .collect();
    Ok(LongRemapResult {
        first_smooth: first,
        residual,
        selector,
        base_before_peak: base_before,
        base,
        remap,
    })
}

/// Run the local 1024-bin long mode-2 remap front end
/// (Python `build_long_psy_remap_mode2`).
pub fn build_long_psy_remap_mode2(
    original: &[f64],
    tables: &WwisePsyLongTables,
) -> Result<LongRemapResult, AnalysisError> {
    build_long_psy_remap_variant(original, 2, tables)
}

/// Build the remap offset curve from base and selector buffers
/// (Python `wwise_psy_row3_curve`).
pub fn wwise_psy_row3_curve(
    base_curve: &[f64],
    selector_curve: &[f64],
    curve_offsets: &[i64],
) -> Result<Vec<f64>, AnalysisError> {
    if base_curve.len() != selector_curve.len() {
        return Err(AnalysisError::PsyBaseSelectorLengthMismatch {
            want: base_curve.len() as i64,
            got: selector_curve.len() as i64,
        });
    }
    Ok(base_curve
        .iter()
        .zip(selector_curve.iter())
        .map(|(base, selector)| {
            let idx = ((*selector + 0.5) as i64).clamp(0, 39) as usize;
            f32_of(*base + curve_offsets[idx] as f64)
        })
        .collect())
}

/// Stage the reference remap as (first_smooth, selector, base)
/// (Python `wwise_psy_residual_core`).
#[allow(clippy::type_complexity)]
pub fn wwise_psy_residual_core(
    original: &[f64],
    look: &WwisePsyLook,
) -> Result<(Vec<f64>, Vec<f64>, Vec<f64>), AnalysisError> {
    if original.len() as i64 != look.n {
        return Err(AnalysisError::PsyLookCurveLengthMismatch {
            look_n: look.n,
            got: original.len() as i64,
        });
    }
    let first = wwise_psy_curve_smooth(original, &look.interval_table, 140.0, 0)?;
    let difference: Vec<f64> = original
        .iter()
        .zip(first.iter())
        .map(|(a, b)| f32_of(a - b))
        .collect();
    let selector = wwise_psy_curve_smooth(
        &difference,
        &look.interval_table,
        0.0,
        look.noise_fixed_window,
    )?;
    let mut base: Vec<f64> = original
        .iter()
        .zip(difference.iter())
        .map(|(a, b)| f32_of(a - b))
        .collect();
    base = wwise_psy_peak_suppress(original, &base, look)?;
    Ok((first, selector, base))
}

/// Apply the remap q-row extension
/// (Python `wwise_psy_apply_q_extension`).
#[allow(clippy::too_many_arguments)]
pub fn wwise_psy_apply_q_extension(
    base_curve: &[f64],
    selector_curve: &[f64],
    q: f64,
    low_extension: Option<&[f64]>,
    high_extension: Option<&[f64]>,
) -> Result<Vec<f64>, AnalysisError> {
    if base_curve.len() != selector_curve.len() {
        return Err(AnalysisError::PsyBaseSelectorLengthMismatch {
            want: base_curve.len() as i64,
            got: selector_curve.len() as i64,
        });
    }
    let low_scalar = 0.0;
    let high_scalar = 0.0;
    if let Some(low) = low_extension {
        if (low.len() as i64) < 40 {
            return Err(AnalysisError::PsyBaseSelectorLengthMismatch {
                want: 40,
                got: low.len() as i64,
            });
        }
    }
    if let Some(high) = high_extension {
        if (high.len() as i64) < 40 {
            return Err(AnalysisError::PsyBaseSelectorLengthMismatch {
                want: 40,
                got: high.len() as i64,
            });
        }
    }
    let mut output = Vec::with_capacity(base_curve.len());
    let q_limit = base_curve.len() / 3;
    for (i, (base, selector)) in base_curve.iter().zip(selector_curve.iter()).enumerate() {
        let index = ((*selector + 0.5) as i64).clamp(0, 39) as usize;
        let low = low_extension.map_or(low_scalar, |t| t[index]);
        let high = high_extension.map_or(high_scalar, |t| t[index]);
        if q > 0.0 && i < q_limit {
            output.push(f32_of(low + *base - (low - high) * q));
        } else {
            output.push(f32_of(low + *base));
        }
    }
    Ok(output)
}

/// Run the confirmed remap front half for one short curve
/// (Python `build_psy_remap`).
///
/// Returns `(first_smooth, selector_smooth, base, noise_mask, noise_mask)`;
/// item 3 is the buffer passed as `remapped spectrum`.
#[allow(clippy::type_complexity)]
pub fn build_psy_remap(
    original: &[f64],
    _q: f64,
    look: &WwisePsyLook,
    curve_offsets: &[i64],
) -> Result<(Vec<f64>, Vec<f64>, Vec<f64>, Vec<f64>, Vec<f64>), AnalysisError> {
    let (first, selector, base) = wwise_psy_residual_core(original, look)?;
    // ``q`` and the extension fields are retained in the signature because
    // their separate scratch role is still modelled; they do not select
    // remapped spectrum.
    let noise_mask = wwise_psy_row3_curve(&base, &selector, curve_offsets)?;
    let noise_mask_copy = noise_mask.clone();
    Ok((first, selector, base, noise_mask, noise_mask_copy))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn signed_hi16_oracle() {
        assert_eq!(wwise_signed_hi16(0x0001_0000), 1);
        assert_eq!(wwise_signed_hi16(0xFFFF_0000), -1);
        // high of 0x00FF_FFFF is 0x00FF = 255 (positive)
        assert_eq!(wwise_signed_hi16(0x00FF_FFFF), 255);
        assert_eq!(wwise_lo16(0x0001_00FF), 0x00FF);
    }

    #[test]
    fn smooth_rejects_short_interval_table() {
        let curve = vec![1.0f64; 8];
        let table = vec![0i64];
        assert!(wwise_psy_curve_smooth(&curve, &table, 140.0, 0).is_err());
    }
}
