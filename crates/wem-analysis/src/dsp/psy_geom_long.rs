//! Long-block (2048 sample / 1024 bin) psychoacoustic geometry.
//!
//! Both long modes share one materializer and select their mask bank and
//! interval spans from profile-derived constants. Bit parity is pinned by
//! `tests/psy_geom_long_parity.rs`.

use super::psy_geom::{
    ath, curve_row1, curve_rows, default_quality_index, interval_table, mask_knots, octave,
};
use super::psy_geom_long_data::{
    FIELD_KNOTS as AUXILIARY_KNOTS, MASK_BANK_MODE_2, MASK_BANK_MODE_3, SCALAR_AXIS_MODE2,
    SCALAR_AXIS_MODE3,
};
use crate::config::AnalysisError;

const SPECTRUM_BINS: u32 = 1024;
const SAMPLE_RATE: u32 = 44100;

fn bank(mode: u32) -> Result<&'static [u32], AnalysisError> {
    match mode {
        2 => Ok(MASK_BANK_MODE_2),
        3 => Ok(MASK_BANK_MODE_3),
        _ => Err(AnalysisError::geometry(format!(
            "unsupported geometry (reason={:?})",
            "long geometry mode must be 2 or 3"
        ))),
    }
}

fn axis(mode: u32) -> Result<(i64, i64), AnalysisError> {
    let (lo, hi) = match mode {
        2 => SCALAR_AXIS_MODE2,
        3 => SCALAR_AXIS_MODE3,
        _ => {
            return Err(AnalysisError::geometry(format!(
                "unsupported geometry (reason={:?})",
                "long geometry mode must be 2 or 3"
            )))
        }
    };
    Ok((lo as i64, hi as i64))
}

/// Three 1024-element mask rows for the selected long mode.
pub fn curves(mode: u32) -> Result<[Vec<u32>; 3], AnalysisError> {
    let knots = mask_knots(bank(mode)?, default_quality_index())?;
    let rows = curve_rows(SPECTRUM_BINS, SAMPLE_RATE, &knots)?;
    Ok(rows.map(|r| r.iter().map(|&v| f32bits(v)).collect()))
}

/// Auxiliary long-mode curve from the shared 18-knot table.
pub fn auxiliary_curve() -> Result<Vec<u32>, AnalysisError> {
    let row: Vec<f64> = AUXILIARY_KNOTS
        .iter()
        .map(|b| f32::from_bits(*b) as f64)
        .collect();
    Ok(curve_row1(SPECTRUM_BINS, SAMPLE_RATE, &row)?
        .iter()
        .map(|&v| f32bits(v))
        .collect())
}

/// Packed smoothing intervals for the selected long mode.
pub fn interval(mode: u32) -> Result<Vec<u32>, AnalysisError> {
    let (lo, hi) = axis(mode)?;
    interval_table(SPECTRUM_BINS, SAMPLE_RATE, lo, hi)
}

/// Absolute hearing threshold curve for long blocks.
pub fn base_curve() -> Result<Vec<u32>, AnalysisError> {
    ath(SPECTRUM_BINS, SAMPLE_RATE)
}

/// Log-frequency group labels for long blocks.
pub fn group_labels() -> Result<Vec<u32>, AnalysisError> {
    Ok(octave(SPECTRUM_BINS, SAMPLE_RATE, 5)?
        .iter()
        .map(|v| *v as u32)
        .collect())
}

#[inline]
fn f32bits(x: f64) -> u32 {
    (x as f32).to_bits()
}

#[cfg(test)]
mod tests {
    use super::{curves, interval};

    #[test]
    fn rejects_unknown_long_mode() {
        assert!(curves(1).is_err());
        assert!(interval(4).is_err());
    }
}
