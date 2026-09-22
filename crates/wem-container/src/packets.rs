//! Pure byte codecs for Wwise size-prefixed packet streams
//! (Python: `wwise_wem/container/packets.py`).

use crate::error::ContainerError;
use crate::fmt::VorbisFmtFields;
use crate::riff::Endian;

/// A *complete* walk of a RIFF data payload: every byte is accounted for by
/// a size prefix and its payload, so `packets` and `sizes` are the whole
/// stream (Python `extract_packets` result dict, `ok=True`).
///
/// A walk that could not consume the payload is not one of these and cannot
/// be mistaken for one: `extract_packets` reports it as
/// [`ContainerError::PacketWalkFailed`], and this type has no success flag to
/// read — the presence of a value is the success.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PacketWalk {
    pub seek_table: Vec<u8>,
    pub packets: Vec<Vec<u8>>,
    pub sizes: Vec<u16>,
    /// Byte offset the walk stopped at; equal to `data_size` for a complete
    /// walk.
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
///
/// The Python function reports a payload it could not walk as
/// `{"ok": False, ...}` alongside the packets read so far. Here that outcome
/// is [`ContainerError::PacketWalkFailed`] instead: a caller that reads only
/// the `Result` cannot take a partial walk for a complete one, and the error
/// distinguishes the two ways the walk stops — a size prefix that overruns
/// the payload (`declared_size: Some(_)`) and trailing bytes too few to be a
/// size prefix at all (`declared_size: None`).
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
            return Err(ContainerError::PacketWalkFailed {
                position,
                declared_size: Some(size),
                remaining: data.len() - position,
            });
        }
        let payload = data[position + 2..position + 2 + size as usize].to_vec();
        sizes.push(size);
        packets.push(payload);
        position += 2 + size as usize;
    }
    if position != data.len() {
        // Fewer than two bytes are left, so there is no size prefix here: the
        // walk observed trailing bytes, not a packet of any size. Reporting
        // `declared_size: None` says exactly that, where a zero would invent a
        // packet the payload does not carry.
        return Err(ContainerError::PacketWalkFailed {
            position,
            declared_size: None,
            remaining: data.len() - position,
        });
    }
    let setup = sizes.first().copied();
    Ok(PacketWalk {
        seek_table: seek,
        packets,
        sizes,
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
///
/// `nAvgBytesPerSec` is *derived*, not carried: Wwise writes
/// `floor(data_payload_bytes * nSamplesPerSec / dwTotalPCMFrames)`. Verified
/// against six reference WEMs emitted by the paired 2013.2 build (6ch/44.1k
/// fixture 108677 B / 139398 frames / 44100 Hz -> 34381, plus five 2ch/48k
/// files); the 96000-frame sample whose exact quotient is 18600.5 pins the
/// operator to truncation (rounding would give 18601). Deriving it also keeps
/// the 6ch container byte-identical, since that registration already equals the
/// computed value.
///
/// The terminal overlap excess is reported as
/// [`ContainerError::TerminalExcessTooLarge`] when it does not fit the u16 the
/// fmt fields store, rather than leaving both derived fields stale.
pub fn recompute_vorbis_fmt_sizes(
    fields: &mut VorbisFmtFields,
    packets: &[&[u8]],
    seek_table: &[u8],
) -> Result<(), ContainerError> {
    let data_size = seek_table.len() + packets.iter().map(|p| 2 + p.len()).sum::<usize>();
    let first = packets.first().map(|p| 2 + p.len()).unwrap_or(0);
    let first_in_data = seek_table.len() + first;
    fields.dw_seek_table_size = seek_table.len() as u32;
    fields.dw_data_payload_size = data_size as u32;
    fields.dw_first_audio_packet_offset = first_in_data as u32;
    fields.dw_vorbis_data_offset = first_in_data as u32;
    // ``uMaxPacketSize`` is the largest *audio* packet.  Verified against the
    // paired build: an all-silent 2ch/48k stream has a 215-byte setup packet and
    // 1-byte audio packets, and the build writes 1, not 215.  Including the setup
    // packet only ever changes the field when the setup is the largest packet.
    if let Some(max) = packets.iter().skip(1).map(|p| p.len()).max() {
        fields.u_max_packet_size = max as u16;
    }
    let audio_packets = &packets[1.min(packets.len())..];
    if audio_packets.len() >= 2
        && audio_packets.iter().all(|packet| !packet.is_empty())
        && fields.u_blocksize0_pow < 32
        && fields.u_blocksize1_pow < 32
    {
        let blocksizes = [
            1u64 << fields.u_blocksize0_pow,
            1u64 << fields.u_blocksize1_pow,
        ];
        let rendered_frames = audio_packets.windows(2).fold(0u64, |total, pair| {
            let previous = (pair[0][0] & 1) as usize;
            let current = (pair[1][0] & 1) as usize;
            total + (blocksizes[previous] + blocksizes[current]) / 4
        });
        let terminal_excess = rendered_frames.saturating_sub(fields.dw_total_pcm_frames as u64);
        let excess =
            u16::try_from(terminal_excess).map_err(|_| ContainerError::TerminalExcessTooLarge {
                excess: terminal_excess,
            })?;
        // Wwise writes the final overlap excess twice: directly at 0x32
        // and in the high word of the 0x24 field.
        fields.u_unknown_0x32 = excess;
        fields.dw_unknown_0x24 = u32::from(excess) << 16;
    }
    if fields.dw_total_pcm_frames != 0 && fields.n_samples_per_sec != 0 {
        fields.n_avg_bytes_per_sec = ((data_size as u64 * fields.n_samples_per_sec as u64)
            / fields.dw_total_pcm_frames as u64) as u32;
    }
    Ok(())
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
        assert_eq!(walk.setup_packet_size, Some(p1.len() as u16));
        assert_eq!(walk.first_audio_offset, Some(seek.len() + 2 + p1.len()));
        assert_eq!(walk.packets, vec![p1.to_vec(), p2.to_vec()]);
        assert_eq!(walk.sizes, vec![p1.len() as u16, p2.len() as u16]);
        // A walk value only exists when it consumed the payload, so its end is
        // the payload end by construction.
        assert_eq!(walk.end, bytes.len());
        assert_eq!(walk.data_size, bytes.len());
    }

    #[test]
    fn packet_stream_rejects_too_large() {
        let big = vec![0u8; 65536];
        assert!(build_packet_stream(&[&big], &[], Endian::Little).is_err());
    }

    #[test]
    fn extract_truncated_payload_reports_the_declared_size() {
        // size claims 10 bytes (LE: 0x0A 0x00), only 3 present.
        let data = [10u8, 0u8, 1, 2, 3];
        let error = extract_packets(&data, 0, Endian::Little)
            .expect_err("a walk that cannot consume the payload is an error");
        assert_eq!(
            error,
            ContainerError::PacketWalkFailed {
                position: 0,
                declared_size: Some(10),
                remaining: 5,
            }
        );
    }

    #[test]
    fn extract_trailing_byte_reports_no_size_rather_than_zero() {
        // One packet of one byte, then a byte that is too few for a size
        // prefix: the walk never read a size there, so the error must not
        // carry the zero a `size` field would have been filled with.
        let framed =
            build_packet_stream(&[b"a"], &[], Endian::Little).expect("packet stream builds");
        let mut data = framed.clone();
        data.push(0xFF);
        let error = extract_packets(&data, 0, Endian::Little)
            .expect_err("trailing bytes are an incomplete walk");
        assert_eq!(
            error,
            ContainerError::PacketWalkFailed {
                position: framed.len(),
                declared_size: None,
                remaining: 1,
            }
        );
        assert!(error.to_string().contains("too few for a size prefix"));
    }

    #[test]
    fn packet_walk_failure_names_what_it_observed() {
        let truncated = extract_packets(&[10u8, 0u8, 1, 2, 3], 0, Endian::Little)
            .expect_err("truncated packet");
        assert_eq!(
            truncated.to_string(),
            "packet walk failed at 0: size 10 declared, but only 5 byte(s) remain from the prefix"
        );
    }

    #[test]
    fn walk_value_only_exists_for_a_complete_payload() {
        // The two failure shapes the Python dict expressed as `ok=False` are
        // both `Err` here, so a caller cannot read a partial walk as a
        // complete one by checking the wrong thing.
        for data in [vec![10u8, 0u8, 1, 2, 3], vec![0u8, 0u8, 0xFF]] {
            assert!(
                extract_packets(&data, 0, Endian::Little).is_err(),
                "{data:?} is not a complete packet stream"
            );
        }
    }

    #[test]
    fn recompute_sizes_matches_python() {
        let mut fields = VorbisFmtFields::DEFAULTS;
        let setup = b"setup";
        let audio = b"packet";
        let seek = b"seekseek";
        recompute_vorbis_fmt_sizes(&mut fields, &[setup.as_ref(), audio.as_ref()], seek)
            .expect("sizes recompute");
        // data = 8 + (2+5) + (2+6) = 23
        assert_eq!(fields.dw_seek_table_size, 8);
        assert_eq!(fields.dw_data_payload_size, 23);
        assert_eq!(fields.dw_first_audio_packet_offset, 8 + 2 + 5);
        assert_eq!(fields.dw_vorbis_data_offset, 8 + 2 + 5);
        assert_eq!(fields.u_max_packet_size, 6);
    }

    #[test]
    fn max_packet_size_counts_audio_packets_only() {
        // The setup packet is excluded: the paired build writes 1 for an
        // all-silent 2ch/48k stream whose setup packet is 215 bytes.
        let mut fields = VorbisFmtFields::DEFAULTS;
        let setup = b"setup-packet";
        let audio = b"a";
        recompute_vorbis_fmt_sizes(&mut fields, &[setup.as_ref(), audio.as_ref()], b"")
            .expect("sizes recompute");
        assert_eq!(fields.u_max_packet_size, 1);
    }

    #[test]
    fn terminal_overlap_excess_follows_audio_modes() {
        let mut fields = VorbisFmtFields {
            dw_total_pcm_frames: 2304,
            u_blocksize0_pow: 8,
            u_blocksize1_pow: 11,
            dw_unknown_0x24: 99,
            u_unknown_0x32: 99,
            ..VorbisFmtFields::DEFAULTS
        };
        let setup = b"setup";
        let long = b"\x01";
        recompute_vorbis_fmt_sizes(
            &mut fields,
            &[
                setup.as_ref(),
                long.as_ref(),
                long.as_ref(),
                long.as_ref(),
                long.as_ref(),
            ],
            b"",
        )
        .expect("sizes recompute");
        assert_eq!(fields.u_unknown_0x32, 768);
        assert_eq!(fields.dw_unknown_0x24, 768 << 16);
    }
}
