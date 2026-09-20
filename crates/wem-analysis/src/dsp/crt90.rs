//! Value-semantics ports of the supplied CRT carrier's `_CIatan`, `_CIexp`
//! and `_CIlog` callees (paired 2013.2 runtime helpers).
//!
//! The helpers use plain binary64 arithmetic, so Rust `f64` maps one-to-one.
//! Operation order is deliberate and parity-locked. Constants and reduction
//! tables live in `crt90_data.rs`; executable code addresses do not leak into
//! the numeric implementation.
//!
//! Like the reference: signed zeros, subnormals, infinities and NaN
//! quieting are modeled; FPU status flags, errno and user matherr
//! callbacks are NOT emulated.

use super::crt90_data::{CIATAN, CIEXP, CILOG};

#[inline]
fn f(b: u64) -> f64 {
    f64::from_bits(b)
}

#[inline]
fn atan_constant(index: usize) -> f64 {
    f(CIATAN[index])
}

#[inline]
fn exp_constant(index: usize) -> f64 {
    f(CIEXP[index])
}

#[inline]
fn log_constant(index: usize) -> f64 {
    f(CILOG[index])
}

/// `_CIatan` value path from the paired CRT carrier.
pub fn ciatan(x: f64) -> f64 {
    let b = x.to_bits();
    let high = ((b >> 32) & 0x7fff_ffff) as u32;
    let negative = b >> 63;

    // Large, NaN, and tiny inputs bypass argument reduction.
    if high > 0x440f_ffff {
        if (b & 0x7fff_ffff_ffff_ffff) > 0x7ff0_0000_0000_0000 {
            return x;
        }
        return atan_constant(if negative != 0 { 9 } else { 8 });
    }
    if high <= 0x3e3f_ffff {
        return x;
    }
    let mut x = x;
    let mut region: i32 = -1;
    if high > 0x3fdb_ffff {
        x = x.abs();
        if high <= 0x3ff2_ffff {
            if high <= 0x3fe5_ffff {
                let numerator = x + x;
                let denominator = x + atan_constant(11);
                let numerator = numerator - atan_constant(10);
                x = numerator / denominator;
                region = 0;
            } else {
                let t = atan_constant(10);
                x = (x - t) / (x + t);
                region = 1;
            }
        } else if high <= 0x4003_7fff {
            let fc0 = atan_constant(12);
            let denominator = x * fc0;
            let numerator = x - fc0;
            let denominator = denominator + atan_constant(10);
            x = numerator / denominator;
            region = 2;
        } else {
            x = atan_constant(13) / x;
            region = 3;
        }
    }
    // Even polynomial in x^4, times x^2.
    let z = x * x;
    let w = z * z;
    let mut even = atan_constant(14) * w;
    even += atan_constant(15);
    even *= w;
    even += atan_constant(16);
    even *= w;
    even += atan_constant(17);
    even *= w;
    even += atan_constant(18);
    even *= w;
    even += atan_constant(19);
    let even = even * z;
    // Odd polynomial; subtraction signs are literal.
    let mut odd = atan_constant(20) * w;
    odd -= atan_constant(21);
    odd *= w;
    odd -= atan_constant(22);
    odd *= w;
    odd -= atan_constant(23);
    odd *= w;
    odd -= atan_constant(24);
    let odd = w * odd;
    let correction = (even + odd) * x;
    if region == -1 {
        return x - correction;
    }
    // Low part, reduced argument, high part, then sign restoration.
    let correction = correction - atan_constant(region as usize);
    let correction = correction - x;
    let result = atan_constant(4 + region as usize) - correction;
    if negative != 0 {
        -result
    } else {
        result
    }
}

/// `_CIexp` value path from the paired CRT carrier.
pub fn ciexp(x: f64) -> f64 {
    let b = x.to_bits();
    let exponent = ((b >> 52) & 0x7ff) as u32;

    if (b & 0x7fff_ffff_ffff_ffff) > 0x7ff0_0000_0000_0000 {
        // The default callback returns the argument; loading quiets a signaling NaN.
        return f(b | 0x0008_0000_0000_0000);
    }
    if exponent < 0x3c9 {
        return x + exp_constant(0);
    }
    if exponent > 0x408 {
        if b == 0xfff0_0000_0000_0000 {
            return 0.0;
        }
        if exponent == 0x7ff {
            return x + exp_constant(0);
        }
        let h = exp_constant(if b >> 63 != 0 { 1 } else { 2 });
        return h * h;
    }
    // Bitwise round to nearest with ties away from zero.
    let scaled = x * exp_constant(6);
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
    let k = rounded as i64;
    let mut reduced = exp_constant(8) * rounded;
    let mut polynomial = rounded * exp_constant(9);
    reduced += x;
    reduced += polynomial;
    let j = 2 * (k & 127);
    polynomial = exp_constant(11) * reduced;
    polynomial += exp_constant(10);
    let mut reduced_square = reduced * reduced;
    let table_bits = exp_constant((21 + j) as usize).to_bits();
    let scale_bits = table_bits.wrapping_add(((k as u64) & 0xffff_ffff) << 45);
    let mut table_correction = exp_constant((20 + j) as usize);
    polynomial *= reduced_square;
    table_correction += reduced;
    reduced_square *= reduced_square;
    reduced *= exp_constant(13);
    reduced += exp_constant(12);
    polynomial += table_correction;
    reduced *= reduced_square;
    polynomial += reduced;
    if exponent <= 0x407 {
        let scale = f(scale_bits);
        polynomial *= scale;
        return polynomial + scale;
    }
    if k >= 0 {
        // Split overflow scaling.
        let scale = f(scale_bits.wrapping_sub(0x3f10_0000u64 << 32));
        polynomial *= scale;
        polynomial += scale;
        return polynomial * exp_constant(3);
    }
    // Split underflow scaling, with compensation before the final store.
    let scale = f(scale_bits.wrapping_add(0x3fe0_0000u64 << 32));
    let one = exp_constant(0);
    polynomial *= scale;
    let mut result = scale + polynomial;
    if result < one {
        let shifted = result + one;
        let scale_error = scale - result;
        polynomial += scale_error;
        let shift_error = one - shifted;
        result += shift_error;
        result += polynomial;
        result += shifted;
        result -= one;
    }
    result * exp_constant(1)
}

/// `_CIlog` value path on the carrier: default rounding, no matherr.
pub fn cilog(x: f64) -> f64 {
    let b = x.to_bits();
    let high = (b >> 32) as u32;

    // A dedicated near-one polynomial avoids cancellation in the general
    // exponent/table reduction.
    if high.wrapping_sub(0x3fee_0000) <= 0x0003_08ff {
        if b == 0x3ff0_0000_0000_0000 {
            return 0.0;
        }
        let delta = x - log_constant(0);
        let leading_coefficient = log_constant(39);
        let mut series = log_constant(38);
        let mut scaled_square = delta;
        let mut delta_cube = delta;
        let mut secondary_polynomial = log_constant(36);
        let split_scale = log_constant(30);
        scaled_square *= delta;
        series *= delta;
        series += log_constant(37);
        let mut term = leading_coefficient * scaled_square;
        delta_cube *= scaled_square;
        secondary_polynomial *= scaled_square;
        scaled_square *= log_constant(33);
        series += term;
        term = log_constant(40) * delta_cube;
        series += term;
        term = log_constant(35) * delta;
        term += log_constant(34);
        series *= delta_cube;
        term += secondary_polynomial;
        let mut high_part = delta;
        series += term;
        term = log_constant(32) * delta;
        term += log_constant(31);
        series *= delta_cube;
        let correction = scaled_square + term;
        let mut remainder = delta;
        series += correction;
        let mut reconstructed = delta;
        series *= delta_cube;
        let split_product = log_constant(2) * delta;
        high_part += split_product;
        high_part -= split_product;
        let mut high_square = high_part;
        high_square *= high_part;
        high_square *= split_scale;
        reconstructed += high_square;
        remainder -= reconstructed;
        remainder += high_square;
        let mut low_part = delta;
        low_part -= high_part;
        let combined = delta + high_part;
        let scaled_low = low_part * split_scale;
        let cross_term = combined * scaled_low;
        let mut result = cross_term + remainder;
        result += series;
        reconstructed += result;
        return reconstructed;
    }
    // Classify before normalizing subnormals.
    let absolute = b & 0x7fff_ffff_ffff_ffff;
    if absolute == 0 {
        return f64::NEG_INFINITY;
    }
    if absolute > 0x7ff0_0000_0000_0000 {
        return f(b | 0x0008_0000_0000_0000);
    }
    if b == 0x7ff0_0000_0000_0000 {
        return x;
    }
    if b >> 63 != 0 {
        return f(0xfff8_0000_0000_0000);
    }
    let mut bits = b;
    let mut high = high;
    if high < 0x10_0000 {
        bits = (x * log_constant(3))
            .to_bits()
            .wrapping_add((0xfcc0_0000u64) << 32);
        high = (bits >> 32) as u32;
    }
    // Integer exponent/index and split-table reduction.
    let delta = high.wrapping_sub(0x3fe6_0000);
    let index = ((delta >> 13) & 0x7f) as u64;
    let signed = delta as i32;
    let mut exponent_term = (signed >> 20) as f64;
    let mut reduced = f(bits.wrapping_sub(((delta & 0xfff0_0000u32) as u64) << 32));
    let mut series = log_constant(29);
    let mut table_sum = log_constant(23);
    let mut odd_polynomial = log_constant(27);
    let split_table = 297 + 2 * index as usize;
    reduced -= log_constant(split_table);
    reduced -= log_constant(split_table + 1);
    let reduction_table = 41 + 2 * index as usize;
    table_sum *= exponent_term;
    reduced *= log_constant(reduction_table);
    table_sum += log_constant(reduction_table + 1);
    exponent_term *= log_constant(24);
    series *= reduced;
    let mut reduced_square = reduced;
    let mut reconstruction = reduced;
    reduced_square *= reduced;
    reconstruction += table_sum;
    series += log_constant(28);
    odd_polynomial *= reduced;
    odd_polynomial += log_constant(26);
    table_sum -= reconstruction;
    series *= reduced_square;
    table_sum += reduced;
    exponent_term += table_sum;
    series += odd_polynomial;
    let mut reduced_cube = reduced;
    reduced_cube *= reduced_square;
    reduced_square *= log_constant(25);
    series *= reduced_cube;
    exponent_term += reduced_square;
    series += exponent_term;
    series += reconstruction;
    series
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn atan_basic_values() {
        // known anchors for the atan series (sanity only; parity test locks bits)
        let a = ciatan(1.0);
        assert!((a - std::f64::consts::FRAC_PI_4).abs() < 1e-15, "{a}");
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
