//! Geometry-materializer surfaces `ath` and `octave` of analysis_geometry_builder.
//!
//! Kernel-side port of the Python builder reference
//! (`corpus/paired-build/the round/src/builders/short_seed.py`); parity is
//! locked bit-for-bit by `tests/psy_geom_parity.rs` (oracle: the Python
//! builder, itself cross-checked against the registered 6ch bytes at export
//! time). VA annotations follow the reference.

use super::crt90::{ciatan, ciexp, cilog};
use super::psy_geom_data::{
    ATH_OFF_BITS, AXIS, EIGHTH_BITS, EPS_F64_BITS, paired_ath_source_curve, HALF_BITS, LN2_BITS, LOG2E_BITS,
    MASK_POOL, PSYCHO_BITS, QUARTER_BITS, SIX_F64_BITS, TWO_BITS,
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
            let slope = f32_round((e60(v42 as usize + 1) - v46) / ((v13 - v11) as f64));
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

/// `a1[6]` — the build's code..the build's code, packed interval endpoints.
///
/// `((v57 << 16) + v28 - 65537)` per bin; cursors persist across bins;
/// thresholds ride the exact-register `mapped()` chain (x87 mul80/add80 +
/// carrier `_CIatan`). `a2_112/a2_116` = 0.5f static-seed pair (r7 byte
/// proof, fixed here as HALF), `a2_120/a2_124` = setter-chain conditional
/// 3/3 adjudicated by the 6ch byte gate (R-R6 structural selection —
/// explicit parameters, no fitting).
pub fn interval_table(a4: u32, a5: u32, a2_120: i64, a2_124: i64) -> Vec<u32> {
    use super::x87::{add80, cmp_f64, ge_f64, mul80, F80};
    // mapped() constants: paired_tail_end/F0/E8/E0/D8 (constants.json, root-verified)
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

    // the build's code..the build's code (repeated at 1001657d..100165e3, 10016658..100166bf)
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

    let a4i = a4 as i64;
    let mut v57: i64 = -99; // the build's code persistent cursors
    let mut v58: i64 = 1;
    let v26 = (a5 as i64) / (2 * a4i); // the build's code.. signed integer division
    let v55 = v26 * v26;
    let mut v68: i64 = 0;
    let mut v65: i64 = 0;
    let mut out = Vec::with_capacity(a4 as usize);
    for v43 in 0..a4i {
        // the build's code: fstp f32 store of the mapped value
        let v92 = f32_round(mapped(v68, v43 * v65).to_f64());
        if a2_120 + v57 < v43 {
            // the build's code / the build's code
            let mut v75 = a2_120 + v57;
            let mut v63 = v57 * v26;
            let mut v60 = v57 * v55;
            loop {
                // the build's code fsub threshold (exact binary64); 100165ec ordered >=
                if ge_f64(mapped(v63, v57 * v60), v92 - 0.5) {
                    break;
                }
                v60 += v55;
                v57 += 1;
                v63 += v26;
                v75 += 1;
                if v75 >= v43 {
                    break; // the build's code strict < continues
                }
            }
        }
        let mut v28 = v58; // the build's code: v58 persists past the upper bound
        if v58 <= a4i {
            let mut v61 = v58 * v26;
            let mut v64 = v58 * v55;
            loop {
                if v28 >= v43 + a2_124 {
                    // the build's code..100166d3: 0.5 + v92 <= mapped (ordered)
                    if matches!(
                        cmp_f64(mapped(v61, v58 * v64), 0.5 + v92),
                        std::cmp::Ordering::Greater | std::cmp::Ordering::Equal
                    ) {
                        break;
                    }
                }
                v64 += v55;
                v61 += v26;
                v28 += 1;
                v58 = v28;
                if v28 > a4i {
                    break; // the build's code inclusive <= continues
                }
            }
        }
        // the build's code..10016719: packed subtraction borrows across halves
        v68 += v26;
        out.push(((v57 << 16) + v28 - 65537) as u32);
        v65 += v55;
    }
    out
}

/// Default gear index through the VBR chain ebd0 -> e2b0 -> ea80
/// (the build's code.., the build's code/ebd9/ebe4, the build's code..e41f). Reference value
/// 6.000000894.. (g = 6, fraction ≈ 2^-20 — not an endpoint hit; the
/// interpolation below is load-bearing).
pub fn default_quality_index() -> f64 {
    use super::x87::{add80, F80};
    const EPS: f64 = f64::from_bits(EPS_F64_BITS); // the build's code
                                                   // the build's code: 4.0f; 10008933: /10; 893b/893f: f32 store before ebd0
    let q = f32_round(4.0 / 10.0);
    // the build's code/ebd9/ebe4: double epsilon add at x87 width, then fstps
    let q = f32_round(add80(F80::from_f64(q), F80::from_f64(EPS)).to_f64());
    // the build's code/e33c..e3c6: segment select over the descriptor axis
    let mut g = 0usize;
    while g < 12 {
        if AXIS[g] as f64 <= q && q <= AXIS[g + 1] as f64 {
            break;
        }
        g += 1;
    }
    assert!(g < 12, "e2b0: q outside axis");
    // the build's code/e407: both endpoints stored as f32
    let lo = AXIS[g] as f64;
    let hi = AXIS[g + 1] as f64;
    // the build's code..e41f: exact-rational divide at 80-bit, fstps, integer add
    let fraction = exact_div80(q - lo, hi - lo);
    let gi = g as i64;
    add80(F80::from_f64(gi as f64), F80::from_f64(f32_round(fraction))).to_f64()
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

/// `a2+132..+332` knot rows — the build's code setter: adjacent quality-row
/// interpolation (g/g+1), per-element f32 store, `first+6` floor clamp.
/// `raw` is the mask_pool_0 word array; returns three f64 rows (f32 grid).
pub fn mask_knots(raw: &[u32], index: f64) -> [Vec<f64>; 3] {
    use super::x87::{add80, mul80, F80};
    const SIX: f64 = f64::from_bits(SIX_F64_BITS); // paired_six_f64
    let g = index as i64; // 1000d94c __ftol2_sse (integral-domain input)
    debug_assert!((g as f64) <= index, "ftol2 floor semantics expected");
    let fi = |x: i64| F80::from_f64(x as f64);
    // 1000d967: fisubl keeps x87 precision
    let frac = add80(F80::from_f64(index), fi(-g));
    // 1000d974/976: complement = 1 - frac (register width)
    let complement = add80(F80::from_f64(1.0), -frac);
    let mut rows = [Vec::new(), Vec::new(), Vec::new()];
    for (row, output) in rows.iter_mut().enumerate() {
        let mut values = Vec::with_capacity(17);
        for j in 0..17 {
            // 1000d96e/d9bc: quality stride 0xcc = 51 words; row stride 0x44
            let at = (g * 51 + row as i64 * 17 + j as i64) as usize;
            let left = (raw[at] as i32) as i64;
            let right = (raw[at + 51] as i32) as i64;
            // d9d6..da98: fildl; separate fmuls; faddp; fstps
            values.push(f32_round(
                add80(mul80(fi(left), complement), mul80(fi(right), frac)).to_f64(),
            ));
        }
        // dac9/dad6/dad8: floor = f32(first + 6.0)
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
    rows
}

/// Bin-center position and three knot lerps — the build's code..the build's code.
pub fn mask_curves_knots(a4: u32, a5: u32, knots: &[Vec<f64>; 3]) -> [Vec<f64>; 3] {
    let mut rows: [Vec<f64>; 3] = [Vec::new(), Vec::new(), Vec::new()];
    for r in rows.iter_mut() {
        r.reserve(a4 as usize);
    }
    for i in 0..a4 {
        let (g, frac, complement) = pos_frac(a4, a5, i);
        for (row, out) in knots.iter().zip(rows.iter_mut()) {
            out.push(lerp_row(row, g, frac, complement));
        }
    }
    rows
}

/// Alias for the LONG wrappers: three-row lerp over given knots.
pub fn curve_rows(a4: u32, a5: u32, knots: &[Vec<f64>; 3]) -> [Vec<f64>; 3] {
    mask_curves_knots(a4, a5, knots)
}

/// Single-row variant (field_18-knot path the build's code..the build's code, where the
/// Python reference passes one row through the same lerp3 loop).
pub fn curve_row1(a4: u32, a5: u32, row: &[f64]) -> Vec<f64> {
    (0..a4)
        .map(|i| {
            let (g, frac, complement) = pos_frac(a4, a5, i);
            lerp_row(row, g, frac, complement)
        })
        .collect()
}

/// Bin-center position and knot weights — the build's code..the build's code, shared by
/// every consumer of the materializer's final loop.
fn pos_frac(a4: u32, a5: u32, i: u32) -> (usize, f64, super::x87::F80) {
    use super::x87::{add80, mul80, F80};
    // 1001681f..82d: (i+0.5)*rate/(2*n), _CIlog
    let logarithm = cilog((f64::from(i) + HALF) * f64::from(a5) / f64::from(2 * a4));
    // 10016832..840: LOG2E, PSYCHO, *2 (register chain), fstps
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
    // 10016844..8d2: clamp [0, 16]
    let position = position.clamp(0.0, 16.0);
    let g = position as usize; // 100168d6 ftol2 of an already-clamped value
    debug_assert!((g as f64) <= position, "trunc toward zero expected");
    let frac = f32_round(add80(F80::from_f64(position), F80::from_f64(-(g as f64))).to_f64());
    let complement = add80(F80::from_f64(1.0), F80::from_f64(-frac));
    (g, frac, complement)
}

/// One knot-pair lerp — 100168fb..1696e (endpoint's following load has zero
/// weight, matching the reference's `g < 16` guard).
#[inline]
fn lerp_row(row: &[f64], g: usize, frac: f64, complement: super::x87::F80) -> f64 {
    use super::x87::{add80, mul80, F80};
    let right = if g < 16 { row[g + 1] } else { 0.0 };
    f32_round(
        add80(
            mul80(F80::from_f64(right), F80::from_f64(frac)),
            mul80(F80::from_f64(row[g]), complement),
        )
        .to_f64(),
    )
}

/// `a1[3]` — the three mask-curve rows flattened to 384 f32 bit patterns
/// (gate ordering). `mask_pool_0` is the family slot array; the export
/// asserted the 6ch and 2ch descriptor copies are identical, so the one
/// generated table serves both geometries.
pub fn mask_curves(a4: u32, a5: u32) -> Vec<u32> {
    let knots = mask_knots(MASK_POOL, default_quality_index());
    mask_curves_knots(a4, a5, &knots)
        .iter()
        .flatten()
        .map(|v| f32_bits(*v))
        .collect()
}

/// `look.mask_curve` — row 1 of `mask_curves` (root-verified byte identity
/// with the registered `short-seed.look.mask_curve`).
pub fn mask_curve(a4: u32, a5: u32) -> Vec<u32> {
    let knots = mask_knots(MASK_POOL, default_quality_index());
    mask_curves_knots(a4, a5, &knots)[1]
        .iter()
        .map(|v| f32_bits(*v))
        .collect()
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
        assert!(o.iter().all(|&x| (-64..=1024).contains(&x)));
        let iv = interval_table(128, 44100, 3, 3);
        assert_eq!(iv.len(), 128);
    }

    #[test]
    fn mask_chain_shapes() {
        assert!((default_quality_index() - 6.000000894).abs() < 1e-6);
        assert_eq!(mask_curve(128, 44100).len(), 128);
        assert_eq!(mask_curves(128, 48000).len(), 384);
    }
}
