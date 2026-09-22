//! Use-case adapters for the WEM encode flow (wem-core).
//!
//! Mirrors the Python `wwise_wem/adapters` package: thin, typed input
//! adapters on top of the deep encoder.

/// Minimal signed-16 PCM WAV reader
/// (Python `wwise_wem/adapters/wav.py::read_pcm_wav`).
///
/// Hand-rolled to keep the dependency surface locked: it reads the RIFF
/// chunk list, requires a PCM (`format 1`) 16-bit `fmt ` chunk and a
/// `data` chunk, and keeps the interleaved sample bytes as they stand in
/// the file.
use std::path::Path;

use crate::error::EncoderError;

/// One uncompressed signed-16 PCM WAV file.
///
/// The `data` chunk's own form is interleaved little-endian signed-16 bytes,
/// so that is what is stored: nothing re-encodes a decoded copy of the
/// samples. Both consumers read this one shape — the streaming API lends the
/// slice out, and `to_pcm16` hands it to the kernel, which decodes it exactly
/// once on the way to the analysis boundary.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Wav16 {
    sample_rate: i64,
    channels: usize,
    /// The `data` chunk's interleaved little-endian signed-16 bytes, trimmed
    /// to whole samples (an odd trailing byte is not a sample).
    data: Vec<u8>,
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
        // zero-channel files and the field is private.
        self.data.len() / (self.channels * 2)
    }

    /// The `data` chunk's interleaved little-endian signed-16 PCM bytes (the
    /// interleaved wire form of the streaming API) — a borrow of the file's
    /// own form, so the name stays honest without a `to_` prefix.
    pub fn interleaved_le_bytes(&self) -> &[u8] {
        &self.data
    }

    /// Convert into a [`Pcm16`](crate::encoder::Pcm16) in that same
    /// interleaved little-endian shape (Python `read_pcm_wav` -> `PcmBuffer`
    /// normalization is applied later at the analysis boundary).
    ///
    /// Takes `&self` because the borrow is what the caller has: `Pcm16` owns
    /// its samples, so this copies the file's bytes into that storage.
    pub fn to_pcm16(&self) -> Result<crate::encoder::Pcm16, EncoderError> {
        // Only whole frames are a PCM buffer: a data chunk whose last frame is
        // cut short contributes the frames before it, exactly as the frame
        // count above reports.
        let whole_frames = self.frames() * self.channels * 2;
        crate::encoder::Pcm16::from_interleaved_le(
            self.sample_rate,
            self.channels,
            &self.data[..whole_frames],
        )
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
        // The size field is caller-chosen, so the chunk end is computed with
        // a checked add: on a 32-bit target (wasm32) a declared size near
        // u32::MAX wraps `body + size`, which would turn the bound check into
        // a slice panic instead of the typed rejection below.
        let end = match body.checked_add(size) {
            Some(end) => end,
            None => return Err(format_error()),
        };
        if end > raw.len() {
            return Err(format_error());
        }
        let payload = &raw[body..end];
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
        // `end` is at most `raw.len()`, so the word-alignment step cannot
        // overflow either.
        offset = end + (size & 1);
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
    // The file's own form, taken as it is: the bytes are the samples, so no
    // decode pass runs here. Only a trailing odd byte is dropped — it is half
    // of a sample and was never one.
    let whole_samples = (data.len() / 2) * 2;
    Ok(Wav16 {
        sample_rate,
        channels,
        data: data[..whole_samples].to_vec(),
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
        // The data chunk of `make_wav` is the little-endian encoding of the
        // samples `[1, -2, 3]`. `Wav16` keeps exactly those bytes, so
        // asserting them asserts the decode — with no encoder in the loop.
        assert_eq!(wav.interleaved_le_bytes(), &[1, 0, 0xFE, 0xFF, 3, 0][..]);
        let pcm = wav.to_pcm16().expect("pcm16 converts");
        assert_eq!(pcm.frame_count(), 3);
        assert_eq!(pcm.channel_count(), 1);
    }

    #[test]
    fn skips_odd_intermediate_chunks() {
        let wav = parse_pcm16(&make_wav(1, 16, 2, true)).expect("junk chunk handled");
        assert_eq!(wav.channels(), 2);
        // 6 bytes of data = 3 interleaved i16 samples `[1, -2, 3]`, held as
        // the file's own little-endian bytes.
        assert_eq!(wav.interleaved_le_bytes(), &[1, 0, 0xFE, 0xFF, 3, 0][..]);
        // Two channels of 4 bytes hold one whole frame: the frame count, and
        // the samples `to_pcm16` hands over, stop at that boundary.
        assert_eq!(wav.frames(), 1);
        let pcm = wav.to_pcm16().expect("whole frames convert");
        assert_eq!(pcm.frame_count(), 1);
        assert_eq!(pcm.channel_count(), 2);
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
