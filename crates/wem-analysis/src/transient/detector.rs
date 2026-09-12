//! Per-channel transient detection and persistent band history.
//!
//! Mirrors Python `wwise_wem/analysis/transient/detector.py`.

use crate::config::{f32_of, MdctLook, TransientDetectorTables};
use crate::dsp::spectrum::wwise_float_log;
use crate::dsp::transform::wwise_psy_mdct;

/// Energy and band-history state for one detector channel
/// (Python `WwisePsyHistory`).
#[derive(Debug, Clone, PartialEq)]
pub struct WwisePsyHistory {
    pub energy_ring: Vec<f64>,
    pub energy_sum: f64,
    pub last_energy: f64,
    pub cursor: usize,
    pub band_rings: Vec<Vec<f64>>,
    pub band_cursors: Vec<usize>,
    pub band_flags: i64,
}

impl Default for WwisePsyHistory {
    fn default() -> Self {
        Self::new()
    }
}

impl WwisePsyHistory {
    pub fn new() -> Self {
        Self {
            energy_ring: vec![0.0f64; 15],
            energy_sum: 0.0,
            last_energy: 0.0,
            cursor: 0,
            band_rings: (0..12).map(|_| vec![0.0f64; 17]).collect(),
            band_cursors: vec![0usize; 12],
            band_flags: 0,
        }
    }
}

/// Update twelve band histories and return their combined flags
/// (Python `_wwise_psy_band_update`).
fn wwise_psy_band_update(
    mask: &[f64],
    history: &mut WwisePsyHistory,
    history_window: i64,
    config_row: &[f64],
    tables: &TransientDetectorTables,
) -> Result<i64, crate::config::AnalysisError> {
    use crate::config::AnalysisError;
    let required = tables
        .bands
        .iter()
        .map(|b| b.offset + b.weights.len() as i64)
        .max()
        .unwrap_or(0);
    if (mask.len() as i64) < required {
        return Err(AnalysisError::PsyMaskShortForBands {
            want: required,
            got: mask.len() as i64,
        });
    }
    if history.band_rings.len() != 12 || history.band_cursors.len() != 12 {
        return Err(AnalysisError::PsyBandStateRings { want: 12 });
    }
    if config_row.len() < 26 {
        return Err(AnalysisError::PsyConfigRowShort { want: 26 });
    }
    let half_window = history_window / 2;
    let window = 2.max(half_window);
    let mut carry = config_row[25] - (half_window - 2) as f64;
    carry = 0.0f64.max(config_row[25].min(carry));

    let mut flags = 0i64;
    for (band, descriptor) in tables.bands.iter().enumerate() {
        let mut accumulator = 0.0f64;
        for (index, weight) in descriptor.weights.iter().enumerate() {
            accumulator =
                f32_of(accumulator + mask[descriptor.offset as usize + index] * (*weight as f64));
        }
        let current = f32_of(accumulator * (descriptor.scale as f64));
        let ring = &mut history.band_rings[band];
        let cursor = history.band_cursors[band];
        let previous = ring[(cursor as i64 - 1).rem_euclid(17) as usize];
        let current_max = current.max(previous);
        let current_min = current.min(previous);
        let mut history_max = -99999.0f64;
        let mut history_min = 99999.0f64;
        for step in 0..window {
            let value = ring[((cursor as i64 - 2 - step).rem_euclid(17)) as usize];
            history_max = history_max.max(value);
            history_min = history_min.min(value);
        }
        let max_delta = current_max - history_max;
        let min_delta = current_min - history_min;

        ring[cursor] = current;
        history.band_cursors[band] = (cursor + 1) % 17;

        if config_row[1 + band] + carry < max_delta {
            flags |= 5;
        }
        if config_row[13 + band] - carry > min_delta {
            flags |= 2;
        }
    }
    history.band_flags = flags;
    Ok(flags)
}

/// Compute an MDCT mask and update twelve-band transient history
/// (Python `wwise_psy_mask`).
pub fn wwise_psy_mask(
    samples: &[f64],
    tables: &TransientDetectorTables,
    mdct_look: &MdctLook,
    history: &mut WwisePsyHistory,
    bias: Option<f64>,
    history_window: i64,
    config_row: Option<&[f64]>,
) -> Result<Vec<f64>, crate::config::AnalysisError> {
    use crate::config::AnalysisError;
    if history.energy_ring.len() != 15 {
        return Err(AnalysisError::PsyEnergyRingSlots {
            want: 15,
            got: history.energy_ring.len(),
        });
    }
    if samples.len() as i64 != tables.n {
        return Err(AnalysisError::TransientMaskSamples {
            want: tables.n,
            got: samples.len() as i64,
        });
    }
    let bias = bias.unwrap_or(tables.bias as f64);
    let config_row_owned: Vec<f64> = tables.config.iter().map(|v| *v as f64).collect();
    let config_row: &[f64] = match config_row {
        Some(row) => row,
        None => &config_row_owned,
    };

    let spectrum = wwise_psy_mdct(samples, mdct_look, tables)?;
    if spectrum.len() < 4 || spectrum.len() & 1 != 0 {
        return Err(AnalysisError::PsySpectrumBinsOdd {
            bins: spectrum.len() as i64,
        });
    }

    let energy = f32_of(
        spectrum[0] * spectrum[0]
            + spectrum[1] * 0.7 * spectrum[1]
            + spectrum[2] * 0.2 * spectrum[2],
    );
    let slot = history.cursor;
    if slot != 0 {
        history.energy_sum = f32_of(history.energy_sum + energy);
        history.last_energy = f32_of(history.last_energy + energy);
    } else {
        history.energy_sum = f32_of(energy + history.last_energy);
        history.last_energy = energy;
    }
    history.energy_sum = f32_of(history.energy_sum - history.energy_ring[slot]);
    history.energy_ring[slot] = energy;
    history.cursor += 1;
    if history.cursor >= 15 {
        history.cursor = 0;
    }

    let avg = f32_of(history.energy_sum * 0.0625);
    let mut descending_floor = f32_of(0.5 * wwise_float_log(avg.abs()) - 15.0);
    let mut out = Vec::with_capacity(spectrum.len() / 2);
    for i in 0..(spectrum.len() / 2) {
        let power =
            f32_of(spectrum[2 * i] * spectrum[2 * i] + spectrum[2 * i + 1] * spectrum[2 * i + 1]);
        let curve = f32_of(0.5 * wwise_float_log(power.abs()));
        let value = descending_floor.max(curve).max(bias);
        out.push(f32_of(value));
        descending_floor = f32_of(descending_floor - 8.0);
    }
    wwise_psy_band_update(out.as_slice(), history, history_window, config_row, tables)?;
    Ok(out)
}

/// Own channel histories and combine their transient flags
/// (Python `TransientDetector`).
pub struct TransientDetector {
    pub channels: i64,
    pub tables: TransientDetectorTables,
    pub mdct_look: MdctLook,
    pub bins: i64,
    pub histories: Vec<WwisePsyHistory>,
    pub quanta: i64,
}

impl TransientDetector {
    /// Construct and reset (Python `__post_init__` + `reset`).
    pub fn new(
        channels: i64,
        tables: TransientDetectorTables,
        mdct_look: MdctLook,
        bins: i64,
    ) -> Result<Self, crate::config::AnalysisError> {
        use crate::config::AnalysisError;
        if channels <= 0 {
            return Err(AnalysisError::DetectorChannelCountMismatch { want: channels });
        }
        if bins != 128 {
            return Err(AnalysisError::TransientMaskSamples {
                want: 128,
                got: bins,
            });
        }
        if tables.n != bins || mdct_look.n != bins {
            return Err(AnalysisError::TransientMdctGeometry {
                look_n: mdct_look.n,
                n: tables.n,
            });
        }
        let mut detector = Self {
            channels,
            tables,
            mdct_look,
            bins,
            histories: Vec::new(),
            quanta: 0,
        };
        detector.reset();
        Ok(detector)
    }

    /// Clear all channel histories and the quantum counter.
    pub fn reset(&mut self) {
        self.histories = (0..self.channels).map(|_| WwisePsyHistory::new()).collect();
        self.quanta = 0;
    }

    /// Analyze one channel-aligned quantum and combine its flags.
    pub fn analyze_quantum(
        &mut self,
        pcm_by_channel: &[Vec<f64>],
        history_window: i64,
    ) -> Result<i64, crate::config::AnalysisError> {
        use crate::config::AnalysisError;
        if pcm_by_channel.len() as i64 != self.channels {
            return Err(AnalysisError::DetectorChannelCountMismatch {
                want: self.channels,
            });
        }
        if pcm_by_channel
            .iter()
            .any(|row| row.len() as i64 != self.bins)
        {
            return Err(AnalysisError::DetectorQuantumSamplesMismatch { want: self.bins });
        }
        let mut flags = 0i64;
        for (samples, history) in pcm_by_channel.iter().zip(self.histories.iter_mut()) {
            wwise_psy_mask(
                samples,
                &self.tables,
                &self.mdct_look,
                history,
                None,
                history_window,
                None,
            )?;
            flags |= history.band_flags;
        }
        self.quanta += 1;
        Ok(flags)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample_tables() -> TransientDetectorTables {
        TransientDetectorTables {
            n: 128,
            bias: -60.0,
            window: vec![0.5f32; 128],
            config: vec![1.0f32; 26],
            bands: vec![
                crate::config::TransientBandConfig {
                    offset: 0,
                    weights: vec![1.0f32, 1.0],
                    scale: 1.0,
                };
                12
            ],
        }
    }

    #[test]
    fn history_initial_shape() {
        let h = WwisePsyHistory::new();
        assert_eq!(h.energy_ring.len(), 15);
        assert_eq!(h.band_rings.len(), 12);
        assert_eq!(h.band_rings[0].len(), 17);
        assert_eq!(h.band_cursors.len(), 12);
    }

    #[test]
    fn mask_runs_with_sample_tables() {
        let tables = sample_tables();
        let look = crate::dsp::transform::make_mdct_look(128, None).expect("look");
        let mut h = WwisePsyHistory::new();
        // 128 samples for the n=128 MDCT look.
        let samples: Vec<f64> = (0..128).map(|i| ((i as f64) % 7.0) - 3.0).collect();
        let mask = wwise_psy_mask(&samples, &tables, &look, &mut h, None, 1, None).expect("mask");
        assert_eq!(mask.len(), 32);
    }
}
