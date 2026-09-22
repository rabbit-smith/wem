//! Complete PCM-to-WEM encode orchestration.
//!
//! Mirrors Python `wwise_wem/application/encoder.py` (plus the profile
//! lookup half of root `api.py`): the [`Encoder`] owns the immutable codec
//! inputs and runs one complete encode per PCM buffer:
//!
//! ```text
//! profiles (bundle + resources)
//!   -> analysis session: selected_windows -> analyze_window per frame
//!   -> pack_analysis_frame per analysis frame
//!   -> build_vorbis_wem container assembly
//! ```

use sha2::{Digest, Sha256};

use std::path::Path;

use wem_analysis::config::AnalysisProfileResources;
use wem_analysis::session::AnalysisSession;
use wem_container::fmt::VorbisFmtFields;
use wem_container::riff::Endian;
use wem_container::wem::build_vorbis_wem;
use wem_profiles::assembly::assemble_encoder_profile_resources;
use wem_profiles::assembly::EncoderProfileResources;
use wem_profiles::carrier::{compiled_profile_for_selection, CompiledProfile};
use wem_profiles::error::ProfileError;
use wem_profiles::model::ContainerMetadata;
use wem_profiles::model::EncoderProfile;
use wem_profiles::selection::{WwiseProfile, WwiseVersion};
use wem_profiles::source::ProfileSource;
use wem_vorbis::codebook::Codebook;
use wem_vorbis::setup::SetupInfo;

use crate::error::{EncoderError, InternalError};
use crate::pack::pack_analysis_packet;

/// Minimum PCM frames for one encode (Python: 4096-frame lower bound).
pub const MIN_PCM_FRAMES: u32 = 4096;

// ---------------------------------------------------------------------------
// PCM input
// ---------------------------------------------------------------------------

/// The storage shape one [`Pcm16`] was handed its samples in.
///
/// The two byte-backed shapes keep the caller's little-endian wire bytes
/// exactly as received and decode one sample at a time inside
/// [`Pcm16::to_float_rows`]. Every shell receives bytes (the C ABI and wasm
/// entries interleaved, the PyO3 memoryview channel-major), so de-interleaving
/// them into an `i16` row structure that is immediately discarded — only to be
/// read back as floats — was pure overhead.
///
/// The decode is statement-identical in every shape:
/// `i16::from_le_bytes([lo, hi])` followed by `value as f64 / 32768.0`
/// (or `*value as f64 / 32768.0` for the row shape, which already holds the
/// decoded sample). The shape a buffer arrived in can therefore never change
/// a single output bit.
#[derive(Debug, Clone)]
enum PcmStorage {
    /// Channel-major `i16` rows with equal frame counts
    /// (Python `PcmBuffer` rows).
    Rows(Vec<Vec<i16>>),
    /// Channel-major little-endian signed-16 bytes: channel `c` occupies
    /// `[c * frames * 2, (c + 1) * frames * 2)` — the layout of a
    /// C-contiguous `(channels, frames)` signed-16 memoryview.
    ChannelMajorLe { bytes: Vec<u8> },
    /// Interleaved little-endian signed-16 bytes (frame-major,
    /// channel-minor: the interleaved wire form of the streaming API).
    InterleavedLe { bytes: Vec<u8> },
}

/// Typed PCM input in the signed-16 wire representation
/// (Python `PcmBuffer`, i16 flavor).
///
/// Samples are channel-major in every storage shape; the shape is whichever
/// form the caller already had (rows of `i16`, channel-major LE bytes, or
/// interleaved LE bytes) and is private to this type. The legacy float
/// normalization (`value / 32768.0`) is applied only at the analysis
/// boundary via [`Pcm16::to_float_rows`].
#[derive(Debug, Clone)]
pub struct Pcm16 {
    sample_rate: i64,
    channel_count: usize,
    frame_count: i64,
    storage: PcmStorage,
}

/// Two buffers are equal when they describe the same PCM — same rate, same
/// geometry, same sample values — whichever storage shape each one holds.
/// Before the byte-backed shapes existed every constructor produced the same
/// rows, so this is exactly the comparison the derive performed, only made
/// independent of how the buffer was handed in.
impl PartialEq for Pcm16 {
    fn eq(&self, other: &Self) -> bool {
        self.sample_rate == other.sample_rate
            && self.channel_count == other.channel_count
            && self.frame_count == other.frame_count
            && (0..self.frame_count as usize).all(|frame| {
                (0..self.channel_count).all(|channel| {
                    self.sample_at(channel, frame) == other.sample_at(channel, frame)
                })
            })
    }
}

impl Eq for Pcm16 {}

impl Pcm16 {
    /// Construct from channel-major i16 rows
    /// (Python `PcmBuffer.__post_init__` checks).
    pub fn new(sample_rate: i64, channels: Vec<Vec<i16>>) -> Result<Self, EncoderError> {
        if sample_rate <= 0 {
            return Err(EncoderError::StateError {
                message: "sample rate must be positive".into(),
            });
        }
        if channels.is_empty() {
            return Err(EncoderError::StateError {
                message: "PCM buffer needs at least one channel".into(),
            });
        }
        if channels[0].is_empty() {
            return Err(EncoderError::StateError {
                message: "PCM buffer needs at least one frame".into(),
            });
        }
        let frame_count = channels[0].len() as i64;
        if channels.iter().any(|row| row.len() as i64 != frame_count) {
            return Err(EncoderError::StateError {
                message: "PCM channels must have equal frame counts".into(),
            });
        }
        Ok(Self {
            sample_rate,
            channel_count: channels.len(),
            frame_count,
            storage: PcmStorage::Rows(channels),
        })
    }

    /// Construct from channel-major little-endian signed-16 PCM bytes:
    /// channel `c` occupies `[c * frames * 2, (c + 1) * frames * 2)`, with the
    /// frame count taken from the buffer length. This is the shape of a
    /// C-contiguous `(channels, frames)` signed-16 memoryview — the PyO3
    /// intake hands its `tobytes()` result straight here, so no `i16` row
    /// structure is ever built on that path.
    ///
    /// Validation matches `new`/`from_interleaved_le`: positive sample rate,
    /// at least one channel, at least one frame, and a byte length that is an
    /// exact multiple of the frame width. A trailing partial frame is a
    /// geometry violation (GEOMETRY_MISMATCH).
    ///
    /// `bytes` is `impl Into<Vec<u8>>`: a caller that already owns the buffer
    /// hands it over without a copy, and one holding only a borrow (a slice
    /// over client memory, or `Wav16`'s `&self`) passes `&[u8]` and pays the
    /// one copy that owning the samples costs.
    pub fn from_channel_major_le(
        sample_rate: i64,
        channel_count: usize,
        bytes: impl Into<Vec<u8>>,
    ) -> Result<Self, EncoderError> {
        let bytes = bytes.into();
        let frame_count = byte_geometry(channel_count, &bytes)?;
        validate_sample_rate(sample_rate)?;
        Ok(Self {
            sample_rate,
            channel_count,
            frame_count: frame_count as i64,
            storage: PcmStorage::ChannelMajorLe { bytes },
        })
    }

    /// Construct from interleaved little-endian signed-16 PCM bytes
    /// (the interleaved wire form of the streaming API).
    ///
    /// The byte length must be a multiple of `2 * channel_count`; a trailing
    /// partial frame is a geometry violation (GEOMETRY_MISMATCH).
    ///
    /// `bytes` is `impl Into<Vec<u8>>`, exactly as
    /// [`Pcm16::from_channel_major_le`].
    pub fn from_interleaved_le(
        sample_rate: i64,
        channel_count: usize,
        bytes: impl Into<Vec<u8>>,
    ) -> Result<Self, EncoderError> {
        let bytes = bytes.into();
        let frame_count = byte_geometry(channel_count, &bytes)?;
        validate_sample_rate(sample_rate)?;
        Ok(Self {
            sample_rate,
            channel_count,
            frame_count: frame_count as i64,
            storage: PcmStorage::InterleavedLe { bytes },
        })
    }

    pub fn sample_rate(&self) -> i64 {
        self.sample_rate
    }

    /// Number of channels (Python `channel_count`).
    pub fn channel_count(&self) -> usize {
        self.channel_count
    }

    /// Frame count (Python `frame_count`).
    pub fn frame_count(&self) -> i64 {
        self.frame_count
    }

    /// Channel-major float rows at the legacy normalization
    /// (`value / 32768.0`), exactly as Python `read_pcm_wav` feeds
    /// `PcmBuffer`.
    ///
    /// One arm per storage shape; the decode statement is the same in all of
    /// them, and only the offset arithmetic differs (which sample lands at
    /// which `(channel, frame)`). This is the whole bit-exactness argument of
    /// the shape split, so the three arms must stay statement-identical.
    pub fn to_float_rows(&self) -> Vec<Vec<f64>> {
        match &self.storage {
            PcmStorage::Rows(rows) => rows
                .iter()
                .map(|row| row.iter().map(|value| *value as f64 / 32768.0).collect())
                .collect(),
            PcmStorage::ChannelMajorLe { bytes } => {
                let channel_bytes = self.frame_count as usize * 2;
                bytes
                    .chunks_exact(channel_bytes)
                    .map(|row| {
                        row.chunks_exact(2)
                            .map(|pair| i16::from_le_bytes([pair[0], pair[1]]) as f64 / 32768.0)
                            .collect()
                    })
                    .collect()
            }
            PcmStorage::InterleavedLe { bytes } => (0..self.channel_count)
                .map(|channel| {
                    (0..self.frame_count as usize)
                        .map(|frame| {
                            let offset = (frame * self.channel_count + channel) * 2;
                            i16::from_le_bytes([bytes[offset], bytes[offset + 1]]) as f64 / 32768.0
                        })
                        .collect()
                })
                .collect(),
        }
    }

    /// One sample in channel-major logical order, decoded exactly as
    /// [`Pcm16::to_float_rows`] decodes it. Used by the shape-independent
    /// `PartialEq`; the geometry is checked at construction, so the indexing
    /// stays in bounds for every shape.
    fn sample_at(&self, channel: usize, frame: usize) -> i16 {
        match &self.storage {
            PcmStorage::Rows(rows) => rows[channel][frame],
            PcmStorage::ChannelMajorLe { bytes } => {
                let offset = (channel * self.frame_count as usize + frame) * 2;
                i16::from_le_bytes([bytes[offset], bytes[offset + 1]])
            }
            PcmStorage::InterleavedLe { bytes } => {
                let offset = (frame * self.channel_count + channel) * 2;
                i16::from_le_bytes([bytes[offset], bytes[offset + 1]])
            }
        }
    }
}

/// Shared geometry validation for the byte-backed shapes: at least one
/// channel, at least one frame, and a byte length that is an exact multiple of
/// the frame width. Returns the frame count.
fn byte_geometry(channel_count: usize, bytes: &[u8]) -> Result<usize, EncoderError> {
    if channel_count == 0 {
        return Err(EncoderError::StateError {
            message: "PCM buffer needs at least one channel".into(),
        });
    }
    if bytes.is_empty() {
        return Err(EncoderError::StateError {
            message: "PCM buffer needs at least one frame".into(),
        });
    }
    let bytes_per_frame = channel_count
        .checked_mul(2)
        .ok_or_else(|| EncoderError::StateError {
            message: "PCM channel count is too large".into(),
        })?;
    if !bytes.len().is_multiple_of(bytes_per_frame) {
        return Err(EncoderError::GeometryMismatch {
            message: "chunk carries a trailing partial PCM frame".into(),
        });
    }
    Ok(bytes.len() / bytes_per_frame)
}

/// Positive sample rate, the last check the byte-backed constructors apply
/// (the order `new` was called in before the storage split).
fn validate_sample_rate(sample_rate: i64) -> Result<(), EncoderError> {
    if sample_rate <= 0 {
        return Err(EncoderError::StateError {
            message: "sample rate must be positive".into(),
        });
    }
    Ok(())
}

// ---------------------------------------------------------------------------
// Container plan
// ---------------------------------------------------------------------------

/// Per-output container metadata, kept separate from codec identity
/// (Python `_ContainerPlan`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ContainerPlan {
    fmt: VorbisFmtFields,
    endian: Endian,
    seek_table: Vec<u8>,
    extra_chunks: Vec<([u8; 4], Vec<u8>)>,
    metadata_source: String,
}

impl ContainerPlan {
    /// Fresh plan from one installed profile
    /// (Python `_ContainerPlan.from_profile`).
    ///
    /// The plan carries the profile's own container metadata; the provenance
    /// label is the name-free selection description (geometry plus Wwise
    /// generation), never the internal profile name.
    pub fn from_profile(profile: &EncoderProfile) -> Self {
        let meta = profile.container_metadata();
        let fmt = from_container_metadata(meta);
        let version = WwiseVersion::from_generation(profile.key().generation())
            .map(|version| version.label().to_string())
            .unwrap_or_else(|_| profile.key().generation().to_string());
        Self {
            fmt,
            endian: if profile.endian() == "be" {
                Endian::Big
            } else {
                Endian::Little
            },
            seek_table: profile.seek_table().to_vec(),
            extra_chunks: profile
                .extra_chunks()
                .iter()
                .filter_map(|(id, payload)| {
                    if id.len() != 4 {
                        return None;
                    }
                    let mut chunk_id = [0u8; 4];
                    chunk_id.copy_from_slice(&id[..4]);
                    Some((chunk_id, payload.clone()))
                })
                .collect(),
            metadata_source: format!(
                "profile:{}ch/{}Hz/{}",
                profile.channels(),
                profile.sample_rate(),
                version
            ),
        }
    }

    pub fn fmt(&self) -> &VorbisFmtFields {
        &self.fmt
    }

    pub fn endian(&self) -> Endian {
        self.endian
    }

    pub fn seek_table(&self) -> &[u8] {
        &self.seek_table
    }

    pub fn extra_chunks(&self) -> &[([u8; 4], Vec<u8>)] {
        &self.extra_chunks
    }

    /// Provenance label reported in stats
    /// (Python `metadata_source`; must be non-empty).
    pub fn metadata_source(&self) -> &str {
        &self.metadata_source
    }
}

fn from_container_metadata(meta: &ContainerMetadata) -> VorbisFmtFields {
    VorbisFmtFields {
        w_format_tag: meta.w_format_tag as u16,
        n_channels: meta.n_channels as u16,
        n_samples_per_sec: meta.n_samples_per_sec as u32,
        n_avg_bytes_per_sec: meta.n_avg_bytes_per_sec as u32,
        n_block_align: meta.n_block_align as u16,
        w_bits_per_sample: meta.w_bits_per_sample as u16,
        cb_size: meta.cb_size as u16,
        w_reserved0: meta.w_reserved0 as u16,
        dw_channel_mask: meta.dw_channel_mask as u32,
        dw_total_pcm_frames: meta.dw_total_pcm_frames as u32,
        dw_first_audio_packet_offset: meta.dw_first_audio_packet_offset as u32,
        dw_data_payload_size: meta.dw_data_payload_size as u32,
        dw_unknown_0x24: meta.dw_unknown_0x24 as u32,
        dw_seek_table_size: meta.dw_seek_table_size as u32,
        dw_vorbis_data_offset: meta.dw_vorbis_data_offset as u32,
        u_max_packet_size: meta.u_max_packet_size as u16,
        u_unknown_0x32: meta.u_unknown_0x32 as u16,
        dw_unknown_0x34: meta.dw_unknown_0x34 as u32,
        dw_unknown_0x38: meta.dw_unknown_0x38 as u32,
        dw_unknown_0x3c: meta.dw_unknown_0x3c as u32,
        u_blocksize0_pow: meta.u_blocksize0_pow as u8,
        u_blocksize1_pow: meta.u_blocksize1_pow as u8,
    }
}

// ---------------------------------------------------------------------------
// Results
// ---------------------------------------------------------------------------

/// Immutable encode statistics (Python `EncodeStats`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EncodeStats {
    pub pcm_frames: i64,
    pub channels: i64,
    pub audio_packets: i64,
    pub short_packets: i64,
    pub long_packets: i64,
    pub bytes: i64,
    pub metadata_source: String,
}

/// Immutable encode result (Python `EncodeResult`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EncodeResult {
    pub data: Vec<u8>,
    pub stats: EncodeStats,
}

impl EncodeResult {
    /// SHA-256 of the encoded bytes, lowercase hex
    /// (Python `EncodeResult.sha256`).
    pub fn sha256(&self) -> String {
        let digest = Sha256::digest(&self.data);
        wem_profiles::resources::hex(digest.as_slice())
    }

    /// Byte length of the assembled container.
    pub fn len(&self) -> usize {
        self.data.len()
    }

    /// Whether the container carries no bytes.
    ///
    /// A successful encode always carries a container, so this is `false`
    /// on every result the encoder produces; it exists because a type with
    /// a `len` should answer the question.
    pub fn is_empty(&self) -> bool {
        self.data.is_empty()
    }

    /// Write the container bytes to `path`.
    ///
    /// The library-user counterpart of reading `data` and calling
    /// `std::fs::write`: the path is named in the error, so a failed write
    /// says which file failed.
    pub fn write_to(&self, path: impl AsRef<Path>) -> Result<(), EncoderError> {
        let path = path.as_ref();
        std::fs::write(path, &self.data).map_err(|error| {
            EncoderError::Internal(InternalError::Io {
                message: format!("{}: {error}", path.display()),
            })
        })
    }
}

// ---------------------------------------------------------------------------
// Encoder
// ---------------------------------------------------------------------------

/// Owns immutable codec inputs and runs one complete encode per PCM buffer
/// (Python `Encoder`).
pub struct Encoder {
    profile: EncoderProfile,
    container: ContainerPlan,
    resources: EncoderProfileResources,
}

impl Encoder {
    /// Construct the encoder from a structured profile selection — the only
    /// caller-facing profile selector (Python `Encoder(profile=...)`).
    ///
    /// The selection is resolved against the profile bundle compiled into
    /// this library; a selection no installed profile satisfies fails with
    /// [`EncoderError::ProfileNotFound`], never with a substituted default.
    pub fn new(selection: WwiseProfile) -> Result<Self, EncoderError> {
        Self::new_with_quality(selection, None)
    }

    /// Construct the encoder from a structured selection, optionally bound to
    /// a quality factor.
    ///
    /// `None` reproduces the historical bytes exactly; with a quality value
    /// the analysis-resource assembly interpolates the profile's quality
    /// curves (a missing quality-curves resource is a clear configuration
    /// error, never a silent fallback).
    pub fn new_with_quality(
        selection: WwiseProfile,
        quality: Option<f64>,
    ) -> Result<Self, EncoderError> {
        let compiled = compiled_profile_for_selection(selection)
            .map_err(|error| selection_error(&error, selection))?;
        let profile = compiled
            .encoder_profile()
            .map_err(|error| selection_error(&error, selection))?;
        let profile = match quality {
            Some(quality) => profile
                .with_quality(quality)
                .map_err(|error| EncoderError::Internal(InternalError::Profile(error)))?,
            None => profile,
        };
        Self::from_profile_and_carrier(&profile, None, &compiled)
    }

    /// Shared construction from one profile identity + the compiled carrier
    /// it came from; every entry funnels through here so the cross-checks
    /// cannot drift between callers.
    pub(crate) fn from_profile_and_carrier(
        profile: &EncoderProfile,
        container: Option<ContainerPlan>,
        compiled: &CompiledProfile,
    ) -> Result<Self, EncoderError> {
        if profile.block_sizes() != [256, 2048] {
            return Err(EncoderError::StateError {
                message: "selected profile block geometry is unsupported; \
                          the installed runtime supports 256/2048 blocks"
                    .into(),
            });
        }
        let plan = match container {
            Some(plan) => plan,
            None => ContainerPlan::from_profile(profile),
        };
        let (plan_channels, plan_rate) = (
            plan.fmt.n_channels as i64,
            plan.fmt.n_samples_per_sec as i64,
        );
        if (plan_channels, plan_rate) != (profile.channels(), profile.sample_rate()) {
            return Err(EncoderError::StateError {
                message: "container metadata geometry differs from encoder profile".into(),
            });
        }

        if compiled.key() != profile.key() {
            return Err(EncoderError::StateError {
                message: "selected profile differs from installed profile bundle".into(),
            });
        }
        // Draft profiles (setup pending corpus export) are listed by the
        // registry but must never encode: refuse with a clear pending
        // error instead of silently forging a setup packet.
        if !profile.setup_available() {
            return Err(EncoderError::StateError {
                message: draft_pending_message(profile),
            });
        }
        let setup_sha = compiled.tables().setup_sha256;
        if profile.setup_sha256() != setup_sha {
            return Err(EncoderError::StateError {
                message: format!(
                    "selected profile {} differs from installed profile setup",
                    profile.name()
                ),
            });
        }
        // A compiled profile carries its analysis resources by construction
        // (they are Rust constants in the same artifact), so the pending
        // analysis-resources state the resource-backed loader could reach
        // does not exist here.
        let setup_packet = profile.setup_packet()?;
        let resources =
            assemble_encoder_profile_resources(compiled, Some(&setup_packet), profile.quality())?;

        Ok(Self {
            profile: profile.clone(),
            container: plan,
            resources,
        })
    }

    /// The selected encoder profile (Python `profile`).
    pub fn profile(&self) -> &EncoderProfile {
        &self.profile
    }

    /// The analysis resources for this profile
    /// (Python `_analysis_resources`; used by stage timers and streaming
    /// shells).
    pub fn analysis_resources(&self) -> &AnalysisProfileResources {
        &self.resources.analysis
    }

    /// The parsed Vorbis setup (Python `_setup`).
    pub fn setup(&self) -> &SetupInfo {
        &self.resources.setup
    }

    /// The resolved codebooks (Python `_books`).
    pub fn codebooks(&self) -> &[Codebook] {
        &self.resources.codebooks
    }

    /// The verified setup packet bytes (Python `_setup_packet`).
    pub fn setup_packet(&self) -> &[u8] {
        &self.resources.setup_packet
    }

    /// The container plan for this encoder (Python `_container`).
    pub fn container_plan(&self) -> &ContainerPlan {
        &self.container
    }

    /// Encode one independent PCM buffer into a complete Wwise WEM
    /// (Python `Encoder.encode_pcm`).
    pub fn encode_pcm(&self, pcm: &Pcm16) -> Result<EncodeResult, EncoderError> {
        if (pcm.channel_count() as i64, pcm.sample_rate())
            != (self.profile.channels(), self.profile.sample_rate())
        {
            return Err(EncoderError::GeometryMismatch {
                message: "PCM channel count/sample rate differs from encoder profile".into(),
            });
        }
        if pcm.frame_count() < MIN_PCM_FRAMES as i64 {
            return Err(EncoderError::InputTooShort {
                want: MIN_PCM_FRAMES,
                got: pcm.frame_count() as u32,
            });
        }

        let mut session = AnalysisSession::new(
            self.profile.channels(),
            self.profile.sample_rate(),
            self.profile.block_sizes(),
            self.resources.analysis.clone(),
        )?;

        let pcm_rows = session.condition_pcm(&pcm.to_float_rows())?;
        let (modes, windows) = session.selected_windows(&pcm_rows)?;

        let channels = self.profile.channels() as u32;
        let mut audio_packets: Vec<Vec<u8>> = Vec::with_capacity(windows.len());
        for window in windows {
            let analysis = session.analyze_window(window, None)?;
            let packet = pack_analysis_packet(
                &self.resources.setup,
                &self.resources.codebooks,
                &analysis,
                channels,
            )?;
            audio_packets.push(packet);
        }
        if audio_packets.len() != modes.len() {
            return Err(EncoderError::Internal(InternalError::Invariant {
                message: "analysis window and mode counts diverged",
            }));
        }

        let mut fields = self.container.fmt;
        fields.dw_total_pcm_frames = pcm.frame_count() as u32;

        let mut packets = Vec::with_capacity(1 + audio_packets.len());
        packets.push(self.resources.setup_packet.clone());
        packets.extend(audio_packets);

        let built = build_vorbis_wem(
            fields,
            &packets,
            &self.container.seek_table,
            self.container.endian,
            &self.container.extra_chunks,
            true,
            None,
        )?;

        let short_packets = modes.iter().filter(|&&m| m == 0).count() as i64;
        let long_packets = modes.iter().filter(|&&m| m == 1).count() as i64;
        let bytes = built.wem_bytes.len() as i64;
        Ok(EncodeResult {
            data: built.wem_bytes,
            stats: EncodeStats {
                pcm_frames: pcm.frame_count(),
                channels: pcm.channel_count() as i64,
                audio_packets: short_packets + long_packets,
                short_packets,
                long_packets,
                bytes,
                metadata_source: self.container.metadata_source.clone(),
            },
        })
    }
}

/// The clear pending-corpus error for a draft profile encode attempt.
pub fn draft_pending_message(profile: &EncoderProfile) -> String {
    match profile.pending_reason() {
        Some(reason) => format!(
            "profile '{}': setup packet pending ({reason}); requires paired Wwise export; encoding unavailable",
            profile.name()
        ),
        None => format!(
            "profile '{}': setup packet missing; requires paired Wwise export; encoding unavailable",
            profile.name()
        ),
    }
}

/// Map a profile-resolution failure onto the caller-facing error class.
///
/// A selection that names no installed profile, or more than one, is a
/// caller-facing resolution failure (`WEM_ERR_PROFILE_NOT_FOUND`), not an
/// internal fault; an unrecognized generation code violates this revision's
/// selection rules (`WEM_ERR_FORMAT_UNSUPPORTED`); anything else stays internal.
fn selection_error(error: &ProfileError, selection: WwiseProfile) -> EncoderError {
    match error {
        ProfileError::NoProfileForSelection { .. }
        | ProfileError::AmbiguousProfileSelection { .. } => EncoderError::ProfileNotFound {
            requested: selection.describe(),
        },
        ProfileError::UnknownWwiseVersion { .. }
        | ProfileError::UnsupportedWwiseGeneration { .. } => EncoderError::FormatUnsupported {
            message: error.to_string(),
        },
        ProfileError::SelectionGeometryNonPositive => EncoderError::StateError {
            message: error.to_string(),
        },
        other => EncoderError::Internal(InternalError::Profile(other.clone())),
    }
}
