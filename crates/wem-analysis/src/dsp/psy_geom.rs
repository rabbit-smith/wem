//! Deterministic psychoacoustic geometry materializer.
//!
//! These builders preserve reference x87 rounding and comparison boundaries;
//! their output is locked bit-for-bit by `tests/psy_geom_parity.rs`. Function
//! and local names describe the data being built, keeping provenance
//! vocabulary out of caller-facing names.

use super::crt90::{ciatan, ciexp, cilog};
use super::psy_geom_data::{
    ATH_OFF_BITS, ATH_SOURCE_CURVE, AXIS, EIGHTH_BITS, EPS_F64_BITS, HALF_BITS, LN2_BITS,
    LOG2E_BITS, MASK_POOL, PSYCHO_BITS, QUARTER_BITS, SIX_F64_BITS, TWO_BITS,
};
use super::x87::{f32_bits, f32_round};
use crate::config::AnalysisError;

#[inline]
const fn d(bits: u64) -> f64 {
    f64::from_bits(bits)
}

const LOG2E: f64 = d(LOG2E_BITS);
const PSYCHO: f64 = d(PSYCHO_BITS);
const LN2: f64 = d(LN2_BITS);
const HALF: f64 = d(HALF_BITS);
const QUARTER: f64 = d(QUARTER_BITS);
const ATH_OFF: f64 = d(ATH_OFF_BITS);
const TWO: f64 = d(TWO_BITS);
const EIGHTH: f64 = d(EIGHTH_BITS);

#[inline]
fn e60(i: usize) -> f64 {
    f32::from_bits(ATH_SOURCE_CURVE[i]) as f64
}

/// Truncation toward zero (Python `int(x)`, finite builder inputs).
#[inline]
fn trunc_zero(x: f64) -> i64 {
    x as i64
}

/// Python `_floor_integer`: int-truncate with the below-correction.
#[inline]
fn floor_integer(x: f64) -> i64 {
    let v = x as i64;
    if (v as f64) > x {
        v - 1
    } else {
        v
    }
}

/// Materialize the logarithmic group label for every spectrum bin.
/// Arithmetic order and truncation are part of the byte identity.
pub fn octave(
    spectrum_bins: u32,
    sample_rate: u32,
    octave_shift: u32,
) -> Result<Vec<i32>, AnalysisError> {
    validate_geometry(spectrum_bins, sample_rate)?;
    let shift = octave_shift
        .checked_add(1)
        .ok_or(AnalysisError::UnsupportedGeometry {
            reason: "octave shift is too large",
        })?;
    let scale = 1i64
        .checked_shl(shift)
        .ok_or(AnalysisError::UnsupportedGeometry {
            reason: "octave shift is too large",
        })? as f64;
    let bin_width = (sample_rate as f64) / (spectrum_bins as f64);
    Ok((0..spectrum_bins)
        .map(|bin| {
            let frequency = ((f64::from(bin) + QUARTER) * 0.5) * bin_width;
            let label = ((cilog(frequency) * LOG2E) - PSYCHO) * scale + HALF;
            trunc_zero(label) as i32
        })
        .collect())
}

/// Materialize the absolute hearing threshold curve as stored f32 bits.
/// Each of the 87 source segments uses a separately rounded f32 ramp; the
/// final segment is extrapolated with the last stored delta.
pub fn ath(spectrum_bins: u32, sample_rate: u32) -> Result<Vec<u32>, AnalysisError> {
    validate_geometry(spectrum_bins, sample_rate)?;
    let bin_count = spectrum_bins as i64;
    let sample_rate = sample_rate as f64;
    let mut curve = vec![0.0; spectrum_bins as usize];
    let mut cursor = 0i64;
    for segment in 0..87i64 {
        let exponent = ((((segment as f64) + 1.0) * EIGHTH) - TWO) + PSYCHO;
        let frequency = ciexp(exponent * LN2);
        let segment_end =
            floor_integer(((2.0f64 * frequency) * (bin_count as f64) / sample_rate) + HALF);
        let segment_value = f32_round(e60(segment as usize));
        if cursor < segment_end {
            // The slope uses the full segment even when writes stop at the
            // spectrum boundary.
            let slope = f32_round(
                (e60(segment as usize + 1) - segment_value) / ((segment_end - cursor) as f64),
            );
            let mut value = segment_value;
            let write_end = segment_end.min(bin_count);
            while cursor < write_end {
                curve[cursor as usize] = f32_round(value + ATH_OFF);
                value = f32_round(value + slope);
                cursor += 1;
            }
        }
    }
    if cursor < bin_count {
        if cursor < 2 {
            return Err(AnalysisError::UnsupportedGeometry {
                reason: "ATH tail requires two materialized bins",
            });
        }
        let mut value = curve[(cursor - 1) as usize];
        let slope = f32_round(value - curve[(cursor - 2) as usize]);
        for bin in cursor..bin_count {
            curve[bin as usize] = value;
            value = f32_round(value + slope);
        }
    }
    Ok(curve.iter().map(|value| f32_bits(*value)).collect())
}

/// Materialize the packed lower and upper smoothing endpoints for every bin.
/// The two cursors persist across bins because the mapped frequency is
/// monotonic. Extended-precision helpers preserve the reference comparisons.
pub fn interval_table(
    spectrum_bins: u32,
    sample_rate: u32,
    lower_span: i64,
    upper_span: i64,
) -> Result<Vec<u32>, AnalysisError> {
    validate_geometry(spectrum_bins, sample_rate)?;
    use super::x87::{add80, cmp_f64, ge_f64, mul80, F80};
    // mapped() constants (verified against the paired build)
    const K_SQ: f64 = 1.8499999754340024e-08;
    const K_FIRST: f64 = 2.240000009536743;
    const K_LIN: f64 = 0.0007399999885819852;
    const K_SEC: f64 = 13.100000381469727;
    const K_MIC: f64 = 9.999999747378752e-05;

    #[inline]
    fn fi(x: i64) -> F80 {
        debug_assert!(
            x.unsigned_abs() < (1u64 << 53),
            "int must be exact in binary64"
        );
        F80::from_f64(x as f64)
    }

    #[inline]
    fn m80(a: F80, b: f64) -> F80 {
        mul80(a, F80::from_f64(b))
    }

    #[inline]
    fn mapped(linear: i64, squared: i64) -> F80 {
        let x = F80::from_f64(f32_round(linear as f64));
        let arg = f32_round(m80(fi(squared), K_SQ).to_f64());
        let first = m80(F80::from_f64(f32_round(ciatan(arg))), K_FIRST);
        let arg = f32_round(m80(x, K_LIN).to_f64());
        let second = f32_round(ciatan(arg));
        add80(
            add80(m80(F80::from_f64(second), K_SEC), first),
            m80(x, K_MIC),
        )
    }

    let bin_count = spectrum_bins as i64;
    let mut lower_cursor = -99i64;
    let mut upper_cursor = 1i64;
    let integer_bin_width = (sample_rate as i64) / (2 * bin_count);
    let bin_width_squared = integer_bin_width * integer_bin_width;
    let mut linear_frequency = 0i64;
    let mut squared_frequency_step = 0i64;
    let mut intervals = Vec::with_capacity(spectrum_bins as usize);
    for bin in 0..bin_count {
        let center = f32_round(mapped(linear_frequency, bin * squared_frequency_step).to_f64());
        if lower_span + lower_cursor < bin {
            let mut bounded_cursor = lower_span + lower_cursor;
            let mut cursor_frequency = lower_cursor * integer_bin_width;
            let mut cursor_squared_step = lower_cursor * bin_width_squared;
            loop {
                if ge_f64(
                    mapped(cursor_frequency, lower_cursor * cursor_squared_step),
                    center - 0.5,
                ) {
                    break;
                }
                cursor_squared_step += bin_width_squared;
                lower_cursor += 1;
                cursor_frequency += integer_bin_width;
                bounded_cursor += 1;
                if bounded_cursor >= bin {
                    break;
                }
            }
        }
        let mut upper_endpoint = upper_cursor;
        if upper_cursor <= bin_count {
            let mut cursor_frequency = upper_cursor * integer_bin_width;
            let mut cursor_squared_step = upper_cursor * bin_width_squared;
            loop {
                if upper_endpoint >= bin + upper_span
                    && matches!(
                        cmp_f64(
                            mapped(cursor_frequency, upper_cursor * cursor_squared_step),
                            0.5 + center,
                        ),
                        std::cmp::Ordering::Greater | std::cmp::Ordering::Equal
                    )
                {
                    break;
                }
                cursor_squared_step += bin_width_squared;
                cursor_frequency += integer_bin_width;
                upper_endpoint += 1;
                upper_cursor = upper_endpoint;
                if upper_endpoint > bin_count {
                    break;
                }
            }
        }
        linear_frequency += integer_bin_width;
        intervals.push(((lower_cursor << 16) + upper_endpoint - 65537) as u32);
        squared_frequency_step += bin_width_squared;
    }
    Ok(intervals)
}

fn validate_geometry(spectrum_bins: u32, sample_rate: u32) -> Result<(), AnalysisError> {
    if spectrum_bins < 2 {
        return Err(AnalysisError::UnsupportedGeometry {
            reason: "psychoacoustic geometry needs at least two spectrum bins",
        });
    }
    if sample_rate == 0 {
        return Err(AnalysisError::UnsupportedGeometry {
            reason: "psychoacoustic geometry needs a positive sample rate",
        });
    }
    Ok(())
}

/// Default fractional quality index used by the geometry profile.
/// The reference value is approximately 6.000000894, so interpolation is
/// required even though it is very close to row 6.
pub fn default_quality_index() -> f64 {
    use super::x87::{add80, F80};
    const EPS: f64 = f64::from_bits(EPS_F64_BITS);
    let q = f32_round(4.0 / 10.0);
    let q = f32_round(add80(F80::from_f64(q), F80::from_f64(EPS)).to_f64());
    let mut segment = 0usize;
    while segment < 12 {
        if AXIS[segment] as f64 <= q && q <= AXIS[segment + 1] as f64 {
            break;
        }
        segment += 1;
    }
    assert!(segment < 12, "quality outside interpolation axis");
    let lo = AXIS[segment] as f64;
    let hi = AXIS[segment + 1] as f64;
    let fraction = exact_div80(q - lo, hi - lo);
    let segment = segment as i64;
    add80(
        F80::from_f64(segment as f64),
        F80::from_f64(f32_round(fraction)),
    )
    .to_f64()
}

/// Exact `Fraction(a)/Fraction(b)` then `_round80`-equivalent single round:
/// numerator/denominator are exact binary rationals here, so the division
/// result equals f64-correct rounding of the register quotient. Python does
/// rational-exact then rounds once to 80 bits; for two binary64 operands the
/// exact quotient's 80-bit round, stored via fstps to f32, equals direct
/// IEEE double division followed by the same f32 round — asserted by parity.
fn exact_div80(a: f64, b: f64) -> f64 {
    a / b
}

/// Interpolate three adjacent quality rows and apply the first-value-plus-six
/// floor clamp. The returned f64 values lie exactly on the f32 grid.
pub(crate) fn mask_knots(raw: &[u32], index: f64) -> Result<[Vec<f64>; 3], AnalysisError> {
    use super::x87::{add80, mul80, F80};
    const SIX: f64 = f64::from_bits(SIX_F64_BITS);
    if !index.is_finite() || index < 0.0 || index > raw.len() as f64 {
        return Err(AnalysisError::MalformedField {
            reason: "mask quality index must be finite and non-negative",
        });
    }
    let quality_row = index as i64;
    debug_assert!((quality_row as f64) <= index, "floor semantics expected");
    let integer = |x: i64| F80::from_f64(x as f64);
    let fraction = add80(F80::from_f64(index), integer(-quality_row));
    let complement = add80(F80::from_f64(1.0), -fraction);
    let mut rows = [Vec::new(), Vec::new(), Vec::new()];
    for (row, output) in rows.iter_mut().enumerate() {
        let mut values = Vec::with_capacity(17);
        for j in 0..17 {
            let offset = (quality_row * 51 + row as i64 * 17 + j as i64) as usize;
            let left = (*raw.get(offset).ok_or(AnalysisError::MalformedField {
                reason: "mask bank is shorter than the selected quality rows",
            })? as i32) as i64;
            let right = (*raw.get(offset + 51).ok_or(AnalysisError::MalformedField {
                reason: "mask bank is shorter than the selected quality rows",
            })? as i32) as i64;
            values.push(f32_round(
                add80(
                    mul80(integer(left), complement),
                    mul80(integer(right), fraction),
                )
                .to_f64(),
            ));
        }
        let floor = f32_round(add80(F80::from_f64(values[0]), F80::from_f64(SIX)).to_f64());
        *output = values
            .iter()
            .map(|v| {
                let r = *v;
                if r < floor {
                    floor
                } else {
                    r
                }
            })
            .collect();
    }
    Ok(rows)
}

/// Interpolate three knot rows at each spectrum-bin center.
pub(crate) fn mask_curves_knots(
    spectrum_bins: u32,
    sample_rate: u32,
    knots: &[Vec<f64>; 3],
) -> Result<[Vec<f64>; 3], AnalysisError> {
    validate_geometry(spectrum_bins, sample_rate)?;
    if knots.iter().any(|row| row.len() < 17) {
        return Err(AnalysisError::MalformedField {
            reason: "mask curve needs seventeen knots per row",
        });
    }
    let mut rows: [Vec<f64>; 3] = [Vec::new(), Vec::new(), Vec::new()];
    for r in rows.iter_mut() {
        r.reserve(spectrum_bins as usize);
    }
    for bin in 0..spectrum_bins {
        let (knot, fraction, complement) = bin_knot_position(spectrum_bins, sample_rate, bin);
        for (row, out) in knots.iter().zip(rows.iter_mut()) {
            out.push(interpolate_knot(row, knot, fraction, complement));
        }
    }
    Ok(rows)
}

/// Alias for the LONG wrappers: three-row lerp over given knots.
pub(crate) fn curve_rows(
    spectrum_bins: u32,
    sample_rate: u32,
    knots: &[Vec<f64>; 3],
) -> Result<[Vec<f64>; 3], AnalysisError> {
    mask_curves_knots(spectrum_bins, sample_rate, knots)
}

/// Single-row variant of the same bin-center interpolation.
pub(crate) fn curve_row1(
    spectrum_bins: u32,
    sample_rate: u32,
    row: &[f64],
) -> Result<Vec<f64>, AnalysisError> {
    validate_geometry(spectrum_bins, sample_rate)?;
    if row.len() < 17 {
        return Err(AnalysisError::MalformedField {
            reason: "mask curve needs seventeen knots",
        });
    }
    Ok((0..spectrum_bins)
        .map(|bin| {
            let (knot, fraction, complement) = bin_knot_position(spectrum_bins, sample_rate, bin);
            interpolate_knot(row, knot, fraction, complement)
        })
        .collect())
}

/// Bin-center position and knot weights shared by every curve materializer.
fn bin_knot_position(
    spectrum_bins: u32,
    sample_rate: u32,
    bin: u32,
) -> (usize, f64, super::x87::F80) {
    use super::x87::{add80, mul80, F80};
    let logarithm =
        cilog((f64::from(bin) + HALF) * f64::from(sample_rate) / f64::from(2 * spectrum_bins));
    let position = f32_round(
        mul80(
            add80(
                mul80(F80::from_f64(logarithm), F80::from_f64(LOG2E)),
                F80::from_f64(-PSYCHO),
            ),
            F80::from_f64(2.0),
        )
        .to_f64(),
    );
    let position = position.clamp(0.0, 16.0);
    let knot = position as usize;
    debug_assert!((knot as f64) <= position, "trunc toward zero expected");
    let fraction =
        f32_round(add80(F80::from_f64(position), F80::from_f64(-(knot as f64))).to_f64());
    let complement = add80(F80::from_f64(1.0), F80::from_f64(-fraction));
    (knot, fraction, complement)
}

/// Interpolate one knot pair. The final knot has no following value and its
/// fraction is zero, so the zero fallback is never observable.
#[inline]
fn interpolate_knot(row: &[f64], knot: usize, fraction: f64, complement: super::x87::F80) -> f64 {
    use super::x87::{add80, mul80, F80};
    let right = if knot < 16 { row[knot + 1] } else { 0.0 };
    f32_round(
        add80(
            mul80(F80::from_f64(right), F80::from_f64(fraction)),
            mul80(F80::from_f64(row[knot]), complement),
        )
        .to_f64(),
    )
}

/// Three mask rows flattened in row-major order as f32 bit patterns.
pub fn mask_curves(spectrum_bins: u32, sample_rate: u32) -> Result<Vec<u32>, AnalysisError> {
    let knots = mask_knots(MASK_POOL, default_quality_index())?;
    Ok(mask_curves_knots(spectrum_bins, sample_rate, &knots)?
        .iter()
        .flatten()
        .map(|v| f32_bits(*v))
        .collect())
}

/// Middle mask row as f32 bit patterns.
pub fn mask_curve(spectrum_bins: u32, sample_rate: u32) -> Result<Vec<u32>, AnalysisError> {
    let knots = mask_knots(MASK_POOL, default_quality_index())?;
    Ok(mask_curves_knots(spectrum_bins, sample_rate, &knots)?[1]
        .iter()
        .map(|v| f32_bits(*v))
        .collect())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tail_guard_and_shape() {
        let v = ath(128, 44100).unwrap();
        assert_eq!(v.len(), 128);
        let o = octave(128, 44100, 5).unwrap();
        assert_eq!(o.len(), 128);
        assert!(o.iter().all(|&x| (-64..=1024).contains(&x)));
        let iv = interval_table(128, 44100, 3, 3).unwrap();
        assert_eq!(iv.len(), 128);
    }

    #[test]
    fn mask_chain_shapes() {
        assert!((default_quality_index() - 6.000000894).abs() < 1e-6);
        assert_eq!(mask_curve(128, 44100).unwrap().len(), 128);
        assert_eq!(mask_curves(128, 48000).unwrap().len(), 384);
    }

    #[test]
    fn rejects_invalid_public_geometry() {
        assert!(ath(0, 44100).is_err());
        assert!(octave(128, 0, 5).is_err());
        assert!(interval_table(1, 44100, 3, 3).is_err());
        assert!(mask_curve(128, 0).is_err());
    }
}
