//! Immutable encoder profile value model (Python: `profiles/model.py` +
//! root `model.py::ContainerMetadata`).

use serde_json::{Map, Value};

use crate::error::ProfileError;
use crate::key::ProfileKey;
use crate::resources::hex;

/// Typed representation of the fixed 66-byte Wwise Vorbis fmt fields
/// (Python `ContainerMetadata`).
///
/// `Copy` so one recorded geometry can sit inside the compiled profile
/// carrier (`crate::tables`) as a plain constant.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ContainerMetadata {
    pub w_format_tag: i64,
    pub n_channels: i64,
    pub n_samples_per_sec: i64,
    pub n_avg_bytes_per_sec: i64,
    pub n_block_align: i64,
    pub w_bits_per_sample: i64,
    pub cb_size: i64,
    pub w_reserved0: i64,
    pub dw_channel_mask: i64,
    pub dw_total_pcm_frames: i64,
    pub dw_first_audio_packet_offset: i64,
    pub dw_data_payload_size: i64,
    pub dw_unknown_0x24: i64,
    pub dw_seek_table_size: i64,
    pub dw_vorbis_data_offset: i64,
    pub u_max_packet_size: i64,
    pub u_unknown_0x32: i64,
    pub dw_unknown_0x34: i64,
    pub dw_unknown_0x38: i64,
    pub dw_unknown_0x3c: i64,
    pub u_blocksize0_pow: i64,
    pub u_blocksize1_pow: i64,
}

impl ContainerMetadata {
    const FIELDS: &'static [&'static str] = &[
        "wFormatTag",
        "nChannels",
        "nSamplesPerSec",
        "nAvgBytesPerSec",
        "nBlockAlign",
        "wBitsPerSample",
        "cbSize",
        "wReserved0",
        "dwChannelMask",
        "dwTotalPCMFrames",
        "dwFirstAudioPacketOffset",
        "dwDataPayloadSize",
        "dwUnknown_0x24",
        "dwSeekTableSize",
        "dwVorbisDataOffset",
        "uMaxPacketSize",
        "uUnknown_0x32",
        "dwUnknown_0x34",
        "dwUnknown_0x38",
        "dwUnknown_0x3C",
        "uBlocksize0Pow",
        "uBlocksize1Pow",
    ];

    /// Build from a manifest `container_metadata` object
    /// (Python `from_fmt_dict`). All 22 fields must be integers;
    /// nChannels/nSamplesPerSec must be positive, all values non-negative.
    pub fn from_fmt_map(values: &Map<String, Value>) -> Result<Self, ProfileError> {
        let get = |field: &'static str| -> Result<i64, ProfileError> {
            match values.get(field) {
                Some(Value::Number(n)) if n.is_i64() || n.is_u64() => Ok(n
                    .as_i64()
                    .unwrap_or_else(|| n.as_u64().map_or(i64::MAX, |v| v as i64))),
                Some(_) => Err(ProfileError::ContainerFieldNotInteger { field }),
                None => Err(ProfileError::ContainerFieldMissing { field }),
            }
        };
        let w_format_tag = get(Self::FIELDS[0])?;
        let n_channels = get(Self::FIELDS[1])?;
        let n_samples_per_sec = get(Self::FIELDS[2])?;
        let n_avg_bytes_per_sec = get(Self::FIELDS[3])?;
        let n_block_align = get(Self::FIELDS[4])?;
        let w_bits_per_sample = get(Self::FIELDS[5])?;
        let cb_size = get(Self::FIELDS[6])?;
        let w_reserved0 = get(Self::FIELDS[7])?;
        let dw_channel_mask = get(Self::FIELDS[8])?;
        let dw_total_pcm_frames = get(Self::FIELDS[9])?;
        let dw_first_audio_packet_offset = get(Self::FIELDS[10])?;
        let dw_data_payload_size = get(Self::FIELDS[11])?;
        let dw_unknown_0x24 = get(Self::FIELDS[12])?;
        let dw_seek_table_size = get(Self::FIELDS[13])?;
        let dw_vorbis_data_offset = get(Self::FIELDS[14])?;
        let u_max_packet_size = get(Self::FIELDS[15])?;
        let u_unknown_0x32 = get(Self::FIELDS[16])?;
        let dw_unknown_0x34 = get(Self::FIELDS[17])?;
        let dw_unknown_0x38 = get(Self::FIELDS[18])?;
        let dw_unknown_0x3c = get(Self::FIELDS[19])?;
        let u_blocksize0_pow = get(Self::FIELDS[20])?;
        let u_blocksize1_pow = get(Self::FIELDS[21])?;

        if n_channels <= 0 || n_samples_per_sec <= 0 {
            return Err(ProfileError::ContainerGeometryNonPositive);
        }
        if let Some((field, _)) = Self::FIELDS
            .iter()
            .zip([
                w_format_tag,
                n_channels,
                n_samples_per_sec,
                n_avg_bytes_per_sec,
                n_block_align,
                w_bits_per_sample,
                cb_size,
                w_reserved0,
                dw_channel_mask,
                dw_total_pcm_frames,
                dw_first_audio_packet_offset,
                dw_data_payload_size,
                dw_unknown_0x24,
                dw_seek_table_size,
                dw_vorbis_data_offset,
                u_max_packet_size,
                u_unknown_0x32,
                dw_unknown_0x34,
                dw_unknown_0x38,
                dw_unknown_0x3c,
                u_blocksize0_pow,
                u_blocksize1_pow,
            ])
            .find(|(_, value)| *value < 0)
        {
            return Err(ProfileError::ContainerFieldNegative { field });
        }

        Ok(Self {
            w_format_tag,
            n_channels,
            n_samples_per_sec,
            n_avg_bytes_per_sec,
            n_block_align,
            w_bits_per_sample,
            cb_size,
            w_reserved0,
            dw_channel_mask,
            dw_total_pcm_frames,
            dw_first_audio_packet_offset,
            dw_data_payload_size,
            dw_unknown_0x24,
            dw_seek_table_size,
            dw_vorbis_data_offset,
            u_max_packet_size,
            u_unknown_0x32,
            dw_unknown_0x34,
            dw_unknown_0x38,
            dw_unknown_0x3c,
            u_blocksize0_pow,
            u_blocksize1_pow,
        })
    }
}

/// Complete immutable identity, setup and container defaults for encoding
/// (Python `EncoderProfile`).
///
/// Two construction shapes coexist, both additive:
/// * complete profiles: the setup packet bytes plus their SHA-256 identity;
/// * draft profiles (setup pending corpus): no setup packet; the profile
///   is listed by the registry with `setup_available == false` and a
///   `pending_reason`, and every encode attempt on it fails with a clear
///   pending error instead of silently forging a setup.
///
/// The setup packet is carried as bytes, not as a resource reference: the
/// packet is a compiled profile fact, and a reference would make the value
/// model depend on where a tree happens to live.
#[derive(Debug, Clone, PartialEq)]
pub struct EncoderProfile {
    name: String,
    key: ProfileKey,
    setup_packet: Option<Vec<u8>>,
    setup_sha256: String,
    block_sizes: [i64; 2],
    container_metadata: ContainerMetadata,
    endian: String,
    seek_table: Vec<u8>,
    extra_chunks: Vec<(Vec<u8>, Vec<u8>)>,
    /// Optional quality factor bound to this profile copy
    /// (Python `EncoderProfile.quality`; `None` = historical behavior).
    quality: Option<f64>,
    /// Whether the profile carries its setup resource.
    setup_available: bool,
    /// Manifest-declared pending reason for a draft profile.
    pending_reason: Option<String>,
}

impl EncoderProfile {
    /// Validate and construct (Python `__post_init__` checks).
    ///
    /// `setup_packet` is `Some` for complete profiles (its SHA-256 must match
    /// `setup_sha256` and the key quality/setup identity) and `None` for
    /// draft profiles, whose `setup_sha256` must be empty.
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        name: String,
        key: ProfileKey,
        setup_packet: Option<Vec<u8>>,
        setup_sha256: String,
        quality: Option<f64>,
        block_sizes: [i64; 2],
        container_metadata: ContainerMetadata,
        setup_available: bool,
        pending_reason: Option<String>,
    ) -> Result<Self, ProfileError> {
        if name.is_empty() {
            return Err(ProfileError::ProfileNameEmpty);
        }
        if let Some(quality) = quality {
            if !quality.is_finite() {
                return Err(ProfileError::QualityValueNonFinite);
            }
        }
        if setup_available != setup_packet.is_some() {
            return Err(ProfileError::BundleMissingVorbisSetup);
        }
        match setup_packet.as_ref() {
            Some(packet) => {
                if hex(sha256_hex(packet)) != setup_sha256 {
                    return Err(ProfileError::ProfileSetupIdentityMismatch);
                }
                if key.quality_setup_identity() != format!("sha256:{setup_sha256}") {
                    return Err(ProfileError::ProfileSetupIdentityMismatch);
                }
            }
            None => {
                if !setup_sha256.is_empty() {
                    return Err(ProfileError::ProfileSetupIdentityMismatch);
                }
            }
        }
        if (key.channels(), key.sample_rate())
            != (
                container_metadata.n_channels,
                container_metadata.n_samples_per_sec,
            )
        {
            return Err(ProfileError::ProfileGeometryMismatch);
        }
        if block_sizes[0] <= 0 || block_sizes[1] <= 0 {
            return Err(ProfileError::ProfileBlockSizesMalformed);
        }
        let expected = [
            1i64 << container_metadata.u_blocksize0_pow.max(0),
            1i64 << container_metadata.u_blocksize1_pow.max(0),
        ];
        if block_sizes != expected {
            return Err(ProfileError::ProfileBlockSizesMismatch);
        }
        Ok(Self {
            name,
            key,
            setup_packet,
            setup_sha256,
            block_sizes,
            container_metadata,
            endian: "le".to_string(),
            seek_table: Vec::new(),
            extra_chunks: Vec::new(),
            quality,
            setup_available,
            pending_reason,
        })
    }

    /// A copy of this profile bound to one quality factor
    /// (Python `dataclasses.replace(profile, quality=...)`; the
    /// registered profile itself is never mutated).
    pub fn with_quality(mut self, quality: f64) -> Result<Self, ProfileError> {
        if !quality.is_finite() {
            return Err(ProfileError::QualityValueNonFinite);
        }
        self.quality = Some(quality);
        Ok(self)
    }

    pub fn name(&self) -> &str {
        &self.name
    }

    pub fn key(&self) -> &ProfileKey {
        &self.key
    }

    /// The quality factor bound to this profile copy, if any.
    pub fn quality(&self) -> Option<f64> {
        self.quality
    }

    /// Whether this profile carries its setup resource.
    pub fn setup_available(&self) -> bool {
        self.setup_available
    }

    /// The manifest-declared pending reason for a draft profile.
    pub fn pending_reason(&self) -> Option<&str> {
        self.pending_reason.as_deref()
    }

    /// The compiled setup packet, when this profile carries one.
    pub fn setup_bytes(&self) -> Option<&[u8]> {
        self.setup_packet.as_deref()
    }

    pub fn setup_sha256(&self) -> &str {
        &self.setup_sha256
    }

    pub fn block_sizes(&self) -> [i64; 2] {
        self.block_sizes
    }

    pub fn container_metadata(&self) -> &ContainerMetadata {
        &self.container_metadata
    }

    pub fn channels(&self) -> i64 {
        self.key.channels()
    }

    pub fn sample_rate(&self) -> i64 {
        self.key.sample_rate()
    }

    /// Container endianness marker (Python `endian` field).
    pub fn endian(&self) -> &str {
        &self.endian
    }

    /// Container seek-table payload (Python `seek_table` field).
    pub fn seek_table(&self) -> &[u8] {
        &self.seek_table
    }

    /// Extra RIFF chunks carried by the profile (Python `extra_chunks`).
    pub fn extra_chunks(&self) -> &[(Vec<u8>, Vec<u8>)] {
        &self.extra_chunks
    }

    /// The compiled setup packet bytes (Python `setup_packet()`).
    pub fn setup_packet(&self) -> Result<Vec<u8>, ProfileError> {
        self.setup_packet
            .clone()
            .ok_or(ProfileError::BundleMissingVorbisSetup)
    }
}

fn sha256_hex(payload: &[u8]) -> [u8; 32] {
    use sha2::{Digest, Sha256};
    Sha256::digest(payload)
        .as_slice()
        .try_into()
        .expect("sha256 digest is 32 bytes")
}
