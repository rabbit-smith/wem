//! Build the LPC-padded PCM timeline consumed by the transient detector.
//!
//! Mirrors Python `wwise_wem/analysis/preprocessing/detector_input.py`.

use crate::config::{f32_of, AnalysisError};
use crate::dsp::lpc::{wwise_first_frame_lpc_prime, wwise_lpc_from_data, wwise_lpc_predict};

/// Build the absolute PCM streams consumed by the transient detector
/// (Python `detector_pcm_streams`).
pub fn detector_pcm_streams(
    pcm: &[Vec<f64>],
    prefix_samples: Option<i64>,
    terminal_samples: i64,
    blocksizes: &[i64],
) -> Result<Vec<Vec<f64>>, AnalysisError> {
    if pcm.is_empty() {
        return Err(AnalysisError::DetectorPcmEmpty);
    }
    let source_len = pcm[0].len() as i64;
    if source_len < 4096 || pcm.iter().any(|c| c.len() as i64 != source_len) {
        return Err(AnalysisError::DetectorPcmShort { frames: source_len });
    }
    if terminal_samples < 0 {
        return Err(AnalysisError::DetectorTerminalNegative {
            terminal: terminal_samples,
        });
    }
    let prefix_samples = match prefix_samples {
        Some(p) => p,
        None => blocksizes[1] / 2,
    };
    if prefix_samples < 1 {
        return Err(AnalysisError::DetectorPrefixNonPositive {
            prefix: prefix_samples,
        });
    }

    let mut result = Vec::with_capacity(pcm.len());
    for source in pcm {
        let channel: Vec<f64> = source.iter().map(|v| f32_of(*v)).collect();
        let prefix =
            wwise_first_frame_lpc_prime(&channel, prefix_samples, 4096, 16)?;
        let len = channel.len();
        let coeffs = wwise_lpc_from_data(&channel[len - 4096..], 32)?;
        let tail = wwise_lpc_predict(&coeffs, &channel[len - 32..], terminal_samples)?;
        let mut stream = prefix;
        stream.extend_from_slice(&channel);
        stream.extend_from_slice(&tail);
        result.push(stream);
    }
    Ok(result)
}

/// Yield channel-major transient-detector PCM windows on the absolute
/// detector timeline (Python `iter_detector_quanta`), returning all quanta.
pub fn iter_detector_quanta(
    pcm: &[Vec<f64>],
    hop: i64,
    window: i64,
    count: Option<i64>,
    terminal_samples: i64,
    blocksizes: &[i64],
) -> Result<Vec<Vec<Vec<f64>>>, AnalysisError> {
    if hop <= 0 || window <= 0 {
        return Err(AnalysisError::DetectorHopWindowInvalid { hop, window });
    }
    let streams = detector_pcm_streams(
        pcm,
        None,
        terminal_samples,
        blocksizes,
    )?;
    let available = (streams[0].len() as i64 - window) / hop + 1;
    let count = match count {
        Some(c) => c,
        None => available,
    };
    if count < 0 || count > available {
        return Err(AnalysisError::DetectorQuantaOutOfRange {
            requested: count,
            available,
        });
    }
    let mut quanta = Vec::with_capacity(count as usize);
    for quantum in 0..count {
        let start = (quantum * hop) as usize;
        let rows = streams
            .iter()
            .map(|channel| channel[start..start + window as usize].to_vec())
            .collect();
        quanta.push(rows);
    }
    Ok(quanta)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn pcm(channels: usize, frames: usize) -> Vec<Vec<f64>> {
        (0..channels)
            .map(|_| (0..frames).map(|i| (i as f64) * 0.001).collect::<Vec<f64>>())
            .collect()
    }

    #[test]
    fn detector_streams_lengths() {
        let p = pcm(2, 5000);
        let streams =
            detector_pcm_streams(&p, None, 8192, &[256, 2048]).expect("streams");
        // 1024 prefix + 5000 + 8192 tail
        assert_eq!(streams[0].len() as i64, 1024 + 5000 + 8192);
    }

    #[test]
    fn detector_quanta_available() {
        let p = pcm(2, 5000);
        let quanta =
            iter_detector_quanta(&p, 64, 128, None, 8192, &[256, 2048]).expect("quanta");
        // (len - 128) // 64 + 1
        let stream_len = 1024 + 5000 + 8192;
        let available = (stream_len - 128) / 64 + 1;
        assert_eq!(quanta.len() as i64, available);
        assert_eq!(quanta[0][0].len(), 128);
    }

    #[test]
    fn detector_rejects() {
        let p = pcm(2, 5000);
        assert!(detector_pcm_streams(&[], None, 8192, &[256, 2048]).is_err());
        assert!(detector_pcm_streams(&p, None, -1, &[256, 2048]).is_err());
        assert!(iter_detector_quanta(&p, 0, 128, None, 8192, &[256, 2048]).is_err());
    }
}
