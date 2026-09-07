//! Immutable encoder profile value model (Python: `profiles/model.py` +
//! root `model.py::ContainerMetadata`).

use serde_json::{Map, Value};

use crate::bundle::{load_profile_bundle, RuntimeResourceManifest};
use crate::error::ProfileError;
use crate::key::ProfileKey;
use crate::resources::ResourceRef;

/// Typed representation of the fixed 66-byte Wwise Vorbis fmt fields
/// (Python `ContainerMetadata`).
#[derive(Debug, Clone, PartialEq, Eq)]
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

    /// Fresh legacy fmt dictionary for compatibility adapters
    /// (Python `to_fmt_dict`).
    pub fn to_fmt_map(&self, frame_count: i64) -> Map<String, Value> {
        let mut out = Map::new();
        let put = |map: &mut Map<String, Value>, key: &str, value: i64| {
            map.insert(key.into(), Value::from(value));
        };
        put(&mut out, "wFormatTag", self.w_format_tag);
        put(&mut out, "nChannels", self.n_channels);
        put(&mut out, "nSamplesPerSec", self.n_samples_per_sec);
        put(&mut out, "nAvgBytesPerSec", self.n_avg_bytes_per_sec);
        put(&mut out, "nBlockAlign", self.n_block_align);
        put(&mut out, "wBitsPerSample", self.w_bits_per_sample);
        put(&mut out, "cbSize", self.cb_size);
        put(&mut out, "wReserved0", self.w_reserved0);
        put(&mut out, "dwChannelMask", self.dw_channel_mask);
        put(&mut out, "dwTotalPCMFrames", frame_count);
        put(
            &mut out,
            "dwFirstAudioPacketOffset",
            self.dw_first_audio_packet_offset,
        );
        put(&mut out, "dwDataPayloadSize", self.dw_data_payload_size);
        put(&mut out, "dwUnknown_0x24", self.dw_unknown_0x24);
        put(&mut out, "dwSeekTableSize", self.dw_seek_table_size);
        put(&mut out, "dwVorbisDataOffset", self.dw_vorbis_data_offset);
        put(&mut out, "uMaxPacketSize", self.u_max_packet_size);
        put(&mut out, "uUnknown_0x32", self.u_unknown_0x32);
        put(&mut out, "dwUnknown_0x34", self.dw_unknown_0x34);
        put(&mut out, "dwUnknown_0x38", self.dw_unknown_0x38);
        put(&mut out, "dwUnknown_0x3C", self.dw_unknown_0x3c);
        put(&mut out, "uBlocksize0Pow", self.u_blocksize0_pow);
        put(&mut out, "uBlocksize1Pow", self.u_blocksize1_pow);
        out
    }
}

/// Read-only view of the installed manifest for one profile identity
/// (Python `EncoderProfile.runtime_manifest()` return value).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProfileManifestView {
    pub schema: String,
    /// logical name -> (manifest-dir-relative path, sha256)
    pub resources: std::collections::BTreeMap<String, (String, String)>,
    /// manifest-dir-relative path -> sha256
    pub files: std::collections::BTreeMap<String, String>,
}

/// Complete immutable identity, setup and container defaults for encoding
/// (Python `EncoderProfile`).
#[derive(Debug, Clone, PartialEq)]
pub struct EncoderProfile {
    name: String,
    key: ProfileKey,
    setup_path: ResourceRef,
    setup_sha256: String,
    block_sizes: [i64; 2],
    container_metadata: ContainerMetadata,
    endian: String,
    seek_table: Vec<u8>,
    extra_chunks: Vec<(Vec<u8>, Vec<u8>)>,
}

impl EncoderProfile {
    /// Validate and construct (Python `__post_init__` checks).
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        name: String,
        key: ProfileKey,
        setup_path: ResourceRef,
        setup_sha256: String,
        block_sizes: [i64; 2],
        container_metadata: ContainerMetadata,
    ) -> Result<Self, ProfileError> {
        if name.is_empty() {
            return Err(ProfileError::ProfileNameEmpty);
        }
        if (key.channels(), key.sample_rate())
            != (
                container_metadata.n_channels,
                container_metadata.n_samples_per_sec,
            )
        {
            return Err(ProfileError::ProfileGeometryMismatch);
        }
        if key.quality_setup_identity() != Some(format!("sha256:{setup_sha256}").as_str()) {
            return Err(ProfileError::ProfileSetupIdentityMismatch);
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
            setup_path,
            setup_sha256,
            block_sizes,
            container_metadata,
            endian: "le".to_string(),
            seek_table: Vec::new(),
            extra_chunks: Vec::new(),
        })
    }

    pub fn name(&self) -> &str {
        &self.name
    }

    pub fn key(&self) -> &ProfileKey {
        &self.key
    }

    pub fn setup_path(&self) -> &ResourceRef {
        &self.setup_path
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

    /// Fresh legacy fmt dictionary (Python `fmt` property).
    pub fn fmt(&self) -> Map<String, Value> {
        self.container_metadata
            .to_fmt_map(self.container_metadata.dw_total_pcm_frames)
    }

    /// Verified setup packet bytes (Python `setup_packet()`).
    pub fn setup_packet(&self) -> Result<Vec<u8>, ProfileError> {
        let payload = self.setup_path.read_bytes()?;
        let digest = crate::resources::hex(sha256_hex(&payload));
        if digest != self.setup_sha256 {
            return Err(ProfileError::ShaMismatch {
                path: self.setup_path.path().display().to_string(),
                expected: self.setup_sha256.clone(),
                actual: digest,
            });
        }
        Ok(payload)
    }

    /// The installed profile manifest for this profile identity
    /// (Python `runtime_manifest()`).
    pub fn runtime_manifest(&self) -> Result<ProfileManifestView, ProfileError> {
        let bundle = load_profile_bundle(&self.setup_path.data().clone(), Some(&self.name), false)?;
        if bundle.key() != &self.key {
            return Err(ProfileError::InstalledBundleMismatch {
                profile: self.name.clone(),
            });
        }
        let setup_sha = bundle
            .setup()
            .map_err(|_| ProfileError::InstalledBundleMismatch {
                profile: self.name.clone(),
            })?
            .sha256()
            .to_string();
        if self.setup_sha256 != setup_sha {
            return Err(ProfileError::InstalledBundleMismatch {
                profile: self.name.clone(),
            });
        }
        Ok(manifest_view(bundle.runtime_manifest()))
    }
}

/// Shared helper for manifest views (Python `runtime_manifest()` body).
pub(crate) fn manifest_view(manifest: &RuntimeResourceManifest) -> ProfileManifestView {
    let parent = manifest
        .ref_path()
        .parent()
        .filter(|p| !p.as_os_str().is_empty())
        .map(std::path::Path::to_path_buf)
        .unwrap_or_default();
    let mut resources = std::collections::BTreeMap::new();
    let mut files = std::collections::BTreeMap::new();
    for (name, ref_) in manifest.resources() {
        let rel = relative_to_profile_dir(ref_.path(), &parent);
        resources.insert(name.clone(), (rel.clone(), ref_.sha256().to_string()));
        files.insert(rel, ref_.sha256().to_string());
    }
    ProfileManifestView {
        schema: manifest.schema().to_string(),
        resources,
        files,
    }
}

/// Convert a profiles-dir-relative path to a manifest-dir-relative path.
pub(crate) fn relative_to_profile_dir(path: &std::path::Path, parent: &std::path::Path) -> String {
    if parent.as_os_str().is_empty() {
        return path.display().to_string();
    }
    path.strip_prefix(parent)
        .map(|p| p.display().to_string())
        .unwrap_or_else(|_| path.display().to_string())
}

fn sha256_hex(payload: &[u8]) -> [u8; 32] {
    use sha2::{Digest, Sha256};
    Sha256::digest(payload)
        .as_slice()
        .try_into()
        .expect("sha256 digest is 32 bytes")
}
