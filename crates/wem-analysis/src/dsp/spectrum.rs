//! Float32 spectral transforms used by Wwise psychoacoustic analysis.
//!
//! Mirrors Python `wwise_wem/analysis/dsp/spectrum.py`. The float-bit
//! logarithm is pure integer/bit arithmetic; the packed FFT reads twiddles
//! exclusively from the frozen profile domain (a missing stage is a domain
//! error, not an analytic fallback).

use crate::config::{f32_of, AnalysisError, FrozenMathTables};

const WWISE_LOG_SCALE: f64 = 0.0000007177114298428933;
const WWISE_LOG_BIAS: f64 = 764.6162109375;
const WWISE_LOG_ADD: f64 = 0.345;

/// Return the positive float32 bit pattern used by spectral scaling
/// (Python `_float_abs_bits`).
fn float_abs_bits(value: f64) -> u32 {
    (value as f32).to_bits() & 0x7FFF_FFFF
}

/// Apply the encoder format's float-bit logarithm approximation
/// (Python `wwise_float_log`).
pub fn wwise_float_log(value: f64) -> f64 {
    f32_of(float_abs_bits(value) as f64 * WWISE_LOG_SCALE - WWISE_LOG_BIAS)
}

/// Return the packed real-FFT layout used by frame analysis
/// (Python `wwise_fft_packed`).
///
/// For an even `n` the layout is `[DC, Re(1), Im(1), ..., Re(n/2)]`. Every
/// input and butterfly result is rounded to float32. `twiddles` must be
/// provided (the frozen profile domain); the analytic path is not taken.
///
/// Each stage's twiddle factor sequence is derived once per call, by the same
/// recurrence the butterfly loop would otherwise run, and read back by index.
/// The recurrence depends on nothing but `(step_re, step_im, k)`, so the
/// sequence is bit-identical at every `start` block of the stage; indexing it
/// hands each butterfly exactly the pair it saw before.
pub fn wwise_fft_packed(
    samples: &[f64],
    twiddles: &FrozenMathTables,
) -> Result<Vec<f64>, AnalysisError> {
    let n = samples.len();
    if n < 2 || (n & (n - 1)) != 0 || n & 1 != 0 {
        return Err(AnalysisError::FftSizeInvalid { n: n as i64 });
    }

    let mut real: Vec<f64> = samples.iter().map(|v| f32_of(*v)).collect();
    let mut imag = vec![0.0f64; n];

    // Bit-reversal permutation.
    let mut j = 0usize;
    for i in 1..n {
        let mut bit = n >> 1;
        while j & bit != 0 {
            j ^= bit;
            bit >>= 1;
        }
        j ^= bit;
        if i < j {
            real.swap(i, j);
        }
    }

    // The twiddle sequence of one stage, at most `n / 2` pairs (the widest
    // stage's `half`). It is scratch, not state: every stage refills the part
    // it needs before reading it. It is allocated here, once per call, because
    // the caller chain (`wwise_log_curve` from `psychoacoustics::pipeline`) has
    // no session-owned buffer to borrow — the same per-call shape as the
    // `real`/`imag`/`packed` buffers this function already allocates.
    let mut twiddle: Vec<(f64, f64)> = vec![(0.0, 0.0); n / 2];

    let mut length = 2usize;
    while length <= n {
        let half = length >> 1;
        let entry = twiddles.fft_twiddles.get(&(length as i64)).ok_or(
            AnalysisError::FrozenTwiddlesMissing {
                length: length as i64,
            },
        )?;
        let step_re = f32_of(entry.0);
        let step_im = f32_of(entry.1);
        // The stage's sequence, from `(1, 0)` up, by the identical recurrence
        // with the identical f32 rounding points. The final step computes
        // `w_half`, which no butterfly reads, exactly as before.
        let stage = &mut twiddle[..half];
        let mut wr = 1.0;
        let mut wi = 0.0;
        for slot in stage.iter_mut() {
            *slot = (wr, wi);
            let (new_wr, new_wi) = (
                f32_of(wr * step_re - wi * step_im),
                f32_of(wr * step_im + wi * step_re),
            );
            wr = new_wr;
            wi = new_wi;
        }
        for start in (0..n).step_by(length) {
            for (k, &(wr, wi)) in stage.iter().enumerate() {
                let even = start + k;
                let odd = even + half;
                let tr = f32_of(wr * real[odd] - wi * imag[odd]);
                let ti = f32_of(wr * imag[odd] + wi * real[odd]);
                let er = real[even];
                let ei = imag[even];
                real[even] = f32_of(er + tr);
                imag[even] = f32_of(ei + ti);
                real[odd] = f32_of(er - tr);
                imag[odd] = f32_of(ei - ti);
            }
        }
        length <<= 1;
    }

    let mut packed = vec![f32_of(real[0])];
    for k in 1..(n / 2) {
        packed.push(f32_of(real[k]));
        packed.push(f32_of(imag[k]));
    }
    packed.push(f32_of(real[n / 2]));
    Ok(packed)
}

/// Convert a packed analysis spectrum to the encoder log domain
/// (Python `wwise_log_curve`).
pub fn wwise_log_curve(
    samples: &[f64],
    twiddles: &FrozenMathTables,
) -> Result<Vec<f64>, AnalysisError> {
    let n = samples.len();
    let packed = wwise_fft_packed(samples, twiddles)?;
    let offset = f32_of(wwise_float_log(4.0 / n as f64) + WWISE_LOG_ADD);
    let mut out = vec![f32_of(wwise_float_log(packed[0]) + offset + WWISE_LOG_ADD)];
    for index in (1..n - 1).step_by(2) {
        let power = f32_of(packed[index] * packed[index] + packed[index + 1] * packed[index + 1]);
        out.push(f32_of(
            0.5 * wwise_float_log(power) + offset + WWISE_LOG_ADD,
        ));
    }
    Ok(out)
}

/// Convert normalized MDCT coefficients to the encoder log domain
/// (Python `wwise_mdct_log_curve`).
pub fn wwise_mdct_log_curve(samples: &[f64]) -> Vec<f64> {
    samples
        .iter()
        .map(|value| f32_of(wwise_float_log((*value).abs()) + WWISE_LOG_ADD))
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn float_log_matches_python_oracle() {
        // Verified against Python oracle (wwise_float_log):
        // log(1.0) = -3.0994415283203125e-05
        // log(2.0) = 6.02056884765625
        // log(0.5) = -6.020630836486816
        assert_eq!(wwise_float_log(1.0), -3.0994415283203125e-05);
        assert_eq!(wwise_float_log(2.0), 6.02056884765625);
        assert_eq!(wwise_float_log(0.5), -6.020630836486816);
    }

    #[test]
    fn fft_rejects_bad_sizes() {
        let frozen = FrozenMathTables {
            coordinate_ln: std::collections::HashMap::new(),
            fft_twiddles: std::collections::HashMap::new(),
            window_halves: std::collections::HashMap::new(),
        };
        assert!(wwise_fft_packed(&[1.0], &frozen).is_err());
        assert!(wwise_fft_packed(&[1.0, 2.0, 3.0], &frozen).is_err());
    }

    #[test]
    fn mdct_log_curve_uses_abs() {
        let out = wwise_mdct_log_curve(&[-2.0, 0.0]);
        assert_eq!(out.len(), 2);
        // abs(-2.0) == abs(2.0)
        let out2 = wwise_mdct_log_curve(&[2.0, 0.0]);
        assert_eq!(out[0], out2[0]);
    }
}
