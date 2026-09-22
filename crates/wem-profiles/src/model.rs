//! Immutable encoder profile value model (Python: `wwise_wem.model` +
//! `wwise_wem.profiles.key`).

use crate::error::ProfileError;
use crate::key::ProfileKey;

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

/// Complete immutable identity, setup and container defaults for encoding
/// (Python `EncoderProfile`).
///
/// Two construction shapes coexist, both additive:
/// * complete profiles: the setup packet bytes themselves;
/// * draft profiles (setup pending corpus): no setup packet, with
///   `setup_available == false` and a `pending_reason`, so every encode
///   attempt on one fails with a clear pending error instead of silently
///   forging a setup.
///
/// Only the first shape has a producer: the compiled carrier always carries a
/// setup packet, and the manifest-declared draft state went with the resource
/// intake. The draft branch is kept because the value model's readers
/// (`wem-core`'s pending-profile error path) still ask for it, and a future
/// carrier that registers a configuration without a setup would need it; it is
/// reachable from no installed profile today.
///
/// The setup packet is carried as bytes, not as a resource reference: the
/// packet is a compiled profile fact, and a reference would make the value
/// model depend on where a tree happens to live. Nothing derived from those
/// bytes is stored beside them — the packet is what the codec consumes, and
/// the profile's identity is its key.
#[derive(Debug, Clone, PartialEq)]
pub struct EncoderProfile {
    key: ProfileKey,
    setup_packet: Option<Vec<u8>>,
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
    /// `setup_packet` is `Some` for complete profiles — the bytes the codec
    /// consumes — and `None` for draft profiles.
    pub fn new(
        key: ProfileKey,
        setup_packet: Option<Vec<u8>>,
        quality: Option<f64>,
        block_sizes: [i64; 2],
        container_metadata: ContainerMetadata,
        setup_available: bool,
        pending_reason: Option<String>,
    ) -> Result<Self, ProfileError> {
        if let Some(quality) = quality {
            if !quality.is_finite() {
                return Err(ProfileError::quality("quality must be a finite number"));
            }
        }
        if setup_available != setup_packet.is_some() {
            return Err(ProfileError::identity(
                "profile setup availability disagrees with its setup packet",
            ));
        }
        if (key.channels(), key.sample_rate())
            != (
                container_metadata.n_channels,
                container_metadata.n_samples_per_sec,
            )
        {
            return Err(ProfileError::identity(
                "profile key geometry differs from container metadata",
            ));
        }
        if block_sizes[0] <= 0 || block_sizes[1] <= 0 {
            return Err(ProfileError::identity(
                "profile block sizes must contain two positive sizes",
            ));
        }
        let expected = [
            1i64 << container_metadata.u_blocksize0_pow.max(0),
            1i64 << container_metadata.u_blocksize1_pow.max(0),
        ];
        if block_sizes != expected {
            return Err(ProfileError::identity(
                "profile block sizes differ from container metadata",
            ));
        }
        Ok(Self {
            key,
            setup_packet,
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
            return Err(ProfileError::quality("quality must be a finite number"));
        }
        self.quality = Some(quality);
        Ok(self)
    }

    /// The human label for this profile, derived from its identity.
    pub fn label(&self) -> String {
        self.key.label()
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
        self.setup_packet.clone().ok_or_else(|| {
            ProfileError::identity("profile setup availability disagrees with its setup packet")
        })
    }
}
