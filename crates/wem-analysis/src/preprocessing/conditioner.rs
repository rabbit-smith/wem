//! Stateful profile-selected PCM conditioning.

use crate::config::{AnalysisError, InputConditionerConfig};

/// Apply the paired build's DC filter and signed-16 storage boundary.
pub struct InputConditioner {
    coefficient: f32,
    previous_input: Vec<f32>,
    previous_output: Vec<f32>,
}

/// A summary: the filter coefficient and the channel count.
///
/// The two per-channel vectors are one-frame filter state — one value per
/// channel, rebuilt on every `process` call — so they are reported by length;
/// the coefficient and the channel count are what identify the conditioner.
impl std::fmt::Debug for InputConditioner {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("InputConditioner")
            .field("coefficient", &self.coefficient)
            .field("channels", &self.previous_input.len())
            .finish()
    }
}

impl InputConditioner {
    pub fn new(channels: i64, config: &InputConditionerConfig) -> Result<Self, AnalysisError> {
        if channels <= 0 {
            return Err(AnalysisError::geometry(format!(
                "session channels non positive (channels={:?})",
                channels
            )));
        }
        Ok(Self {
            coefficient: config.dc_filter_coefficient,
            previous_input: vec![0.0; channels as usize],
            previous_output: vec![0.0; channels as usize],
        })
    }

    pub fn reset(&mut self) {
        self.previous_input.fill(0.0);
        self.previous_output.fill(0.0);
    }

    pub fn process(&mut self, rows: &[Vec<f64>]) -> Result<Vec<Vec<f64>>, AnalysisError> {
        if rows.len() != self.previous_input.len() {
            return Err(AnalysisError::input(format!(
                "input conditioner channel count mismatch (want={:?}, got={:?})",
                self.previous_input.len() as i64,
                rows.len() as i64
            )));
        }
        let frames = rows.first().map(Vec::len).unwrap_or(0);
        if rows.iter().any(|row| row.len() != frames) {
            return Err(AnalysisError::input(format!(
                "pcm channels unequal (want={:?}, got={:?})",
                frames as i64, 0
            )));
        }

        let mut conditioned = Vec::with_capacity(rows.len());
        for (channel, row) in rows.iter().enumerate() {
            let mut previous_input = self.previous_input[channel];
            let mut previous_output = self.previous_output[channel];
            let mut output = Vec::with_capacity(row.len());
            for &sample in row {
                let current = sample as f32;
                // The paired implementation evaluates the subtraction,
                // feedback multiply and addition before the single f32 store.
                // Keep the operands at their stored f32 values, but do not
                // insert extra f32 assignment boundaries inside the expression.
                let filtered = ((current as f64 - previous_input as f64)
                    + self.coefficient as f64 * previous_output as f64)
                    as f32;
                let stored = (filtered * 32767.0)
                    .round_ties_even()
                    .clamp(-32768.0, 32767.0);
                output.push(stored as f64 / 32768.0);
                previous_input = current;
                previous_output = filtered;
            }
            self.previous_input[channel] = previous_input;
            self.previous_output[channel] = previous_output;
            conditioned.push(output);
        }
        Ok(conditioned)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn chunk_boundaries_preserve_filter_state() {
        let config = InputConditionerConfig::new(f32::from_bits(0x3f7f_546d)).unwrap();
        let rows = vec![vec![0.3812255859375, 0.352569580078125, 0.3597412109375]];
        let mut whole = InputConditioner::new(1, &config).unwrap();
        let expected = whole.process(&rows).unwrap();
        let mut chunked = InputConditioner::new(1, &config).unwrap();
        let mut actual = chunked.process(&[rows[0][..1].to_vec()]).unwrap();
        actual[0].extend(chunked.process(&[rows[0][1..].to_vec()]).unwrap().remove(0));
        assert_eq!(actual, expected);
        assert_eq!(
            actual[0],
            vec![0.3812255859375, 0.3515625, 0.357818603515625]
        );
    }
}
