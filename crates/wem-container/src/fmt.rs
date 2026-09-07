//! Pure byte codecs for the Wwise Vorbis `fmt ` payload (66 bytes)
//! (Python: `wwise_wem/container/fmt.py`).

use crate::error::ContainerError;

pub const WWISE_VORBIS_FORMAT_TAG: u16 = 0xFFFF;
pub const WWISE_PCM_EXT_FORMAT_TAG: u16 = 0xFFFE;
pub const WWISE_PCM_FORMAT_TAG: u16 = 0x0001;
pub const WWISE_VORBIS_FMT_SIZE: usize = 66;

/// Typed 66-byte Wwise Vorbis fmt fields.
///
/// Field names follow the Wwise layout; offsets are in the module docs.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct VorbisFmtFields {
    /// wFormatTag @ 0x00 (u16)
    pub w_format_tag: u16,
    /// nChannels @ 0x02 (u16)
    pub n_channels: u16,
    /// nSamplesPerSec @ 0x04 (u32)
    pub n_samples_per_sec: u32,
    /// nAvgBytesPerSec @ 0x08 (u32)
    pub n_avg_bytes_per_sec: u32,
    /// nBlockAlign @ 0x0C (u16)
    pub n_block_align: u16,
    /// wBitsPerSample @ 0x0E (u16)
    pub w_bits_per_sample: u16,
    /// cbSize @ 0x10 (u16)
    pub cb_size: u16,
    /// wReserved0 @ 0x12 (u16)
    pub w_reserved0: u16,
    /// dwChannelMask @ 0x14 (u32)
    pub dw_channel_mask: u32,
    /// dwTotalPCMFrames @ 0x18 (u32)
    pub dw_total_pcm_frames: u32,
    /// dwFirstAudioPacketOffset @ 0x1C (u32)
    pub dw_first_audio_packet_offset: u32,
    /// dwDataPayloadSize @ 0x20 (u32)
    pub dw_data_payload_size: u32,
    /// dwUnknown_0x24 @ 0x24 (u32)
    pub dw_unknown_0x24: u32,
    /// dwSeekTableSize @ 0x28 (u32)
    pub dw_seek_table_size: u32,
    /// dwVorbisDataOffset @ 0x2C (u32)
    pub dw_vorbis_data_offset: u32,
    /// uMaxPacketSize @ 0x30 (u16)
    pub u_max_packet_size: u16,
    /// uUnknown_0x32 @ 0x32 (u16)
    pub u_unknown_0x32: u16,
    /// dwUnknown_0x34 @ 0x34 (u32)
    pub dw_unknown_0x34: u32,
    /// dwUnknown_0x38 @ 0x38 (u32)
    pub dw_unknown_0x38: u32,
    /// dwUnknown_0x3C @ 0x3C (u32)
    pub dw_unknown_0x3c: u32,
    /// uBlocksize0Pow @ 0x40 (u8)
    pub u_blocksize0_pow: u8,
    /// uBlocksize1Pow @ 0x41 (u8)
    pub u_blocksize1_pow: u8,
}

impl VorbisFmtFields {
    /// The installed profile defaults (Python `pack_vorbis_fmt` defaults).
    ///
    /// `n_channels`, `n_samples_per_sec`, `n_avg_bytes_per_sec` and
    /// `dw_total_pcm_frames` must be filled from the profile.
    pub const DEFAULTS: VorbisFmtFields = VorbisFmtFields {
        w_format_tag: WWISE_VORBIS_FORMAT_TAG,
        n_channels: 0,
        n_samples_per_sec: 0,
        n_avg_bytes_per_sec: 0,
        n_block_align: 0,
        w_bits_per_sample: 0,
        cb_size: 48,
        w_reserved0: 0,
        dw_channel_mask: 0,
        dw_total_pcm_frames: 0,
        dw_first_audio_packet_offset: 0,
        dw_data_payload_size: 0,
        dw_unknown_0x24: 0,
        dw_seek_table_size: 0,
        dw_vorbis_data_offset: 0,
        u_max_packet_size: 0,
        u_unknown_0x32: 0,
        dw_unknown_0x34: 0,
        dw_unknown_0x38: 0,
        dw_unknown_0x3c: 0,
        u_blocksize0_pow: 8,
        u_blocksize1_pow: 11,
    };

    /// Build a 66-byte Wwise Vorbis fmt payload (Python `pack_vorbis_fmt`).
    pub fn pack(self) -> Vec<u8> {
        let mut buf = vec![0u8; WWISE_VORBIS_FMT_SIZE];
        buf[0x00..0x02].copy_from_slice(&self.w_format_tag.to_le_bytes());
        buf[0x02..0x04].copy_from_slice(&self.n_channels.to_le_bytes());
        buf[0x04..0x08].copy_from_slice(&self.n_samples_per_sec.to_le_bytes());
        buf[0x08..0x0C].copy_from_slice(&self.n_avg_bytes_per_sec.to_le_bytes());
        buf[0x0C..0x0E].copy_from_slice(&self.n_block_align.to_le_bytes());
        buf[0x0E..0x10].copy_from_slice(&self.w_bits_per_sample.to_le_bytes());
        buf[0x10..0x12].copy_from_slice(&self.cb_size.to_le_bytes());
        buf[0x12..0x14].copy_from_slice(&self.w_reserved0.to_le_bytes());
        buf[0x14..0x18].copy_from_slice(&self.dw_channel_mask.to_le_bytes());
        buf[0x18..0x1C].copy_from_slice(&self.dw_total_pcm_frames.to_le_bytes());
        buf[0x1C..0x20].copy_from_slice(&self.dw_first_audio_packet_offset.to_le_bytes());
        buf[0x20..0x24].copy_from_slice(&self.dw_data_payload_size.to_le_bytes());
        buf[0x24..0x28].copy_from_slice(&self.dw_unknown_0x24.to_le_bytes());
        buf[0x28..0x2C].copy_from_slice(&self.dw_seek_table_size.to_le_bytes());
        buf[0x2C..0x30].copy_from_slice(&self.dw_vorbis_data_offset.to_le_bytes());
        buf[0x30..0x32].copy_from_slice(&self.u_max_packet_size.to_le_bytes());
        buf[0x32..0x34].copy_from_slice(&self.u_unknown_0x32.to_le_bytes());
        buf[0x34..0x38].copy_from_slice(&self.dw_unknown_0x34.to_le_bytes());
        buf[0x38..0x3C].copy_from_slice(&self.dw_unknown_0x38.to_le_bytes());
        buf[0x3C..0x40].copy_from_slice(&self.dw_unknown_0x3c.to_le_bytes());
        buf[0x40] = self.u_blocksize0_pow;
        buf[0x41] = self.u_blocksize1_pow;
        buf
    }

    /// Parse a 66-byte Wwise Vorbis fmt payload (Python `parse_vorbis_fmt`).
    pub fn parse(payload: &[u8]) -> Result<Self, ContainerError> {
        if payload.len() < WWISE_VORBIS_FMT_SIZE {
            return Err(ContainerError::FmtTooShort {
                got: payload.len(),
            });
        }
        let u16 = |off: usize| {
            u16::from_le_bytes(payload[off..off + 2].try_into().unwrap())
        };
        let u32 = |off: usize| {
            u32::from_le_bytes(payload[off..off + 4].try_into().unwrap())
        };
        Ok(Self {
            w_format_tag: u16(0x00),
            n_channels: u16(0x02),
            n_samples_per_sec: u32(0x04),
            n_avg_bytes_per_sec: u32(0x08),
            n_block_align: u16(0x0C),
            w_bits_per_sample: u16(0x0E),
            cb_size: u16(0x10),
            w_reserved0: u16(0x12),
            dw_channel_mask: u32(0x14),
            dw_total_pcm_frames: u32(0x18),
            dw_first_audio_packet_offset: u32(0x1C),
            dw_data_payload_size: u32(0x20),
            dw_unknown_0x24: u32(0x24),
            dw_seek_table_size: u32(0x28),
            dw_vorbis_data_offset: u32(0x2C),
            u_max_packet_size: u16(0x30),
            u_unknown_0x32: u16(0x32),
            dw_unknown_0x34: u32(0x34),
            dw_unknown_0x38: u32(0x38),
            dw_unknown_0x3c: u32(0x3C),
            u_blocksize0_pow: payload[0x40],
            u_blocksize1_pow: payload[0x41],
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fmt_pack_parse_roundtrip() {
        let mut f = VorbisFmtFields::DEFAULTS;
        f.n_channels = 6;
        f.n_samples_per_sec = 44100;
        f.dw_channel_mask = 0x3F;
        f.dw_total_pcm_frames = 139398;
        let bytes = f.pack();
        assert_eq!(bytes.len(), WWISE_VORBIS_FMT_SIZE);
        assert_eq!(VorbisFmtFields::parse(&bytes).unwrap(), f);
    }

    #[test]
    fn fmt_parse_rejects_short() {
        assert!(VorbisFmtFields::parse(&[0u8; 65]).is_err());
    }
}
