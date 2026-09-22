//! Pure byte composition for complete Wwise WEM containers
//! (Python: `wwise_wem/container/wem.py`).

use crate::error::ContainerError;
use crate::fmt::{VorbisFmtFields, WWISE_VORBIS_FMT_SIZE, WWISE_VORBIS_FORMAT_TAG};
use crate::packets::{
    build_packet_stream, extract_packets, recompute_vorbis_fmt_sizes, PacketWalk,
};
use crate::riff::{build_riff, parse_chunks, Endian};

/// A (fourcc, payload) chunk to embed in the RIFF container.
pub type RiffChunkTuple = ([u8; 4], Vec<u8>);

/// Built WEM bytes plus the fmt/data segment payloads
/// (the Rust-side convenience for segment hashing).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WemBuildResult {
    pub wem_bytes: Vec<u8>,
    pub fmt_raw: Vec<u8>,
    pub data_raw: Vec<u8>,
}

/// Build complete Wwise Vorbis WEM bytes
/// (Python `build_vorbis_wem`).
///
/// `fmt_fields` is consumed with packet-derived sizes/offsets recomputed when
/// `recompute_sizes && fmt_raw.is_none()`. When `fmt_raw` is provided and
/// `recompute_sizes` is set, the 0x1C/0x20/0x28/0x2C/0x30 offset fields are
/// back-filled in place (little-endian regardless of container endian,
/// matching the Python implementation).
pub fn build_vorbis_wem(
    fmt_fields: VorbisFmtFields,
    packets: &[Vec<u8>],
    seek_table: &[u8],
    endian: Endian,
    extra_chunks: &[RiffChunkTuple],
    recompute_sizes: bool,
    fmt_raw: Option<&[u8]>,
) -> Result<WemBuildResult, ContainerError> {
    let mut fields = fmt_fields;
    if recompute_sizes && fmt_raw.is_none() {
        recompute_vorbis_fmt_sizes(
            &mut fields,
            &packets.iter().map(|p| p.as_slice()).collect::<Vec<_>>(),
            seek_table,
        )?;
    }
    let mut fmt_payload = match fmt_raw {
        Some(raw) => raw.to_vec(),
        None => fields.pack(),
    };
    if fmt_payload.len() != WWISE_VORBIS_FMT_SIZE {
        return Err(ContainerError::FmtSize {
            got: fmt_payload.len(),
        });
    }
    let data_payload = build_packet_stream(
        &packets.iter().map(|p| p.as_slice()).collect::<Vec<_>>(),
        seek_table,
        endian,
    )?;
    if recompute_sizes && fmt_raw.is_some() {
        write_le_u32(&mut fmt_payload, 0x20, data_payload.len() as u32);
        write_le_u32(&mut fmt_payload, 0x28, seek_table.len() as u32);
        let setup_offset = seek_table.len() + packets.first().map(|p| 2 + p.len()).unwrap_or(0);
        write_le_u32(&mut fmt_payload, 0x1C, setup_offset as u32);
        write_le_u32(&mut fmt_payload, 0x2C, setup_offset as u32);
        // larger *audio* packet only: the setup packet is not counted (see
        // ``recompute_vorbis_fmt_sizes``).
        if let Some(max) = packets.iter().skip(1).map(|p| p.len()).max() {
            write_le_u16(&mut fmt_payload, 0x30, max as u16);
        }
    }
    let mut chunks: Vec<RiffChunkTuple> = vec![(*b"fmt ", fmt_payload.clone())];
    for (id, payload) in extra_chunks {
        chunks.push((*id, payload.clone()));
    }
    chunks.push((*b"data", data_payload.clone()));
    let wem_bytes = build_riff(&chunks, endian, false)?;
    Ok(WemBuildResult {
        wem_bytes,
        fmt_raw: fmt_payload,
        data_raw: data_payload,
    })
}

/// Patch a u32 into `buf` at `off`, **always little-endian**.
///
/// Pinned little-endian on purpose — it is not a duplicate of
/// [`Endian::push_u32`], which follows the container's byte order, and the two
/// must never be merged. These fields are patched in place after the fmt
/// payload already exists, and the oracle patches them with
/// `struct.pack_into("<I", …)` unconditionally (`container/fmt.py`), even for
/// a big-endian (`RIFX`) container. Merging them would make a big-endian WEM
/// diverge from the oracle's bytes, and **no test would catch it**: no
/// installed profile is big-endian, so the parity suites only ever exercise
/// the little-endian path where the two spellings agree.
fn write_le_u32(buf: &mut [u8], off: usize, value: u32) {
    buf[off..off + 4].copy_from_slice(&value.to_le_bytes());
}

/// Patch a u16 into `buf` at `off`, **always little-endian**; same pinning as
/// [`write_le_u32`] and for the same reason, against
/// [`Endian::push_u16`]. The oracle's `struct.pack_into("<H", …)` is
/// unconditional, even for a big-endian container, and no test would catch a
/// merge.
fn write_le_u16(buf: &mut [u8], off: usize, value: u16) {
    buf[off..off + 2].copy_from_slice(&value.to_le_bytes());
}

/// Structural WEM parts (Python `load_wem_parts_bytes`, reduced to the
/// Vorbis fields the encoder parity tests consume).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WemParts {
    pub endian: Endian,
    /// true when the container is a Wwise Vorbis WEM.
    pub is_wwise_vorbis: bool,
    pub fmt_raw: Vec<u8>,
    pub data_raw: Vec<u8>,
    pub fmt: VorbisFmtFields,
    pub seek_table: Vec<u8>,
    pub setup_packet: Option<Vec<u8>>,
    pub audio_packets: Vec<Vec<u8>>,
    pub sizes: Vec<u16>,
    /// Chunk ids in container order.
    pub chunk_ids: Vec<[u8; 4]>,
}

/// Load structural WEM parts from bytes (Python `load_wem_parts_bytes`).
pub fn load_wem_parts_bytes(raw: &[u8]) -> Result<WemParts, ContainerError> {
    let (endian, chunks) = parse_chunks(raw)?;
    let fmt_chunk = chunks
        .iter()
        .find(|c| c.id == *b"fmt ")
        .ok_or(ContainerError::MissingChunk { id: "fmt" })?;
    let data_chunk = chunks
        .iter()
        .find(|c| c.id == *b"data")
        .ok_or(ContainerError::MissingChunk { id: "data" })?;

    let fmt_payload = &fmt_chunk.payload;
    if fmt_payload.len() < 2 {
        return Err(ContainerError::FmtTooShort {
            got: fmt_payload.len(),
        });
    }
    let tag = u16::from_le_bytes([fmt_payload[0], fmt_payload[1]]);
    let mut is_wwise_vorbis = false;
    let mut seek_table = Vec::new();
    let mut setup_packet = None;
    let mut audio_packets = Vec::new();
    let mut sizes = Vec::new();

    if tag == WWISE_VORBIS_FORMAT_TAG {
        let fmt = VorbisFmtFields::parse(fmt_payload)?;
        // A payload the walk cannot consume is an error, not a walk with a
        // flag on it: `?` is the whole handling, and the error already carries
        // where the walk stopped and what it saw there.
        let walk: PacketWalk =
            extract_packets(&data_chunk.payload, fmt.dw_seek_table_size as usize, endian)?;
        is_wwise_vorbis = true;
        setup_packet = walk.setup_packet().map(|p| p.to_vec());
        seek_table = walk.seek_table;
        audio_packets = walk.packets.into_iter().skip(1).collect();
        sizes = walk.sizes;
    }

    Ok(WemParts {
        endian,
        is_wwise_vorbis,
        fmt_raw: fmt_payload.clone(),
        data_raw: data_chunk.payload.clone(),
        fmt: VorbisFmtFields::parse(fmt_payload)?,
        seek_table,
        setup_packet,
        audio_packets,
        sizes,
        chunk_ids: chunks.iter().map(|c| c.id).collect(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample_fields() -> VorbisFmtFields {
        let mut f = VorbisFmtFields::DEFAULTS;
        f.n_channels = 2;
        f.n_samples_per_sec = 44100;
        f
    }

    #[test]
    fn build_vorbis_wem_recompute_roundtrip() {
        let setup = vec![1u8; 10];
        let audio1 = vec![2u8; 4];
        let audio2 = vec![3u8; 7];
        let seek = vec![9u8; 8];
        let res = build_vorbis_wem(
            sample_fields(),
            &[setup.clone(), audio1.clone(), audio2.clone()],
            &seek,
            Endian::Little,
            &[],
            true,
            None,
        )
        .unwrap();

        let parts = load_wem_parts_bytes(&res.wem_bytes).unwrap();
        assert!(parts.is_wwise_vorbis);
        assert_eq!(parts.chunk_ids, vec![*b"fmt ", *b"data"]);
        assert_eq!(parts.fmt_raw, res.fmt_raw);
        assert_eq!(parts.data_raw, res.data_raw);
        assert_eq!(parts.setup_packet, Some(setup.clone()));
        assert_eq!(parts.audio_packets, vec![audio1.clone(), audio2.clone()]);
        assert_eq!(parts.seek_table, seek.clone());

        // recomputed fmt offsets:
        // data size = 8 + (2+10) + (2+4) + (2+7) = 35
        // first audio in data = 8 + 2 + 10 = 20
        assert_eq!(parts.fmt.dw_seek_table_size, 8);
        assert_eq!(parts.fmt.dw_data_payload_size, 35);
        assert_eq!(parts.fmt.dw_first_audio_packet_offset, 20);
        assert_eq!(parts.fmt.dw_vorbis_data_offset, 20);
        // largest *audio* packet (4 and 7), not the 10-byte setup packet
        assert_eq!(parts.fmt.u_max_packet_size, 7);
        assert_eq!(parts.fmt.w_format_tag, WWISE_VORBIS_FORMAT_TAG);

        // WAVE body: RIFF(8) + WAVE(4) + fmt(8+66) + data(8+35)
        assert_eq!(res.wem_bytes.len(), 8 + 4 + 74 + 43);
    }

    #[test]
    fn build_vorbis_wem_with_extra_chunks_order() {
        let setup = vec![1u8; 5];
        let res = build_vorbis_wem(
            sample_fields(),
            std::slice::from_ref(&setup),
            &[],
            Endian::Little,
            &[(*b"junk", vec![0u8; 3])],
            true,
            None,
        )
        .unwrap();
        let parts = load_wem_parts_bytes(&res.wem_bytes).unwrap();
        assert_eq!(parts.chunk_ids, vec![*b"fmt ", *b"junk", *b"data"]);
        // odd intermediate junk payload (3 bytes) is padded to a 4-byte chunk.
        let expected = 8 + 4 + (8 + 66) + (8 + 4) + (8 + 2 + setup.len());
        assert_eq!(res.wem_bytes.len(), expected);
    }

    #[test]
    fn build_vorbis_wem_fmt_raw_backfill() {
        // fmt_raw path with recompute_sizes: offsets back-filled in place.
        let setup = vec![1u8; 6];
        let fields = sample_fields();
        let mut raw = fields.pack();
        // Poison the offset fields so the test proves the back-fill.
        for (off, len) in [(0x1C, 4usize), (0x20, 4), (0x28, 4), (0x2C, 4), (0x30, 2)] {
            raw[off..off + len].fill(0xEE);
        }
        let res = build_vorbis_wem(
            fields,
            std::slice::from_ref(&setup),
            &[],
            Endian::Little,
            &[],
            true,
            Some(&raw),
        )
        .unwrap();
        let parts = load_wem_parts_bytes(&res.wem_bytes).unwrap();
        assert_eq!(
            parts.fmt.dw_first_audio_packet_offset,
            2 + setup.len() as u32
        );
        assert_eq!(parts.fmt.dw_data_payload_size, 2 + setup.len() as u32);
        assert_eq!(parts.fmt.dw_seek_table_size, 0);
        // no audio packets: the field keeps its incoming value, mirroring the
        // Python reference's named "preserve existing max" case
        assert_eq!(parts.fmt.u_max_packet_size, 0xEEEE);
    }
}
