//! Value-semantics ports of the supplied CRT carrier's `_CIatan`, `_CIexp`
//! and `_CIlog` callees (paired 2013.2 runtime helpers).
//!
//! Port of `corpus/paired-build/the round/src/f32.py::ciatan/ciexp/cilog`
//! — the disassembled callees are plain binary64 SSE2 arithmetic, so Rust
//! `f64` maps one-to-one; VA annotations from the reference are preserved.
//! Tables come from `crt90_data.rs` (byte-read from the carrier; the
//! on-disk carrier is Wine-builtin, gate-proven only on the registered
//! inputs — see the CRT adjudication in surface-init-spec.md).
//!
//! Like the reference: signed zeros, subnormals, infinities and NaN
//! quieting are modeled; FPU status flags, errno and user matherr
//! callbacks are NOT emulated.

use super::crt90_data::{CIATAN, CIATAN_BASE, CIEXP, CIEXP_BASE, CILOG, CILOG_BASE};

#[inline]
fn f(b: u64) -> f64 {
    f64::from_bits(b)
}

#[inline]
fn atan_c(va: u64) -> f64 {
    f(CIATAN[((va - CIATAN_BASE) >> 3) as usize])
}

#[inline]
fn exp_c(va: u64) -> f64 {
    f(CIEXP[((va - CIEXP_BASE) >> 3) as usize])
}

#[inline]
fn log_c(va: u64) -> f64 {
    f(CILOG[((va - CILOG_BASE) >> 3) as usize])
}

/// `_CIatan` — SSE2 callee the build's code..the build's code on the carrier.
pub fn ciatan(x: f64) -> f64 {
    let b = x.to_bits(); // the build's code: wrapper fstpl bounds to binary64
    let high = ((b >> 32) & 0x7fff_ffff) as u32;
    let negative = b >> 63;

    // the build's code..100393ee / 10039538..100395f1: large, NaN, tiny.
    if high > 0x440f_ffff {
        if (b & 0x7fff_ffff_ffff_ffff) > 0x7ff0_0000_0000_0000 {
            return x;
        }
        return atan_c(if negative != 0 {
            0x1008_6fa8
        } else {
            0x1008_6fa0
        });
    }
    if high <= 0x3e3f_ffff {
        return x;
    }
    let mut x = x;
    let mut region: i32 = -1;
    if high > 0x3fdb_ffff {
        x = x.abs(); // the build's code..100394f3: fabs and fstpl
        if high <= 0x3ff2_ffff {
            if high <= 0x3fe5_ffff {
                // the build's code..1003952b, preserving each SSE2 operation
                let numerator = x + x;
                let denominator = x + atan_c(0x1008_6fb8);
                let numerator = numerator - atan_c(0x1008_6fb0);
                x = numerator / denominator;
                region = 0;
            } else {
                // the build's code..1003963d
                let t = atan_c(0x1008_6fb0);
                x = (x - t) / (x + t);
                region = 1;
            }
        } else if high <= 0x4003_7fff {
            // the build's code..100395a1
            let fc0 = atan_c(0x1008_6fc0);
            let denominator = x * fc0;
            let numerator = x - fc0;
            let denominator = denominator + atan_c(0x1008_6fb0);
            x = numerator / denominator;
            region = 2;
        } else {
            x = atan_c(0x1008_6fc8) / x; // the build's code..10039661
            region = 3;
        }
    }
    // the build's code..10039449: even polynomial in x^4, times x^2
    let z = x * x;
    let w = z * z;
    let mut even = atan_c(0x1008_6fd0) * w;
    even += atan_c(0x1008_6fd8);
    even *= w;
    even += atan_c(0x1008_6fe0);
    even *= w;
    even += atan_c(0x1008_6fe8);
    even *= w;
    even += atan_c(0x1008_6ff0);
    even *= w;
    even += atan_c(0x1008_6ff8);
    let even = even * z;
    // the build's code..10039485: odd polynomial; subtraction signs are literal
    let mut odd = atan_c(0x1008_7000) * w;
    odd -= atan_c(0x1008_7008);
    odd *= w;
    odd -= atan_c(0x1008_7010);
    odd *= w;
    odd -= atan_c(0x1008_7018);
    odd *= w;
    odd -= atan_c(0x1008_7020);
    let odd = w * odd;
    let correction = (even + odd) * x; // the build's code..1003948d
    if region == -1 {
        return x - correction; // the build's code..10039619
    }
    // the build's code..100394cc: low part, reduced argument, high part, sign
    let correction = correction - atan_c(0x1008_6f60 + 8 * region as u64);
    let correction = correction - x;
    let result = atan_c(0x1008_6f80 + 8 * region as u64) - correction;
    if negative != 0 {
        -result
    } else {
        result
    }
}

/// `_CIexp` → callee 41a40 → fb70 on the carrier (default CRT value path).
pub fn ciexp(x: f64) -> f64 {
    let b = x.to_bits(); // 1000c23a; return store is 1000c264
    let exponent = ((b >> 52) & 0x7ff) as u32;

    if (b & 0x7fff_ffff_ffff_ffff) > 0x7ff0_0000_0000_0000 {
        // 15ce0 class 2; fcc0 default callback returns the argument, fld quiets
        return f(b | 0x0008_0000_0000_0000);
    }
    if exponent < 0x3c9 {
        return x + exp_c(0x1008_7110); // 1000fe38: 1+x (zeros/subnormals)
    }
    if exponent > 0x408 {
        if b == 0xfff0_0000_0000_0000 {
            return 0.0; // 1000fd0c..fd16: fldz
        }
        if exponent == 0x7ff {
            return x + exp_c(0x1008_7110); // +inf -> 1000fe38
        }
        // fd2f/ff0b: default error result from overflow/underflow multiply
        let h = exp_c(if b >> 63 != 0 {
            0x1008_7118
        } else {
            0x1008_7120
        });
        return h * h;
    }
    // fbc8 -> 5a770: bitwise round to nearest, ties AWAY from zero
    let scaled = x * exp_c(0x1008_7140);
    let sb = scaled.to_bits();
    let e = ((sb >> 52) & 0x7ff) as i32 - 0x3ff;
    let rounded: f64 = if e < -1 {
        scaled * 0.0
    } else if e == -1 {
        if sb >> 63 != 0 {
            -1.0
        } else {
            1.0
        }
    } else if e <= 51 {
        let mask = (1u64 << (52 - e)) - 1;
        f(sb.wrapping_add(1u64 << (51 - e)) & !mask)
    } else {
        scaled
    };
    // fbd5 fstl -> xmm1; fbe3 fisttp consumes the same integral value
    let k = rounded as i64;
    let mut v0 = exp_c(0x1008_7150) * rounded;
    let mut v1 = rounded * exp_c(0x1008_7158);
    v0 += x;
    v0 += v1;
    let j = 2 * (k & 127);
    v1 = exp_c(0x1008_7168) * v0;
    v1 += exp_c(0x1008_7160);
    let mut v2 = v0 * v0;
    let table_bits = exp_c(0x1008_7140 + ((j + 15) * 8) as u64).to_bits();
    let scale_bits = table_bits.wrapping_add(((k as u64) & 0xffff_ffff) << 45);
    let mut v3 = exp_c(0x1008_7140 + ((j + 14) * 8) as u64);
    v1 *= v2;
    v3 += v0;
    v2 = v2 * v2;
    v0 *= exp_c(0x1008_7178);
    v0 += exp_c(0x1008_7170);
    v1 += v3;
    v0 *= v2;
    v1 += v0;
    if exponent <= 0x407 {
        // fc91..fcad
        let scale = f(scale_bits);
        v1 *= scale;
        return v1 + scale;
    }
    if k >= 0 {
        // fde0..fe28: split overflow scaling
        let scale = f(scale_bits.wrapping_sub(0x3f10_0000u64 << 32));
        v1 *= scale;
        v1 += scale;
        return v1 * exp_c(0x1008_7128);
    }
    // fd88..fdca: split underflow scaling; fe58 compensates before store
    let scale = f(scale_bits.wrapping_add(0x3fe0_0000u64 << 32));
    let one = exp_c(0x1008_7110);
    v1 *= scale;
    let mut v0 = scale + v1;
    if v0 < one {
        // fe58..fe8c binary64 compensation chain (spill/reload included)
        let v4 = v0 + one;
        let t1 = scale - v0;
        v1 += t1;
        let t2 = one - v4;
        v0 += t2;
        v0 += v1;
        v0 += v4;
        v0 -= one;
    }
    v0 * exp_c(0x1008_7118)
}

/// `_CIlog` value path on the carrier: default rounding, no matherr.
pub fn cilog(x: f64) -> f64 {
    let b = x.to_bits();
    let high = (b >> 32) as u32;

    // 1004977a..1004987f: near-one polynomial (((high-0x3fee0000)&0xffffffff) <= 0x308ff)
    if high.wrapping_sub(0x3fee_0000) <= 0x0003_08ff {
        if b == 0x3ff0_0000_0000_0000 {
            return 0.0; // 10049998: fldz
        }
        let v0 = x - log_c(0x1008_8288); // 1004977a/1004977f
        let a = log_c(0x1008_83c0); // 10049787 (v4 initializer)
        let mut v1 = log_c(0x1008_83b8); // 1004978f
        let mut v3 = v0; // 10049797
        let mut v2 = v0; // 1004979b
        let mut v5 = log_c(0x1008_83a8); // 1004979f
        let v6c = log_c(0x1008_8378); // 100497a7
        v3 *= v0; // 100497af
        v1 *= v0; // 100497b3
        v1 += log_c(0x1008_83b0); // 100497b7
        let mut v4 = a * v3; // 100497bf
        v2 *= v3; // 100497c3
        v5 *= v3; // 100497c7
        v3 *= log_c(0x1008_8390); // 100497cb
        v1 += v4; // 100497d3
        v4 = log_c(0x1008_83c8) * v2; // 100497d7..100497df
        v1 += v4; // 100497e3
        v4 = log_c(0x1008_83a0) * v0; // 100497e7..100497ef
        v4 += log_c(0x1008_8398); // 100497f3
        v1 *= v2; // 100497fb
        v4 += v5; // 100497ff
        let mut v5 = v0; // 10049803
        v1 += v4; // 10049807
        v4 = log_c(0x1008_8388) * v0; // 1004980b..10049813
        v4 += log_c(0x1008_8380); // 10049817
        v1 *= v2; // 1004981f
        let v3 = v3 + v4; // 10049823
        let mut v4 = v0; // 10049827
        v1 += v3; // 1004982b
        let mut v3 = v0; // 1004982f
        v1 *= v2; // 10049833
        let v2 = log_c(0x1008_8298) * v0; // 10049837..1004983f
        v5 += v2; // 10049843
        v5 -= v2; // 10049847 (Dekker: recovers high part of v0)
        let mut v2 = v5; // 1004984b
        v2 *= v5; // 1004984f
        v2 *= v6c; // 10049853
        v3 += v2; // 10049857
        v4 -= v3; // 1004985b
        v4 += v2; // 1004985f
        let mut v2 = v0; // 10049863
        v2 -= v5; // 10049867
        let v0a = v0 + v5; // 1004986b
        let v2b = v2 * v6c; // 1004986f
        let v0b = v0a * v2b; // 10049873
        let mut v0 = v0b + v4; // 10049877
        v0 += v1; // 1004987b
        let v3 = v3 + v0; // 1004987f
        return v3;
    }
    // 10049898..100499d6: classify before normalizing subnormals
    let absolute = b & 0x7fff_ffff_ffff_ffff;
    if absolute == 0 {
        return f64::NEG_INFINITY; // +/-0: signed reciprocal, default error return
    }
    if absolute > 0x7ff0_0000_0000_0000 {
        return f(b | 0x0008_0000_0000_0000); // fld/fst quiets sNaN
    }
    if b == 0x7ff0_0000_0000_0000 {
        return x; // +inf
    }
    if b >> 63 != 0 {
        return f(0xfff8_0000_0000_0000); // SSE invalid indefinite
    }
    let mut bits = b;
    let mut high = high;
    if high < 0x10_0000 {
        // subnormal normalization
        bits = (x * log_c(0x1008_82a0))
            .to_bits()
            .wrapping_add((0xfcc0_0000u64) << 32);
        high = (bits >> 32) as u32;
    }
    // 10049651..100496e9: integer exponent/index and split table reduction
    let delta = high.wrapping_sub(0x3fe6_0000);
    let index = ((delta >> 13) & 0x7f) as u64;
    let signed = delta as i32; // same value the reference picks (<0x80000000 branch or wrap)
    let mut v2 = (signed >> 20) as f64; // arithmetic shift, like Python's
    let mut v1 = f(bits.wrapping_sub(((delta & 0xfff0_0000u32) as u64) << 32));
    let mut v0 = log_c(0x1008_8370);
    let mut v3 = log_c(0x1008_8340);
    let mut v6 = log_c(0x1008_8360);
    let at = 0x1008_8340u64 + (index + 0x89) * 16;
    v1 -= log_c(at);
    v1 -= log_c(at + 8);
    let at = 0x1008_8340u64 + (index + 9) * 16;
    v3 *= v2;
    v1 *= log_c(at);
    v3 += log_c(at + 8);
    v2 *= log_c(0x1008_8348);
    v0 *= v1; // 100496f1
    let mut v5 = v1; // 100496f5
    let mut v4 = v1; // 100496f9
    v5 *= v1; // 100496fd
    v4 += v3; // 10049701
    v0 += log_c(0x1008_8368); // 10049705
    v6 *= v1; // 1004970d
    v6 += log_c(0x1008_8358); // 10049711
    v3 -= v4; // 10049719
    v0 *= v5; // 1004971d
    v3 += v1; // 10049721
    v2 += v3; // 10049725
    v0 += v6; // 10049729
    v6 = v1; // 1004972d
    v6 *= v5; // 10049731
    v5 *= log_c(0x1008_8350); // 10049735
    v0 *= v6; // 1004973d
    v2 += v5; // 10049741
    v0 += v2; // 10049745
    v0 += v4; // 10049749
    v0
}

#[cfg(test)]
mod tests {
    use super::*;

    const TAU_HINT: f64 = 0.785_398_163_397_4483;

    #[test]
    fn atan_basic_values() {
        // known anchors for the atan series (sanity only; parity test locks bits)
        let a = ciatan(1.0);
        assert!((a - TAU_HINT).abs() < 1e-15, "{a}");
        assert!(ciatan(0.0).abs() <= f64::from_bits(1)); // tiny path is identity (±0)
        assert_eq!(ciatan(-1.0), -a);
    }

    #[test]
    fn exp_and_log_roundtrip_anchors() {
        assert_eq!(ciexp(0.0), 1.0); // exponent<0x3c9 path: 0+1
        let e1 = ciexp(1.0);
        assert!((e1 - std::f64::consts::E).abs() < 1e-14, "{e1}");
        let l = cilog(2.0);
        assert!((l - std::f64::consts::LN_2).abs() < 1e-14, "{l}");
    }
}
