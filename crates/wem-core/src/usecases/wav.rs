//! Use-case adapters for the WEM encode flow (wem-core).
//!
//! Mirrors the Python `wwise_wem/adapters` package: thin, typed input
//! adapters on top of the deep encoder.

/// Minimal signed-16 PCM WAV reader
/// (Python `wwise_wem/adapters/wav.py::read_pcm16`).
///
/// Hand-rolled to keep the dependency surface locked: it reads the RIFF
/// chunk list, requires a PCM (`format 1`) 16-bit `fmt ` chunk and a
/// `data` chunk, and returns the interleaved samples.
use std::path::Path;

use crate::error::EncoderError;

/// One uncompressed signed-16 PCM WAV file.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Wav16 {
    sample_rate: i64,
    channels: usize,
    /// Interleaved samples (frames * channels, in file order).
    samples: Vec<i16>,
}

impl Wav16 {
    pub fn sample_rate(&self) -> i64 {
        self.sample_rate
    }

    pub fn channels(&self) -> usize {
        self.channels
    }

    pub fn frames(&self) -> usize {
        // `channels` is structurally non-zero: the parser rejects
        // zero-channel files, and the field is private.
        self.samples.len() / self.channels
    }

    /// Interleaved little-endian signed-16 PCM bytes (the wwise.v1
    /// `PcmFrames.data` wire form).
    pub fn interleaved_le_bytes(&self) -> Vec<u8> {
        self.samples
            .iter()
            .flat_map(|value| value.to_le_bytes())
            .collect()
    }

    /// Convert into a [`Pcm16`](crate::encoder::Pcm16) with channel-major
    /// rows (Python `read_pcm16` -> `PcmBuffer` normalization is applied
    /// later at the analysis boundary).
    pub fn to_pcm16(&self) -> Result<crate::encoder::Pcm16, EncoderError> {
        let frames = self.frames();
        let mut channels = vec![Vec::with_capacity(frames); self.channels];
        for frame in 0..frames {
            for channel in 0..self.channels {
                channels[channel].push(self.samples[frame * self.channels + channel]);
            }
        }
        crate::encoder::Pcm16::new(self.sample_rate, channels)
    }
}

/// Read an uncompressed signed-16 PCM WAV file.
///
/// `FormatUnsupported` for non-RIFF input, non-PCM layouts, or non-16-bit
/// sample widths (Python: "encoder input must be uncompressed
/// signed-16 PCM WAV"); `Internal(Io)` for I/O failures.
pub fn read_pcm16(path: &Path) -> Result<Wav16, EncoderError> {
    let raw = std::fs::read(path).map_err(|error| {
        EncoderError::Internal(crate::error::InternalError::Io {
            message: format!("{}: {error}", path.display()),
        })
    })?;
    parse_pcm16(&raw)
}

/// Parse an uncompressed signed-16 PCM WAV from bytes.
pub fn parse_pcm16(raw: &[u8]) -> Result<Wav16, EncoderError> {
    const INVALID: &str = "encoder input must be uncompressed signed-16 PCM WAV";
    let format_error = || EncoderError::FormatUnsupported {
        message: INVALID.into(),
    };

    let read_u16 =
        |raw: &[u8], off: usize| u16::from_le_bytes(raw[off..off + 2].try_into().unwrap());
    let read_u32 =
        |raw: &[u8], off: usize| u32::from_le_bytes(raw[off..off + 4].try_into().unwrap());

    if raw.len() < 12 || &raw[0..4] != b"RIFF" || &raw[8..12] != b"WAVE" {
        return Err(format_error());
    }

    let mut sample_rate: i64 = 0;
    let mut channels: usize = 0;
    let mut data: Option<&[u8]> = None;
    let mut offset = 12usize;
    while offset + 8 <= raw.len() {
        let id: &[u8] = &raw[offset..offset + 4];
        let size = read_u32(raw, offset + 4) as usize;
        let body = offset + 8;
        if body + size > raw.len() {
            return Err(format_error());
        }
        let payload = &raw[body..body + size];
        if &id == b"fmt " {
            if payload.len() < 16 {
                return Err(format_error());
            }
            let audio_format = read_u16(payload, 0);
            let num_channels = read_u16(payload, 2) as usize;
            let samplerate = read_u32(payload, 4) as i64;
            let bits_per_sample = read_u16(payload, 14);
            if audio_format != 1 || bits_per_sample != 16 || num_channels == 0 {
                return Err(format_error());
            }
            sample_rate = samplerate;
            channels = num_channels;
        } else if &id == b"data" && data.is_none() {
            data = Some(payload);
        }
        offset = body + size + (size & 1);
    }

    let data = match data {
        Some(data) if sample_rate > 0 && channels > 0 => data,
        _ => return Err(format_error()),
    };
    let frames = data.len() / (2 * channels);
    if frames == 0 {
        return Err(EncoderError::FormatUnsupported {
            message: "encoder input WAV must contain at least one frame".into(),
        });
    }
    let mut samples = Vec::with_capacity(frames * channels);
    for pair in data.chunks_exact(2) {
        samples.push(i16::from_le_bytes([pair[0], pair[1]]));
    }
    Ok(Wav16 {
        sample_rate,
        channels,
        samples,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Build a minimal RIFF/WAVE file: optional junk chunk (odd payload),
    /// the fmt chunk, then the data chunk.
    fn make_wav(
        audio_format: u16,
        bits_per_sample: u16,
        channels: u16,
        with_junk: bool,
    ) -> Vec<u8> {
        let mut out = Vec::new();
        let mut body = Vec::new();
        if with_junk {
            body.extend_from_slice(b"junk");
            body.extend_from_slice(&3u32.to_le_bytes());
            body.extend_from_slice(&[7u8; 3]);
            body.push(0); // word-align padding
        }
        body.extend_from_slice(b"fmt ");
        body.extend_from_slice(&16u32.to_le_bytes());
        body.extend_from_slice(&audio_format.to_le_bytes());
        body.extend_from_slice(&channels.to_le_bytes());
        body.extend_from_slice(&44100u32.to_le_bytes());
        body.extend_from_slice(&44100u32.to_le_bytes());
        body.extend_from_slice(&2u16.to_le_bytes());
        body.extend_from_slice(&bits_per_sample.to_le_bytes());
        body.extend_from_slice(b"data");
        body.extend_from_slice(&6u32.to_le_bytes());
        body.extend_from_slice(&[1u8, 0, 0xFE, 0xFF, 3u8, 0]);
        out.extend_from_slice(b"RIFF");
        out.extend_from_slice(&(body.len() as u32 + 4).to_le_bytes());
        out.extend_from_slice(b"WAVE");
        out.extend_from_slice(&body);
        out
    }

    #[test]
    fn parses_minimal_pcm16_wav() {
        let wav = parse_pcm16(&make_wav(1, 16, 1, false)).expect("valid wav parses");
        assert_eq!(wav.channels(), 1);
        assert_eq!(wav.sample_rate(), 44100);
        assert_eq!(wav.frames(), 3);
        assert_eq!(wav.samples, vec![1, -2, 3]);
        let pcm = wav.to_pcm16().expect("pcm16 converts");
        assert_eq!(pcm.frame_count(), 3);
        assert_eq!(pcm.channel_count(), 1);
    }

    #[test]
    fn skips_odd_intermediate_chunks() {
        let wav = parse_pcm16(&make_wav(1, 16, 2, true)).expect("junk chunk handled");
        assert_eq!(wav.channels(), 2);
        // 6 bytes of data = 3 interleaved i16 samples.
        assert_eq!(wav.samples, vec![1, -2, 3]);
    }

    #[test]
    fn rejects_non_pcm_layouts() {
        assert!(matches!(
            parse_pcm16(&make_wav(3, 16, 1, false)),
            Err(EncoderError::FormatUnsupported { .. })
        ));
        assert!(matches!(
            parse_pcm16(&make_wav(1, 32, 1, false)),
            Err(EncoderError::FormatUnsupported { .. })
        ));
        assert!(parse_pcm16(b"notriff").is_err());
        assert!(parse_pcm16(b"").is_err());
    }

    #[test]
    fn rejects_truncated_chunks() {
        let mut raw = make_wav(1, 16, 1, false);
        raw.truncate(raw.len() - 2); // cut into the data chunk
        assert!(parse_pcm16(&raw).is_err());
    }
}
