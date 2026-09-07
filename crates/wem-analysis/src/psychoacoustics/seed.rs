//! Psychoacoustic floor-seed curves and spectrum-peak tracking.
//!
//! Mirrors Python `wwise_wem/analysis/psychoacoustics/seed.py`. Float32
//! storage boundaries are `f32_of`; chase/clamp ordering is preserved.

use crate::config::{
    f32_of, AnalysisError, TONE_BAND_COUNT, TONE_LEVEL_COUNT, WwisePsyLongSeedLook,
    WwisePsyLongTables, NEGATIVE_INFINITY_DB, SPECTRUM_PEAK_DECAY_DB_PER_SECOND,
    EHMER_OFFSET, TONE_REFERENCE_LEVEL_DB, WwisePsyLook,
};

/// Persisted cross-frame spectrum-peak state
/// (Python `SpectrumPeakState`).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct SpectrumPeakState {
    pub value: f64,
}

impl Default for SpectrumPeakState {
    fn default() -> Self {
        Self {
            value: NEGATIVE_INFINITY_DB as f64,
        }
    }
}

impl SpectrumPeakState {
    /// A fresh state (Python dataclass default).
    pub fn new() -> Self {
        Self::default()
    }
}

/// Apply one selected tone curve to a seeded surface
/// (Python `wwise_seed_curve`).
fn wwise_seed_curve(
    seed: &mut [f64],
    curves: &[Vec<f64>],
    amp: f64,
    octave: i64,
    total_octave_lines: i64,
    eighth_octave_lines: i64,
    db_offset: f64,
) -> Result<(), AnalysisError> {
    let mut choice =
        ((amp + db_offset - TONE_REFERENCE_LEVEL_DB as f64) * 0.1) as i64;
    choice = choice.clamp(0, TONE_LEVEL_COUNT - 1);
    if (choice as usize) >= curves.len() {
        return Err(AnalysisError::ToneCurveBankShort {
            want: choice,
            got: curves.len() as i64,
        });
    }
    let posts = &curves[choice as usize];
    if posts.len() < 2 {
        return Err(AnalysisError::ToneCurvePostArrayShort);
    }
    let post0 = posts[0] as i64;
    let post1 = posts[1] as i64;
    let mut seed_ptr =
        octave + (post0 - EHMER_OFFSET) * eighth_octave_lines - (eighth_octave_lines >> 1);
    let curve = &posts[2..];
    for post in post0..post1 {
        if seed_ptr > 0 && seed_ptr < (seed.len() as i64) {
            let curve_index = post;
            if (curve_index as usize) >= curve.len() {
                return Err(AnalysisError::ToneCurvePostsShort);
            }
            seed[seed_ptr as usize] =
                seed[seed_ptr as usize].max(f32_of(amp + curve[curve_index as usize]));
        }
        seed_ptr += eighth_octave_lines;
        if seed_ptr >= total_octave_lines {
            break;
        }
    }
    Ok(())
}

/// Chase a seeded curve forward (Python `wwise_seed_chase`).
#[allow(clippy::needless_range_loop)]
fn wwise_seed_chase(seeds: &mut [f64], eighth_octave_lines: i64, total_octave_lines: i64) {
    let mut positions: Vec<i64> = Vec::new();
    let mut amplitudes: Vec<f64> = Vec::new();
    let limit = (total_octave_lines as usize).min(seeds.len());
    for i in 0..limit {
        let value = seeds[i];
        if positions.len() < 2 {
            positions.push(i as i64);
            amplitudes.push(value);
            continue;
        }
        loop {
            let last_amp = amplitudes[amplitudes.len() - 1];
            if value < last_amp {
                positions.push(i as i64);
                amplitudes.push(value);
                break;
            }
            let i_i64 = i as i64;
            let prev_pos = positions[positions.len() - 1];
            if i_i64 < prev_pos + eighth_octave_lines
                && positions.len() > 1
                    && amplitudes[amplitudes.len() - 1] <= amplitudes[amplitudes.len() - 2]
                    && i_i64 < positions[positions.len() - 2] + eighth_octave_lines
                {
                    positions.pop();
                    amplitudes.pop();
                    continue;
                }
            positions.push(i as i64);
            amplitudes.push(value);
            break;
        }
    }
    let mut pos = 0i64;
    for (stack_index, &start) in positions.iter().enumerate() {
        let end = if stack_index + 1 < positions.len()
            && amplitudes[stack_index + 1] > amplitudes[stack_index]
        {
            positions[stack_index + 1]
        } else {
            start + eighth_octave_lines + 1
        };
        let end = end.min(total_octave_lines);
        for cursor in pos..end {
            seeds[cursor as usize] = f32_of(amplitudes[stack_index]);
        }
        pos = pos.max(end);
    }
}

/// Apply the chased-seed floor to a floor curve
/// (Python `wwise_apply_max_seed_floor`).
fn wwise_apply_max_seed_floor(
    seed: &[f64],
    floor_curve: &mut [f64],
    octave: &[i64],
    first_octave: i64,
    eighth_octave_lines: i64,
    total_octave_lines: i64,
    seed_ceiling: f64,
) -> Result<(), AnalysisError> {
    if octave.len() != floor_curve.len() {
        return Err(AnalysisError::OctaveFloorLengthMismatch);
    }
    if (seed.len() as i64) < total_octave_lines {
        return Err(AnalysisError::SeedSurfaceShort {
            want: total_octave_lines,
        });
    }
    if octave.is_empty() {
        return Ok(());
    }
    let mut pos = octave[0] - first_octave - (eighth_octave_lines >> 1);
    let mut linpos = 0i64;
    let n = octave.len() as i64;
    while linpos + 1 < n {
        if !(pos >= 0 && pos < total_octave_lines) {
            return Err(AnalysisError::SeedCursorOutOfRange {
                cursor: pos,
                total: total_octave_lines,
            });
        }
        let mut min_value = seed[pos as usize];
        let mut end = ((octave[linpos as usize] + octave[(linpos + 1) as usize]) >> 1) - first_octave;
        if min_value > seed_ceiling {
            min_value = seed_ceiling;
        }
        while pos < end {
            pos += 1;
            let s = seed[pos as usize];
            if (s > NEGATIVE_INFINITY_DB as f64 && s < min_value)
                || min_value == NEGATIVE_INFINITY_DB as f64
            {
                min_value = s;
            }
        }
        end = pos + first_octave;
        while linpos < n && octave[linpos as usize] <= end {
            if floor_curve[linpos as usize] < min_value {
                floor_curve[linpos as usize] = f32_of(min_value);
            }
            linpos += 1;
        }
    }
    let min_value = seed[(total_octave_lines - 1) as usize];
    while linpos < n {
        if floor_curve[linpos as usize] < min_value {
            floor_curve[linpos as usize] = f32_of(min_value);
        }
        linpos += 1;
    }
    Ok(())
}

/// Chase and apply the max seed floor (Python `wwise_max_seeds`).
fn wwise_max_seeds(
    seed: &mut [f64],
    floor_curve: &mut [f64],
    octave: &[i64],
    first_octave: i64,
    eighth_octave_lines: i64,
    total_octave_lines: i64,
    seed_ceiling: f64,
) -> Result<(), AnalysisError> {
    if octave.len() != floor_curve.len() {
        return Err(AnalysisError::OctaveFloorLengthMismatch);
    }
    wwise_seed_chase(seed, eighth_octave_lines, total_octave_lines);
    wwise_apply_max_seed_floor(
        seed,
        floor_curve,
        octave,
        first_octave,
        eighth_octave_lines,
        total_octave_lines,
        seed_ceiling,
    )
}

/// Run the long seed-loop over octave groups
/// (Python `wwise_seed_loop`).
#[allow(clippy::too_many_arguments)]
fn wwise_seed_loop(
    curves: &[Vec<Vec<f64>>],
    spectrum: &[f64],
    floor_curve: &mut [f64],
    seed: &mut [f64],
    octave: &[i64],
    first_octave: i64,
    shift_octave: i64,
    total_octave_lines: i64,
    eighth_octave_lines: i64,
    db_offset: f64,
) -> Result<(), AnalysisError> {
    let n = spectrum.len();
    if floor_curve.len() != n || octave.len() != n {
        return Err(AnalysisError::SeedLoopInputLengthMismatch {
            want: n as i64,
        });
    }
    let mut i = 0;
    while i < n {
        let mut peak = spectrum[i];
        let group_octave = octave[i];
        while i + 1 < n && octave[i + 1] == group_octave {
            i += 1;
            if spectrum[i] > peak {
                peak = spectrum[i];
            }
        }
        if peak + 6.0 > floor_curve[i] {
            let mut band = group_octave >> shift_octave;
            band = band.clamp(0, TONE_BAND_COUNT - 1);
            if (band as usize) >= curves.len() {
                return Err(AnalysisError::ToneCurveBandBankShort {
                    want: band,
                    got: curves.len() as i64,
                });
            }
            wwise_seed_curve(
                seed,
                &curves[band as usize],
                peak,
                octave[i] - first_octave,
                total_octave_lines,
                eighth_octave_lines,
                db_offset,
            )?;
        }
        i += 1;
    }
    Ok(())
}

/// Compute per-curve maxima and their global maximum
/// (Python `compute_spectrum_peak`).
pub fn compute_spectrum_peak(
    fft_curves: &[Vec<f64>],
    initial_global: f64,
) -> Result<(Vec<f64>, f64), AnalysisError> {
    let mut global_max = f32_of(initial_global);
    let mut local_maxima = Vec::with_capacity(fft_curves.len());
    for curve in fft_curves {
        if curve.is_empty() {
            return Err(AnalysisError::FftCurveEmpty);
        }
        let local = f32_of(
            0.0f64.min(curve.iter().fold(f64::NEG_INFINITY, |a, b| a.max(*b))),
        );
        local_maxima.push(local);
        if local > global_max {
            global_max = local;
        }
    }
    Ok((local_maxima, global_max))
}

/// Fold one FFT frame into the persisted spectrum peak
/// (Python `update_frame_spectrum_peak`).
pub fn update_frame_spectrum_peak(
    fft_curves: &[Vec<f64>],
    block_bins: i64,
    sample_rate: i64,
    state: &mut SpectrumPeakState,
) -> Result<(Vec<f64>, f64), AnalysisError> {
    if block_bins <= 0 || sample_rate <= 0 {
        return Err(AnalysisError::SpecmaxGeometry {
            block_bins,
            sample_rate,
        });
    }
    let decay = f32_of(
        SPECTRUM_PEAK_DECAY_DB_PER_SECOND as f64 * (block_bins as f64) / (sample_rate as f64),
    );
    let decayed = f32_of(state.value - decay);
    let (local, global_max) = compute_spectrum_peak(fft_curves, decayed)?;
    state.value = global_max;
    Ok((local, global_max))
}


/// Build the floor seed from a long seed look
/// (Python `build_long_floor_seed_from_look`).
pub fn build_long_floor_seed_from_look(
    look: &WwisePsyLongSeedLook,
    logfft: &[f64],
    channel_specmax: f64,
    global_specmax: f64,
) -> Result<Vec<f64>, AnalysisError> {
    if logfft.len() as i64 != look.n {
        return Err(AnalysisError::LongSeedLogFftLength {
            want: look.n,
            got: logfft.len() as i64,
        });
    }
    let ath_shift =
        ((look.ath_offset as f64) + channel_specmax).max(look.ath_floor as f64);
    let initial: Vec<f64> = look
        .base_curve
        .iter()
        .map(|value| f32_of(*value as f64 + ath_shift))
        .collect();
    let tone_banks: Vec<Vec<Vec<f64>>> = look
        .tone_banks
        .iter()
        .map(|bank| {
            bank.iter()
                .map(|curve| curve.iter().map(|v| *v as f64).collect())
                .collect()
        })
        .collect();
    wwise_seed_floor(
        logfft,
        &initial,
        &tone_banks,
        &look.group_labels,
        look.first_octave,
        look.shift_octave,
        look.total_octave_lines,
        look.eighth_octave_lines,
        look.max_curve_db as f64,
        global_specmax,
        look.seed_ceiling as f64,
    )
}

/// Convenience wrapper to build a floor seed from long tables
/// (Python `build_long_floor_seed`).
pub fn build_long_floor_seed(
    table: &WwisePsyLongTables,
    logfft: &[f64],
    channel_specmax: f64,
    global_specmax: f64,
) -> Result<Vec<f64>, AnalysisError> {
    let look = crate::config::make_wwise_long_seed_look(table)?;
    build_long_floor_seed_from_look(&look, logfft, channel_specmax, global_specmax)
}

/// Build the seed floor (Python `wwise_seed_floor`).
#[allow(clippy::too_many_arguments)]
fn wwise_seed_floor(
    spectrum: &[f64],
    floor_curve: &[f64],
    curves: &[Vec<Vec<f64>>],
    octave: &[i64],
    first_octave: i64,
    shift_octave: i64,
    total_octave_lines: i64,
    eighth_octave_lines: i64,
    max_curve_db: f64,
    specmax: f64,
    seed_ceiling: f64,
) -> Result<Vec<f64>, AnalysisError> {
    if spectrum.len() != floor_curve.len() || octave.len() != spectrum.len() {
        return Err(AnalysisError::SeedLoopInputLengthMismatch {
            want: spectrum.len() as i64,
        });
    }
    if total_octave_lines <= 0 {
        return Err(AnalysisError::SeedTotalLinesNonPositive {
            total: total_octave_lines,
        });
    }
    let mut seed = vec![NEGATIVE_INFINITY_DB as f64; total_octave_lines as usize];
    let mut out: Vec<f64> = floor_curve.iter().map(|v| f32_of(*v)).collect();
    let db_offset = max_curve_db - specmax;
    wwise_seed_loop(
        curves,
        spectrum,
        &mut out,
        &mut seed,
        octave,
        first_octave,
        shift_octave,
        total_octave_lines,
        eighth_octave_lines,
        db_offset,
    )?;
    wwise_max_seeds(
        &mut seed,
        &mut out,
        octave,
        first_octave,
        eighth_octave_lines,
        total_octave_lines,
        seed_ceiling,
    )?;
    Ok(out)
}

/// Build the initial ATH curve for one short channel
/// (Python `_wwise_seed_initial_curve`).
fn wwise_seed_initial_curve(
    look: &WwisePsyLook,
    channel_specmax: f64,
) -> Result<Vec<f64>, AnalysisError> {
    if look.ath.len() as i64 != look.n {
        return Err(AnalysisError::LongSeedGeometry);
    }
    let ath_shift =
        ((look.ath_offset as f64) + channel_specmax).max(look.ath_floor as f64);
    Ok(look
        .ath
        .iter()
        .map(|value| f32_of(*value as f64 + ath_shift))
        .collect())
}

/// Build the seed floor from a short seed look
/// (Python `wwise_seed_floor_from_look`).
pub fn wwise_seed_floor_from_look(
    look: &WwisePsyLook,
    spectrum: &[f64],
    channel_specmax: f64,
    global_specmax: f64,
) -> Result<Vec<f64>, AnalysisError> {
    if spectrum.len() as i64 != look.n {
        return Err(AnalysisError::LongSeedGeometry);
    }
    if look.tone_curves.is_empty() {
        return Err(AnalysisError::ShortVectorsLength { want: 1 });
    }
    let initial = wwise_seed_initial_curve(look, channel_specmax)?;
    let tone_curves: Vec<Vec<Vec<f64>>> = look
        .tone_curves
        .iter()
        .map(|bank| {
            bank.iter()
                .map(|curve| curve.iter().map(|v| *v as f64).collect())
                .collect()
        })
        .collect();
    wwise_seed_floor(
        spectrum,
        &initial,
        &tone_curves,
        &look.octave,
        look.first_octave,
        look.shift_octave,
        look.total_octave_lines,
        look.eighth_octave_lines,
        look.max_curve_db as f64,
        global_specmax,
        look.seed_ceiling as f64,
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn compute_spectrum_peak_basic() {
        let curves = vec![
            vec![1.0f64, 2.0, -5.0],
            vec![3.0, 4.0],
        ];
        let (local, global) = compute_spectrum_peak(&curves, 0.0).unwrap();
        assert_eq!(local.len(), 2);
        // min(0, max(...)) then f32
        assert!(global >= 0.0);
    }
}
