//! Geometry-materializer surfaces `ath` and `octave` of analysis_geometry_builder.
//!
//! Kernel-side port of the Python builder reference
//! (`corpus/paired-build/the round/src/builders/short_seed.py`); parity is
//! locked bit-for-bit by `tests/psy_geom_parity.rs` (oracle: the Python
//! builder, itself cross-checked against the registered 6ch bytes at export
//! time). VA annotations follow the reference.

use super::crt90::{ciexp, cilog};
use super::psy_geom_data::{
    ATH_OFF_BITS, EIGHTH_BITS, paired_ath_source_curve, HALF_BITS, LN2_BITS, LOG2E_BITS, PSYCHO_BITS,
    QUARTER_BITS, TWO_BITS,
};
use super::x87::{f32_bits, f32_round};

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
    f32::from_bits(paired_ath_source_curve[i]) as f64
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

/// `a1[5]` — the build's code..the build's code (VERIFIED 128/128 on the corpus gate).
///
/// x87 loop per bin i: `trunc( ((cilog(((i+0.25)*0.5*r)) * LOG2E - PSYCHO) * S + 0.5) )`
/// with `r = a5/a4` (both fild'd as integers), `S = 1 << (a1[8]+1)`.
pub fn octave(a4: u32, a5: u32, a1_8: u32) -> Vec<i32> {
    let s = (1i64 << (a1_8 + 1)) as f64; // shl of 1 by a1[8]+1
    let r = (a5 as f64) / (a4 as f64); // var_18 / var_20 (exact int/uint division)
    (0..a4)
        .map(|i| {
            let arg = ((f64::from(i) + QUARTER) * 0.5) * r;
            let v = ((cilog(arg) * LOG2E) - PSYCHO) * s + HALF;
            trunc_zero(v) as i32
        })
        .collect()
}

/// `a1[4]` — the build's code..the build's code, supplied CRT exp value path.
///
/// 87 source segments (v42 = 0..=86; the build's code compares the incremented
/// index with 0x57), each filled with the f32-stepped ramp
/// `paired_ath_source_curve[v42] + 100 + k*slope`, plus the tail extrapolation at
/// the build's code..the build's code. Returns the f32 bit patterns as stored.
pub fn ath(a4: u32, a5: u32) -> Vec<u32> {
    let a4i = a4 as i64;
    let a5f = a5 as f64;
    let mut buf: Vec<f64> = vec![0.0; a4 as usize];
    let mut v11: i64 = 0;
    for v42 in 0..87i64 {
        // the build's code: v12 = exp(((v42+1)*0.125 - 2 + PSYCHO) * LN2)
        let inner = ((((v42 as f64) + 1.0) * EIGHTH) - TWO) + PSYCHO;
        let v12 = ciexp(inner * LN2);
        // v13 = floor(2*v12*a4/a5 + 0.5) (full value, may exceed a4)
        let t = ((2.0f64 * v12) * (a4i as f64) / a5f) + HALF;
        let v13 = floor_integer(t);
        let v46 = f32_round(e60(v42 as usize));
        if v11 < v13 {
            // slope over the FULL segment length even when stores clamp to a4
            let slope =
                f32_round((e60(v42 as usize + 1) - v46) / ((v13 - v11) as f64));
            let mut run = v46;
            let n = if v13 < a4i { v13 } else { a4i };
            while v11 < n {
                buf[v11 as usize] = f32_round(run + ATH_OFF);
                run = f32_round(run + slope);
                v11 += 1;
            }
        }
    }
    // the build's code..the build's code: repeat last stored value, advance by last diff,
    // f32 store/reload at every addition.
    if v11 < a4i {
        assert!(v11 >= 2, "ATH tail requires two materialized bins");
        let mut run = buf[(v11 - 1) as usize];
        let slope = f32_round(run - buf[(v11 - 2) as usize]);
        for i in v11..a4i {
            buf[i as usize] = run;
            run = f32_round(run + slope);
        }
    }
    buf.iter().map(|x| f32_bits(*x)).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tail_guard_and_shape() {
        let v = ath(128, 44100);
        assert_eq!(v.len(), 128);
        let o = octave(128, 44100, 5);
        assert_eq!(o.len(), 128);
        assert!(o.iter().all(|&x| x >= -64 && x <= 1024));
    }
}
