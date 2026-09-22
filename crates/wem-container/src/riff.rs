//! Pure RIFF/RIFX WAVE chunk parsing and construction
//! (Python: `wwise_wem/container/riff.py`).

use crate::error::ContainerError;

/// Byte order of a RIFF container.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Endian {
    /// Little-endian `RIFF` (the Wwise default).
    #[default]
    Little,
    /// Big-endian `RIFX`.
    Big,
}

impl Endian {
    /// Push a u16 with the container's byte order.
    pub fn push_u16(&self, out: &mut Vec<u8>, value: u16) {
        match self {
            Endian::Little => out.extend_from_slice(&value.to_le_bytes()),
            Endian::Big => out.extend_from_slice(&value.to_be_bytes()),
        }
    }

    /// Push a u32 with the container's byte order.
    pub fn push_u32(&self, out: &mut Vec<u8>, value: u32) {
        match self {
            Endian::Little => out.extend_from_slice(&value.to_le_bytes()),
            Endian::Big => out.extend_from_slice(&value.to_be_bytes()),
        }
    }

    /// Read a u32 at `off` with the container's byte order.
    fn read_u32(&self, data: &[u8], off: usize) -> Result<u32, ContainerError> {
        if off + 4 > data.len() {
            return Err(ContainerError::TruncatedRiff { at: off });
        }
        let mut buf = [0u8; 4];
        buf.copy_from_slice(&data[off..off + 4]);
        Ok(match self {
            Endian::Little => u32::from_le_bytes(buf),
            Endian::Big => u32::from_be_bytes(buf),
        })
    }
}

impl Endian {
    fn marker(&self) -> &'static [u8; 4] {
        match self {
            Endian::Little => b"RIFF",
            Endian::Big => b"RIFX",
        }
    }
}

/// One ordered RIFF chunk (Python chunk dict: id/size/off/payload).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ParsedChunk {
    pub id: [u8; 4],
    pub size: u32,
    pub off: usize,
    pub payload: Vec<u8>,
}

/// Parse RIFF/RIFX chunks while preserving the legacy permissive walk
/// (Python `parse_chunks`).
///
/// The declared RIFF extent is clamped to the supplied bytes. A chunk whose
/// declared payload extends past that extent is returned with its declared
/// `size` and the available payload bytes.
pub fn parse_chunks(raw: &[u8]) -> Result<(Endian, Vec<ParsedChunk>), ContainerError> {
    if raw.len() < 4 {
        return Err(ContainerError::NotRiff);
    }
    let endian = if &raw[0..4] == b"RIFF" {
        Endian::Little
    } else if &raw[0..4] == b"RIFX" {
        Endian::Big
    } else {
        return Err(ContainerError::NotRiff);
    };
    let rsize = endian.read_u32(raw, 4)?;
    // The declared extent is permissively clamped to the supplied bytes. On a
    // 32-bit target a u32 RIFF size plus its eight-byte prefix can exceed
    // `usize`; that still means the supplied bytes are the whole readable
    // extent, rather than an arithmetic panic.
    let end = 8usize.saturating_add(rsize as usize).min(raw.len());
    let mut pos = 12usize;
    let mut chunks = Vec::new();
    while pos
        .checked_add(8)
        .is_some_and(|header_end| header_end <= end)
    {
        let mut cid = [0u8; 4];
        cid.copy_from_slice(&raw[pos..pos + 4]);
        let csize = endian.read_u32(raw, pos + 4)?;
        let payload_start = pos + 8;
        let avail = raw.len() - payload_start;
        let take = (csize as usize).min(avail);
        let payload = raw[payload_start..payload_start + take].to_vec();
        chunks.push(ParsedChunk {
            id: cid,
            size: csize,
            off: pos,
            payload,
        });
        // Once a declared chunk end cannot fit in `usize`, no later header can
        // be addressable in this input. Saturating to the end of the address
        // space preserves the permissive walk while stopping cleanly.
        let advance = 8usize
            .saturating_add(csize as usize)
            .saturating_add(csize as usize & 1);
        pos = pos.saturating_add(advance);
    }
    Ok((endian, chunks))
}

/// Build a RIFF/RIFX WAVE byte string from `(fourcc, payload)` chunks
/// (Python `build_riff`).
///
/// Odd intermediate payloads are word-padded. Wwise 2013 output files commonly
/// omit the pad after the final chunk, so that remains the default; pass
/// `pad_final=true` for a strict final pad.
pub fn build_riff(
    chunks: &[(impl AsRef<[u8]>, impl AsRef<[u8]>)],
    endian: Endian,
    pad_final: bool,
) -> Result<Vec<u8>, ContainerError> {
    let mut body = Vec::with_capacity(4);
    body.extend_from_slice(b"WAVE");
    let n = chunks.len();
    for (i, (cid, payload)) in chunks.iter().enumerate() {
        let cid = cid.as_ref();
        let payload = payload.as_ref();
        if cid.len() != 4 {
            return Err(ContainerError::BadChunkId {
                id: format!("{cid:?}"),
            });
        }
        body.extend_from_slice(cid);
        endian.push_u32(&mut body, payload.len() as u32);
        body.extend_from_slice(payload);
        let is_last = i == n - 1;
        if payload.len() & 1 != 0 && (!is_last || pad_final) {
            body.push(0);
        }
    }
    let mut out = Vec::with_capacity(8 + body.len());
    out.extend_from_slice(endian.marker());
    endian.push_u32(&mut out, body.len() as u32);
    out.extend_from_slice(&body);
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn build_riff_roundtrip_le() {
        let bytes = build_riff(
            &[(b"fmt " as &[u8], b"abcd" as &[u8]), (b"data", b"xy")],
            Endian::Little,
            false,
        )
        .unwrap();
        // RIFF + size + WAVE + fmt (8+4) + data (8+2)
        assert_eq!(bytes.len(), 8 + 4 + 12 + 10);
        let (endian, chunks) = parse_chunks(&bytes).unwrap();
        assert_eq!(endian, Endian::Little);
        assert_eq!(chunks.len(), 2);
        assert_eq!(chunks[0].id, *b"fmt ");
        assert_eq!(chunks[0].payload, b"abcd");
        assert_eq!(chunks[1].id, *b"data");
        assert_eq!(chunks[1].payload, b"xy");
    }

    #[test]
    fn build_riff_odd_payload_padding() {
        // Odd intermediate payload is padded; odd final payload is not
        // (the Wwise 2013 default).
        let bytes = build_riff(
            &[(b"junk", b"abc"), (b"data", b"xyz")],
            Endian::Little,
            false,
        )
        .unwrap();
        // body: WAVE(4) + junk header(8)+3+1 + data header(8)+3 = 27
        assert_eq!(&bytes[0..4], b"RIFF");
        assert_eq!(u32::from_le_bytes(bytes[4..8].try_into().unwrap()), 27);
        // padded junk chunk: 8 + 3 + 1
        assert_eq!(bytes[4 + 4 + 8 + 3], 0);

        // pad_final pads the last odd payload too.
        let bytes = build_riff(&[(b"data", b"xyz")], Endian::Little, true).unwrap();
        // body: WAVE(4) + data header(8) + 3 + 1 = 16; RIFF size 16
        assert_eq!(u32::from_le_bytes(bytes[4..8].try_into().unwrap()), 16);
        assert_eq!(&bytes[8..12], b"WAVE");
        assert_eq!(&bytes[12..16], b"data");
        assert_eq!(bytes[16 + 3 + 4], 0); // final pad byte
    }

    #[test]
    fn build_riff_big_endian_marker() {
        let bytes = build_riff(&[(b"data", b"")], Endian::Big, false).unwrap();
        assert_eq!(&bytes[0..4], b"RIFX");
        let (endian, chunks) = parse_chunks(&bytes).unwrap();
        assert_eq!(endian, Endian::Big);
        assert_eq!(chunks[0].id, *b"data");
    }

    #[test]
    fn parse_chunks_rejects_non_riff() {
        assert!(parse_chunks(b"XXXX12345678").is_err());
        assert!(parse_chunks(b"RI").is_err());
    }

    #[test]
    fn parse_chunks_bad_chunk_id_in_build() {
        assert!(build_riff(&[(b"fm", b"")], Endian::Little, false).is_err());
    }

    #[test]
    fn parse_chunks_clamps_a_maximum_riff_size() {
        let mut raw = Vec::from(&b"RIFF"[..]);
        raw.extend_from_slice(&u32::MAX.to_le_bytes());
        raw.extend_from_slice(b"WAVE");

        let (_, chunks) = parse_chunks(&raw).expect("the truncated RIFF extent is permissive");
        assert!(chunks.is_empty());
    }

    #[test]
    fn parse_chunks_keeps_available_payload_for_a_maximum_chunk_size() {
        let mut raw = Vec::from(&b"RIFF"[..]);
        raw.extend_from_slice(&13u32.to_le_bytes());
        raw.extend_from_slice(b"WAVE");
        raw.extend_from_slice(b"data");
        raw.extend_from_slice(&u32::MAX.to_le_bytes());
        raw.push(0xA5);

        let (_, chunks) = parse_chunks(&raw).expect("the truncated chunk remains observable");
        assert_eq!(chunks.len(), 1);
        assert_eq!(chunks[0].id, *b"data");
        assert_eq!(chunks[0].size, u32::MAX);
        assert_eq!(chunks[0].payload, [0xA5]);
    }
}
