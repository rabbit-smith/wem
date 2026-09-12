//! LONG (2048-block / 1024-bin) materializer outputs of analysis_geometry_builder.
//!
//! Kernel-side mirror of `builders/long_base.py` + `long_variants.py`: the
//! same geometry materializer engine (`psy_geom`) at `a4 = 1024`, with the
//! mode-selected mask banks (the build's code mode 2 → descriptor+48;
//! the build's code mode 3 → descriptor+4c) and the descriptor+3c scalar-axis
//! pairs feeding the interval cursors (the build's code..d9ac). Registered only
//! for the 6ch geometry, exactly like the Python builder (`build_mode`
//! rejects other keys); 2ch long regeneration stays an the round/the round question.
//!
//! Parity: `tests/psy_geom_long_parity.rs`; the export cross-checks all 13
//! fields against the registered 6ch JSONs before emitting.

use super::psy_geom::{ath, curve_row1, curve_rows, default_quality_index, interval_table, mask_knots, octave};
use super::psy_geom_long_data::{
    FIELD_KNOTS, MASK_BANK_MODE_2, MASK_BANK_MODE_3, SCALAR_AXIS_MODE2, SCALAR_AXIS_MODE3,
};

/// LONG geometry: 1024 bins at the 6ch sample rate (call the build's code).
const A4: u32 = 1024;
const A5: u32 = 44100;

fn bank(mode: u32) -> &'static [u32] {
    match mode {
        2 => MASK_BANK_MODE_2,
        3 => MASK_BANK_MODE_3,
        _ => panic!("LONG mode must be 2 or 3 (descriptor+3c/+48/+4c wiring)"),
    }
}

fn axis(mode: u32) -> (i64, i64) {
    let (lo, hi) = match mode {
        2 => SCALAR_AXIS_MODE2,
        3 => SCALAR_AXIS_MODE3,
        _ => panic!("LONG mode must be 2 or 3"),
    };
    (lo as i64, hi as i64)
}

/// `analysis.curves[0..3]` (and mode-3 `variants[3].curves`): three 1024-
/// element rows from the mode bank through the mode bank + the bin-center lerps.
pub fn curves(mode: u32) -> [Vec<u32>; 3] {
    let knots = mask_knots(bank(mode), default_quality_index());
    let rows = curve_rows(A4, A5, &knots);
    rows.map(|r| r.iter().map(|&v| f32bits(v)).collect())
}

/// `analysis.field_19_curve` — the build's code..the build's code over the DLL knot
/// pair table (18 words: the extra endpoint is never read with nonzero
/// weight, matching the Python "<18f" load).
pub fn field_19(_mode: u32) -> Vec<u32> {
    let row: Vec<f64> = FIELD_KNOTS.iter().map(|b| f32::from_bits(*b) as f64).collect();
    curve_row1(A4, A5, &row)
        .iter()
        .map(|&v| f32bits(v))
        .collect()
}

/// `analysis.interval_u32` — interval engine at 1024 bins with the mode's
/// descriptor+3c scalar pair as the mode-bank cursor integers.
pub fn interval(mode: u32) -> Vec<u32> {
    let (lo, hi) = axis(mode);
    interval_table(A4, A5, lo, hi)
}

/// `seed.base_curve` — same 87-segment ATH writer at 1024 bins
/// (the build's code..the build's code).
pub fn base_curve() -> Vec<u32> {
    ath(A4, A5)
}

/// `seed.group_labels_u32` — bin log-coordinate ladder at 1024 bins
/// (the build's code..the build's code), truncated toward zero like the reference.
pub fn group_labels() -> Vec<u32> {
    octave(A4, A5, 5)
        .iter()
        .map(|v| *v as u32)
        .collect()
}

#[inline]
fn f32bits(x: f64) -> u32 {
    (x as f32).to_bits()
}
