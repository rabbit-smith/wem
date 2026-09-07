//! Float32-compatible MDCT bank construction (Python:
//! `wwise_wem/analysis/dsp/transform.py`, `make_mdct_look` and friends).

use crate::config::{f32_round, AnalysisError, MdctLook};

/// Build one MDCT look (Python `make_mdct_look`).
///
/// `static_trig` supplies the reference encoder's stored f32 trig bank. When
/// `None`, the libvorbis analytic construction is used for geometries the
/// Wwise binary has no static bank for (non-profile fallback path).
pub fn make_mdct_look(n: i64, static_trig: Option<&[f32]>) -> Result<MdctLook, AnalysisError> {
    if n < 64 || (n & (n - 1)) != 0 {
        return Err(AnalysisError::MalformedField {
            reason: "MDCT size must be a power of two >= 64",
        });
    }
    let log2n = n.trailing_zeros() as i64;

    let trig: Vec<f32> = match static_trig {
        Some(values) => {
            if values.len() as i64 != n + n / 4 {
                return Err(AnalysisError::MalformedField {
                    reason: "static MDCT trig bank has the wrong length",
                });
            }
            values.to_vec()
        }
        None => {
            // Retain the reference libvorbis construction.
            let mut trig = vec![0.0f32; (n + n / 4) as usize];
            for i in 0..(n / 4) {
                trig[2 * i as usize] =
                    f32_round((std::f64::consts::PI / n as f64 * (4 * i) as f64).cos());
                trig[2 * i as usize + 1] =
                    f32_round(-(std::f64::consts::PI / n as f64 * (4 * i) as f64).sin());
                trig[n as usize / 2 + 2 * i as usize] = f32_round(
                    (std::f64::consts::PI / (2.0 * n as f64) * ((2 * i) as f64 + 1.0)).cos(),
                );
                trig[n as usize / 2 + 2 * i as usize + 1] = f32_round(
                    (std::f64::consts::PI / (2.0 * n as f64) * ((2 * i) as f64 + 1.0)).sin(),
                );
            }
            for i in 0..(n / 8) {
                trig[n as usize + 2 * i as usize] = f32_round(
                    (std::f64::consts::PI / n as f64 * ((4 * i) as f64 + 2.0)).cos() * 0.5,
                );
                trig[n as usize + 2 * i as usize + 1] = f32_round(
                    -(std::f64::consts::PI / n as f64 * ((4 * i) as f64 + 2.0)).sin() * 0.5,
                );
            }
            trig
        }
    };

    let mask = (1i64 << (log2n - 1)) - 1;
    let msb = 1i64 << (log2n - 2);
    let mut bitrev = Vec::with_capacity((n / 4) as usize);
    for i in 0..(n / 8) {
        let mut acc = 0i64;
        for j in 0..log2n {
            if (msb >> j) & i != 0 {
                acc |= 1i64 << j;
            }
        }
        bitrev.push(((!acc) & mask) - 1);
        bitrev.push(acc);
    }
    Ok(MdctLook {
        n,
        log2n,
        trig,
        bitrev,
        scale: f32_round(4.0 / n as f64),
    })
}
