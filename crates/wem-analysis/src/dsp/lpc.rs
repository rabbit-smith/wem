//! Linear-prediction helpers for analysis-stream boundary synthesis.
//!
//! Mirrors Python `wwise_wem/analysis/dsp/lpc.py`. Autocorrelation and the
//! Durbin recursion run in float64; coefficients are stored as float32 with
//! per-tap 0.99 damping for deterministic output.

use crate::config::f32_of;

/// Return damped LPC coefficients using Durbin recursion
/// (Python `wwise_lpc_from_data`).
pub fn wwise_lpc_from_data(
    samples: &[f64],
    order: i64,
) -> Result<Vec<f64>, crate::config::AnalysisError> {
    use crate::config::AnalysisError;
    if order < 1 {
        return Err(AnalysisError::invariant(format!(
            "lpc order invalid (order={:?})",
            order
        )));
    }
    if (samples.len() as i64) <= order {
        return Err(AnalysisError::input(format!(
            "lpc samples short (order={:?}, got={:?})",
            order,
            samples.len() as i64
        )));
    }

    let source: Vec<f64> = samples.to_vec();
    let len = source.len() as i64;
    let mut autocorrelation = Vec::with_capacity((order + 1) as usize);
    for lag in 0..=order {
        let mut acc = 0.0f64;
        for index in lag..len {
            acc += source[index as usize] * source[(index - lag) as usize];
        }
        autocorrelation.push(acc);
    }
    let mut error = autocorrelation[0] * (1.0 + 1.0e-10);
    let epsilon = autocorrelation[0] * 1.0e-9 + 1.0e-10;
    let mut coefficients = vec![0.0f64; order as usize];

    for tap in 0..order {
        if error < epsilon {
            break;
        }
        let mut reflection = -autocorrelation[tap as usize + 1];
        for previous in 0..tap {
            reflection -=
                coefficients[previous as usize] * autocorrelation[(tap - previous) as usize];
        }
        reflection /= error;
        coefficients[tap as usize] = reflection;

        for previous in 0..(tap / 2) {
            let saved = coefficients[previous as usize];
            coefficients[previous as usize] =
                coefficients[(tap - previous - 1) as usize] * reflection + saved;
            coefficients[(tap - previous - 1) as usize] += saved * reflection;
        }
        if tap & 1 != 0 {
            let midpoint = (tap / 2) as usize;
            coefficients[midpoint] += coefficients[midpoint] * reflection;
        }
        error *= 1.0 - reflection * reflection;
    }

    let mut damping = 0.99;
    for tap in 0..order {
        coefficients[tap as usize] = f32_of(coefficients[tap as usize] * damping);
        damping *= 0.99;
    }
    Ok(coefficients)
}

/// Predict `count` samples with a float32 work ring and recurrence
/// (Python `wwise_lpc_predict`).
pub fn wwise_lpc_predict(
    coefficients: &[f64],
    prime: &[f64],
    count: i64,
) -> Result<Vec<f64>, crate::config::AnalysisError> {
    use crate::config::AnalysisError;
    let order = coefficients.len() as i64;
    if order < 1 {
        return Err(AnalysisError::invariant("lpc coefficients empty"));
    }
    if prime.len() as i64 != order {
        return Err(AnalysisError::geometry(format!(
            "lpc prime length mismatch (want={:?}, got={:?})",
            order,
            prime.len() as i64
        )));
    }
    if count < 0 {
        return Err(AnalysisError::geometry(format!(
            "lpc count negative (count={:?})",
            count
        )));
    }

    let mut work: Vec<f64> = prime.iter().map(|v| f32_of(*v)).collect();
    work.extend(std::iter::repeat_n(0.0, count as usize));
    let mut output: Vec<f64> = Vec::with_capacity(count as usize);
    for index in 0..count {
        let mut prediction = 0.0;
        for tap in 0..order {
            prediction = f32_of(
                prediction
                    - work[(index + tap) as usize] * coefficients[(order - tap - 1) as usize],
            );
        }
        work[order as usize + index as usize] = prediction;
        output.push(prediction);
    }
    Ok(output)
}

/// Build the initial left-half history for the first analysis frame
/// (Python `wwise_first_frame_lpc_prime`).
pub fn wwise_first_frame_lpc_prime(
    source: &[f64],
    prefill: i64,
    batch: i64,
    order: i64,
) -> Result<Vec<f64>, crate::config::AnalysisError> {
    use crate::config::AnalysisError;
    if prefill < 1 {
        return Err(AnalysisError::invariant(format!(
            "lpc prefill invalid (prefill={:?})",
            prefill
        )));
    }
    if batch <= order {
        return Err(AnalysisError::invariant(format!(
            "lpc batch invalid (batch={:?}, order={:?})",
            batch, order
        )));
    }
    if (source.len() as i64) < batch {
        return Err(AnalysisError::input(format!(
            "lpc source short (want={:?}, got={:?})",
            batch,
            source.len() as i64
        )));
    }

    let mut reversed_buffer: Vec<f64> = Vec::with_capacity((prefill + batch) as usize);
    reversed_buffer.extend(std::iter::repeat_n(0.0, prefill as usize));
    for value in &source[..batch as usize] {
        reversed_buffer.push(f32_of(*value));
    }
    reversed_buffer.reverse();

    let coefficients = wwise_lpc_from_data(&reversed_buffer[..batch as usize], order)?;
    let predicted_reversed = wwise_lpc_predict(
        &coefficients,
        &reversed_buffer[(batch - order) as usize..batch as usize],
        prefill,
    )?;
    let mut out = predicted_reversed;
    out.reverse();
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn lpc_from_data_rejects_bad_order() {
        assert!(wwise_lpc_from_data(&[1.0, 2.0, 3.0], 0).is_err());
        // order must be < len
        assert!(wwise_lpc_from_data(&[1.0, 2.0, 3.0], 3).is_err());
    }

    #[test]
    fn lpc_predict_rejects_mismatch() {
        assert!(wwise_lpc_predict(&[], &[1.0], 1).is_err());
        assert!(wwise_lpc_predict(&[0.5], &[1.0, 2.0], 1).is_err());
        assert!(wwise_lpc_predict(&[0.5], &[1.0], -1).is_err());
    }

    #[test]
    fn lpc_predict_produces_expected_length() {
        let coeffs = vec![0.5, -0.25];
        let prime = vec![1.0, 0.0];
        let out = wwise_lpc_predict(&coeffs, &prime, 4).expect("predict");
        assert_eq!(out.len(), 4);
    }

    #[test]
    fn first_frame_prime_length() {
        let source: Vec<f64> = (0..5000).map(|i| (i as f64) * 0.001).collect();
        let out = wwise_first_frame_lpc_prime(&source, 128, 4096, 16).expect("prime");
        assert_eq!(out.len(), 128);
    }
}
