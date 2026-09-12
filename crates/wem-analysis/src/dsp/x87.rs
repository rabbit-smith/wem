//! x87 / f32 primitives of the paired 2013.2 build.
//!
//! Port of `corpus/paired-build/the round/src/f32.py` (single source of the
//! semantics). The paired build is 32-bit x86: intermediates live in 80-bit
//! x87 registers (64-bit significand) and are re-rounded only at explicit
//! store points. `F80` holds a register value exactly as
//! `(-1)^neg * man * 2^(exp - 63)` with `man` a 64-bit integer whose MSB
//! (integer bit) is set. `mul80`/`add80` compute the exact aligned integer
//! first and round once to 64 significant bits, ties-to-even — bit-for-bit
//! equal to the Python reference's `Fraction` + `_round80` semantics
//! (parity vectors: `tests/x87_parity.rs`, generated from the reference).
//!
//! Domain: finite register values whose exponent difference is <= 64 — the
//! same "finite normal-range" the Python reference models with exact
//! rationals; anything wider trips a debug assertion (builder call sites are
//! curve/dB arithmetic with tiny exponent spans).
//!
//! Determinism contract (AGENTS.md): no host transcendental calls, no clocks,
//! no randomness; bit patterns travel as integers.

/// Finite x87 register value. `man == 0` encodes zero (mirroring the
/// reference's `Fraction(0)` round trip). Builders never feed inf/NaN.
#[derive(Clone, Copy, Debug)]
pub struct F80 {
    neg: bool,
    /// Exponent of the integer bit: value = man * 2^(exp - 63).
    exp: i32,
    /// 64-bit significand, MSB set when nonzero.
    man: u64,
}

impl PartialEq for F80 {
    fn eq(&self, other: &Self) -> bool {
        (self.man == 0 && other.man == 0)
            || (self.neg == other.neg && self.exp == other.exp && self.man == other.man)
    }
}

impl F80 {
    /// Exact 80-bit form of a finite f64 (normal and subnormal).
    pub fn from_f64(x: f64) -> F80 {
        assert!(x.is_finite(), "F80::from_f64: non-finite input");
        let bits = x.to_bits();
        let neg = bits >> 63 == 1;
        let biased = ((bits >> 52) & 0x7FF) as i32;
        let frac = bits & ((1u64 << 52) - 1);
        if biased == 0 {
            if frac == 0 {
                return F80 { neg, exp: 0, man: 0 };
            }
            let lz = frac.leading_zeros(); // 12..=63 for subnormals
            // value = frac * 2^-1074 = (frac << lz) * 2^(-1074 - lz)
            F80 { neg, exp: -1011 - lz as i32, man: frac << lz }
        } else {
            F80 { neg, exp: biased - 1023, man: (1u64 << 63) | (frac << 11) }
        }
    }

    /// Python `float(Fraction(...))`: correctly rounded f64 (RNE), including
    /// the subnormal exit. Zero keeps the sign of the register.
    pub fn to_f64(self) -> f64 {
        if self.man == 0 {
            return if self.neg { -0.0 } else { 0.0 };
        }
        let r = self.man & 0x7FF;
        let mut q = self.man >> 11; // q in [2^52, 2^53]
        let mut exp = self.exp;
        if r > 0x400 || (r == 0x400 && (q & 1) == 1) {
            q += 1;
            if q >> 53 != 0 {
                q >>= 1; // exact power-of-two landing
                exp += 1;
            }
        }
        let biased = exp + 1023;
        assert!(biased < 2047, "F80::to_f64: f64 overflow (reference raises here too)");
        if biased <= 0 {
            // subnormal exit: value = q * 2^(exp - 52)
            let shift = (1 - biased) as u32; // > 0
            if shift >= 64 {
                return if self.neg { -0.0 } else { 0.0 };
            }
            let r = q & ((1u64 << shift) - 1);
            let half = 1u64 << (shift - 1);
            let mut keep = q >> shift;
            if r > half || (r == half && (keep & 1) == 1) {
                keep += 1;
            }
            return f64::from_bits((self.neg as u64) << 63 | (keep & ((1u64 << 52) - 1)));
        }
        let frac = q & ((1u64 << 52) - 1);
        f64::from_bits(((self.neg as u64) << 63) | ((biased as u64) << 52) | frac)
    }
}

/// Round the exact nonzero `value * 2^scale` to a 64-bit significand with
/// ties-to-even. `sticky` witnesses a strictly positive residue below the
/// window (lost bits), lifting exact ties upward like IEEE sticky semantics.
fn round64(value: u128, scale: i32, sticky: bool) -> F80 {
    let pos = 127 - value.leading_zeros() as i32; // MSB index (0..=127)
    if pos <= 63 {
        // window fits entirely; a sticky residue can only exist when drop>0,
        // which cannot happen here (callers pass sticky=false on this path)
        debug_assert!(!sticky);
        return F80 { neg: false, exp: scale + pos, man: (value << (63 - pos)) as u64 };
    }
    let drop = (pos - 63) as u32; // 1..=64
    let mask = (1u128 << drop) - 1;
    let r = value & mask;
    let mut man = (value >> drop) as u64;
    let half = 1u128 << (drop - 1);
    if r > half || (r == half && (sticky || (man & 1) == 1)) {
        // rounding up from the all-ones window lands on a fresh power of two
        if man == u64::MAX {
            return F80 { neg: false, exp: scale + pos + 1, man: 1u64 << 63 };
        }
        man += 1;
    }
    F80 { neg: false, exp: scale + pos, man }
}

/// Python `mul80`: exact product, one round to 64 significant bits.
pub fn mul80(a: F80, b: F80) -> F80 {
    if a.man == 0 || b.man == 0 {
        return F80 { neg: false, exp: 0, man: 0 };
    }
    let product = (a.man as u128) * (b.man as u128); // 2^126 .. 2^128-2^65+1
    let mut out = round64(product, a.exp + b.exp - 126, false);
    out.neg = a.neg ^ b.neg;
    out
}

/// Python `add80`: exact aligned sum/difference with a single RNE round for
/// any exponent difference (matching the reference's exact `Fraction` math,
/// including the wide-gap sticky paths). Supports full cancellation.
pub fn add80(a: F80, b: F80) -> F80 {
    if a.man == 0 {
        return b;
    }
    if b.man == 0 {
        return a;
    }
    let (hi, lo) = if a.exp >= b.exp { (a, b) } else { (b, a) };
    let diff = (hi.exp - lo.exp) as u32;
    let scale0 = lo.exp - 63;
    if diff <= 64 {
        // both significands fully inside the u128 window (64+64 bits max)
        let hi_w = (hi.man as u128) << diff;
        let lo_w = lo.man as u128;
        if hi.neg == lo.neg {
            let mut out = round64(hi_w + lo_w, scale0, false);
            out.neg = hi.neg;
            return out;
        }
        match hi_w.cmp(&lo_w) {
            std::cmp::Ordering::Equal => F80 { neg: false, exp: 0, man: 0 },
            std::cmp::Ordering::Greater => {
                let mut out = round64(hi_w - lo_w, scale0, false);
                out.neg = hi.neg;
                out
            }
            std::cmp::Ordering::Less => {
                let mut out = round64(lo_w - hi_w, scale0, false);
                out.neg = lo.neg;
                out
            }
        }
    } else {
        // wide gap: keep hi.man<<63 in the window; lo contributes hi bits plus
        // a strict-positive sticky residue below it
        let k = diff - 63; // >= 2
        let lo_top = if k >= 64 { 0u64 } else { lo.man >> k };
        let dropped = if k >= 64 {
            lo.man
        } else {
            lo.man & ((1u64 << k) - 1)
        };
        let base = (hi.man as u128) << 63;
        let scale = scale0 + k as i32;
        if hi.neg == lo.neg {
            let mut out = round64(base + lo_top as u128, scale, dropped != 0);
            out.neg = hi.neg;
            return out;
        }
        // |hi|*2^diff - |lo| > 0 strictly (base >= 2^126, |lo| < 2^127 - no flip)
        let out = if dropped == 0 {
            round64(base - lo_top as u128, scale, false)
        } else {
            // value = W*2^scale - dropped = (W-1)*2^scale + (2^scale-unit - dropped):
            // positive residue below the window -> sticky, and W-1 >= 2^126 safe
            round64(base - lo_top as u128 - 1, scale, true)
        };
        let mut out = out;
        out.neg = hi.neg;
        out
    }
}

/// Python `f32(x)`: round to single precision (x87 `fstps`).
#[inline]
pub fn f32_round(x: f64) -> f64 {
    (x as f32) as f64
}

/// Python `f32_bits(x)`.
#[inline]
pub fn f32_bits(x: f64) -> u32 {
    (x as f32).to_bits()
}

/// Python `bits_f32(b)`.
#[inline]
pub fn bits_f32(b: u32) -> f64 {
    f32::from_bits(b) as f64
}

/// wwise_float_log core (bit-trick log, NOT math.log):
/// `f32( float(absbits(f32(x))) * LOG_L - LOG_M )`.
#[inline]
pub fn wwise_float_log(x: f64) -> f64 {
    const LOG_L: f64 = 7.177114298428933e-07;
    const LOG_M: f64 = 764.6162109375;
    let bits = ((f32_bits(x) & 0x7FFF_FFFF)) as f64; // <= 2^31: exact in f64
    f32_round(bits * LOG_L - LOG_M)
}

/// S3 offset: `f32( wwise_float_log(f32(4.0/n)) + 0.345 )`.
#[inline]
pub fn offset_for_n(n: u32) -> f64 {
    const ADD: f64 = 0.345;
    f32_round(wwise_float_log(f32_round(4.0 / n as f64)) + ADD)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn f64_roundtrip() {
        for x in [1.0f64, 0.5, 2.25, 1e-3, 12345.6789, -42.0, 3.1415927410125732, 5.96e-8] {
            assert_eq!(F80::from_f64(x).to_f64(), x, "roundtrip {x}");
        }
    }

    #[test]
    fn exact_ops() {
        assert_eq!(mul80(F80::from_f64(0.5), F80::from_f64(2.0)).to_f64(), 1.0);
        assert_eq!(add80(F80::from_f64(0.1), F80::from_f64(-0.1)).man, 0);
        // domain guard: exponent diff <= 64 (builder curve arithmetic)
        let m = add80(F80::from_f64(2.0f64.powi(-40)), F80::from_f64(1.0));
        assert_eq!(m.to_f64(), 1.0 + 2f64.powi(-40));
    }
}
