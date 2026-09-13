//! Pure byte codecs for Wwise size-prefixed packet streams
//! (Python: `wwise_wem/container/packets.py`).

use crate::error::ContainerError;
use crate::fmt::VorbisFmtFields;
use crate::riff::Endian;

/// Result of splitting a RIFF data payload into seek table and sized
/// packets (Python `extract_packets` result dict).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PacketWalk {
    pub ok: bool,
    pub seek_table: Vec<u8>,
    pub packets: Vec<Vec<u8>>,
    pub sizes: Vec<u16>,
    pub error_at: Option<usize>,
    pub error_size: Option<u16>,
    pub end: usize,
    pub data_size: usize,
    /// Size of the (first) setup packet, if any.
    pub setup_packet_size: Option<u16>,
    /// Byte offset of the first audio packet within the data payload.
    pub first_audio_offset: Option<usize>,
}

impl PacketWalk {
    /// Setup packet (packet 0) of a Wwise Vorbis data payload.
    pub fn setup_packet(&self) -> Option<&[u8]> {
        self.packets.first().map(|p| p.as_slice())
    }
}

fn read_u16(data: &[u8], pos: usize, endian: Endian) -> Result<u16, ContainerError> {
    if pos + 2 > data.len() {
        return Err(ContainerError::TruncatedPacketStream { at: pos });
    }
    let mut buf = [0u8; 2];
    buf.copy_from_slice(&data[pos..pos + 2]);
    Ok(match endian {
        Endian::Little => u16::from_le_bytes(buf),
        Endian::Big => u16::from_be_bytes(buf),
    })
}

/// Split a data payload into its seek table and sized packets
/// (Python `extract_packets`).
pub fn extract_packets(
    data: &[u8],
    seek_table_size: usize,
    endian: Endian,
) -> Result<PacketWalk, ContainerError> {
    if seek_table_size > data.len() {
        return Err(ContainerError::BadSeekTableSize {
            seek_table_size,
            data_size: data.len(),
        });
    }
    let seek = data[..seek_table_size].to_vec();
    let mut sizes = Vec::new();
    let mut packets = Vec::new();
    let mut position = seek_table_size;
    while position + 2 <= data.len() {
        let size = read_u16(data, position, endian)?;
        if position + 2 + size as usize > data.len() {
            return Ok(PacketWalk {
                ok: false,
                seek_table: seek,
                packets,
                sizes,
                error_at: Some(position),
                error_size: Some(size),
                end: position,
                data_size: data.len(),
                setup_packet_size: None,
                first_audio_offset: None,
            });
        }
        let payload = data[position + 2..position + 2 + size as usize].to_vec();
        sizes.push(size);
        packets.push(payload);
        position += 2 + size as usize;
    }
    let setup = sizes.first().copied();
    Ok(PacketWalk {
        ok: position == data.len(),
        seek_table: seek,
        packets,
        sizes,
        error_at: None,
        error_size: None,
        end: position,
        data_size: data.len(),
        setup_packet_size: setup,
        first_audio_offset: setup.map(|s| seek_table_size + 2 + s as usize),
    })
}

/// Build `seek_table + Σ(u16 size + payload)` bytes
/// (Python `build_packet_stream`).
pub fn build_packet_stream(
    packets: &[&[u8]],
    seek_table: &[u8],
    endian: Endian,
) -> Result<Vec<u8>, ContainerError> {
    let mut out = Vec::with_capacity(seek_table.len() + 2 * packets.len());
    out.extend_from_slice(seek_table);
    for packet in packets {
        if packet.len() > u16::MAX as usize {
            return Err(ContainerError::PacketTooLarge { len: packet.len() });
        }
        endian.push_u16(&mut out, packet.len() as u16);
        out.extend_from_slice(packet);
    }
    Ok(out)
}

/// Return fmt fields with packet-derived sizes and offsets updated
/// (Python `recompute_vorbis_fmt_sizes`).
///
/// The typed struct mirrors the Python dict with all keys present, so the
/// Python `fields.get("wFormatTag", default)` pass-through is a no-op here;
/// the tag is preserved exactly.
pub fn recompute_vorbis_fmt_sizes(
    fields: &mut VorbisFmtFields,
    packets: &[&[u8]],
    seek_table: &[u8],
) {
    let data_size = seek_table.len() + packets.iter().map(|p| 2 + p.len()).sum::<usize>();
    let first = packets.first().map(|p| 2 + p.len()).unwrap_or(0);
    let first_in_data = seek_table.len() + first;
    fields.dw_seek_table_size = seek_table.len() as u32;
    fields.dw_data_payload_size = data_size as u32;
    fields.dw_first_audio_packet_offset = first_in_data as u32;
    fields.dw_vorbis_data_offset = first_in_data as u32;
    if let Some(max) = packets.iter().map(|p| p.len()).max() {
        fields.u_max_packet_size = max as u16;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn packet_stream_roundtrip() {
        let seek = b"seek-table";
        let p1 = b"setup-packet";
        let p2 = b"audio";
        let bytes = build_packet_stream(&[p1, p2], seek, Endian::Little).unwrap();
        let walk = extract_packets(&bytes, seek.len(), Endian::Little).unwrap();
        assert!(walk.ok);
        assert_eq!(walk.setup_packet_size, Some(p1.len() as u16));
        assert_eq!(walk.first_audio_offset, Some(seek.len() + 2 + p1.len()));
        assert_eq!(walk.packets, vec![p1.to_vec(), p2.to_vec()]);
        assert_eq!(walk.sizes, vec![p1.len() as u16, p2.len() as u16]);
    }

    #[test]
    fn packet_stream_rejects_too_large() {
        let big = vec![0u8; 65536];
        assert!(build_packet_stream(&[&big], &[], Endian::Little).is_err());
    }

    #[test]
    fn extract_truncated_payload_reports_error() {
        // size claims 10 bytes (LE: 0x0A 0x00), only 3 present -> the walk
        // stops with error info instead of erroring.
        let data = [10u8, 0u8, 1, 2, 3];
        let walk = extract_packets(&data, 0, Endian::Little).unwrap();
        assert!(!walk.ok);
        assert_eq!(walk.error_at, Some(0));
        assert_eq!(walk.error_size, Some(10));
        assert_eq!(walk.packets.len(), 0);
    }

    #[test]
    fn recompute_sizes_matches_python() {
        let mut fields = VorbisFmtFields::DEFAULTS;
        let setup = b"setup";
        let audio = b"packet";
        let seek = b"seekseek";
        recompute_vorbis_fmt_sizes(&mut fields, &[setup.as_ref(), audio.as_ref()], seek);
        // data = 8 + (2+5) + (2+6) = 23
        assert_eq!(fields.dw_seek_table_size, 8);
        assert_eq!(fields.dw_data_payload_size, 23);
        assert_eq!(fields.dw_first_audio_packet_offset, 8 + 2 + 5);
        assert_eq!(fields.dw_vorbis_data_offset, 8 + 2 + 5);
        assert_eq!(fields.u_max_packet_size, 6);
    }
}
