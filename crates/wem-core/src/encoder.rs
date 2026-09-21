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
use wem_analysis::config::AnalysisProfileResources;
use wem_analysis::session::AnalysisSession;
use wem_container::fmt::VorbisFmtFields;
use wem_container::riff::Endian;
use wem_container::wem::build_vorbis_wem;
use wem_profiles::assembly::assemble_encoder_profile_resources;
use wem_profiles::assembly::EncoderProfileResources;
use wem_profiles::bundle::{load_profile_bundle, load_profile_bundle_from_bytes, ProfileBundle};
pub use wem_profiles::data::DataDir;
use wem_profiles::embedded::load_embedded_profile_bundle;
use wem_profiles::error::ProfileError;
use wem_profiles::model::ContainerMetadata;
use wem_profiles::model::EncoderProfile;
use wem_profiles::registry::{
    embedded_registry, resolve_wem_profile_selection_quality, ProfileRegistry,
};
use wem_profiles::selection::WwiseProfile;
use wem_vorbis::codebook::Codebook;
use wem_vorbis::setup::SetupInfo;

use crate::error::{EncoderError, InternalError};
use crate::pack::pack_analysis_packet;

/// Minimum PCM frames for one encode (Python: 4096-frame lower bound).
pub const MIN_PCM_FRAMES: u32 = 4096;

// ---------------------------------------------------------------------------
// PCM input
// ---------------------------------------------------------------------------

/// Typed PCM input in the signed-16 wire representation
/// (Python `PcmBuffer`, i16 flavor).
///
/// Rows are channel-major with equal frame counts; the legacy float
/// normalization (`value / 32768.0`) is applied only at the analysis
/// boundary via [`Pcm16::to_float_rows`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Pcm16 {
    sample_rate: i64,
    channels: Vec<Vec<i16>>,
}

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
            channels,
        })
    }

    /// Construct from interleaved little-endian signed-16 PCM bytes
    /// (the interleaved wire form of the streaming API).
    ///
    /// The byte length must be a multiple of `2 * channel_count`; a trailing
    /// partial frame is a geometry violation (GEOMETRY_MISMATCH).
    pub fn from_interleaved_le(
        sample_rate: i64,
        channel_count: usize,
        bytes: &[u8],
    ) -> Result<Self, EncoderError> {
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
        let bytes_per_frame =
            channel_count
                .checked_mul(2)
                .ok_or_else(|| EncoderError::StateError {
                    message: "PCM channel count is too large".into(),
                })?;
        if !bytes.len().is_multiple_of(bytes_per_frame) {
            return Err(EncoderError::GeometryMismatch {
                message: "chunk carries a trailing partial PCM frame".into(),
            });
        }
        let frames = bytes.len() / bytes_per_frame;
        let mut channels = vec![Vec::with_capacity(frames); channel_count];
        for frame in 0..frames {
            for (slot, row) in channels.iter_mut().enumerate() {
                let offset = (frame * channel_count + slot) * 2;
                row.push(i16::from_le_bytes([bytes[offset], bytes[offset + 1]]));
            }
        }
        Self::new(sample_rate, channels)
    }

    pub fn sample_rate(&self) -> i64 {
        self.sample_rate
    }

    /// Number of channels (Python `channel_count`).
    pub fn channel_count(&self) -> usize {
        self.channels.len()
    }

    /// Frame count (Python `frame_count`).
    pub fn frame_count(&self) -> i64 {
        self.channels[0].len() as i64
    }

    /// Channel-major float rows at the legacy normalization
    /// (`value / 32768.0`), exactly as Python `read_pcm_wav` feeds
    /// `PcmBuffer`.
    pub fn to_float_rows(&self) -> Vec<Vec<f64>> {
        self.channels
            .iter()
            .map(|row| row.iter().map(|value| *value as f64 / 32768.0).collect())
            .collect()
    }
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
    pub fn from_profile(profile: &EncoderProfile) -> Self {
        let meta = profile.container_metadata();
        let fmt = from_container_metadata(meta);
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
            metadata_source: format!("profile:{}", profile.name()),
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
        let profile = resolve_wem_profile_selection_quality(selection, quality)
            .map_err(|error| selection_error(&error, selection))?;
        Self::from_profile_model(&profile, None)
    }

    /// Load one installed profile by name and construct the encoder
    /// (Python `load_wem_profile` + `Encoder.__init__`).
    pub fn from_profile(name: &str) -> Result<Self, EncoderError> {
        Self::from_profile_quality(name, None)
    }

    /// Load one installed profile from an explicit profile data tree.
    pub fn from_profile_in(data: &DataDir, name: &str) -> Result<Self, EncoderError> {
        Self::from_profile_quality_in(data, name, None)
    }

    /// Load one installed profile bound to an explicit quality factor
    /// (Python `load_wem_profile(name, quality=...)` + `Encoder.__init__`).
    ///
    /// `None` reproduces the historical behavior exactly; with a quality
    /// value the analysis-resource assembly interpolates the profile's
    /// quality curves (a missing quality-curves resource is a clear
    /// configuration error, never a silent fallback).
    pub fn from_profile_quality(name: &str, quality: Option<f64>) -> Result<Self, EncoderError> {
        let bundle = load_embedded_named_bundle(name)?;
        let mut profile = bundle.to_encoder_profile()?;
        if let Some(quality) = quality {
            profile = profile
                .with_quality(quality)
                .map_err(|error| EncoderError::Internal(InternalError::Profile(error)))?;
        }
        Self::from_profile_and_bundle(&profile, None, &bundle)
    }

    /// Load one installed profile from an explicit profile data tree and
    /// optionally bind a quality factor. The tree is the one passed in: the
    /// kernel resolves no profile data from the process environment.
    pub fn from_profile_quality_in(
        data: &DataDir,
        name: &str,
        quality: Option<f64>,
    ) -> Result<Self, EncoderError> {
        let bundle = load_named_bundle(data, name)?;
        let mut profile = bundle.to_encoder_profile()?;
        if let Some(quality) = quality {
            profile = profile
                .with_quality(quality)
                .map_err(|error| EncoderError::Internal(InternalError::Profile(error)))?;
        }
        Self::from_profile_and_bundle(&profile, None, &bundle)
    }

    /// Construct from an encoder profile (Python `Encoder.__init__`).
    ///
    /// `container` may override the profile-derived container plan
    /// (Python `_container` parameter); the geometry cross-check still
    /// applies. A quality factor bound to the profile (see
    /// [`wem_profiles::load_wem_profile_quality`]) is forwarded to the
    /// analysis-resource assembly; with no quality the historical bytes
    /// are reproduced exactly.
    pub fn from_profile_model(
        profile: &EncoderProfile,
        container: Option<ContainerPlan>,
    ) -> Result<Self, EncoderError> {
        let bundle = load_embedded_named_bundle(profile.name())?;
        Self::from_profile_and_bundle(profile, container, &bundle)
    }

    /// Construct from an encoder profile using an explicit profile data
    /// tree. The tree is the one passed in, never an ambient default.
    pub fn from_profile_model_in(
        data: &DataDir,
        profile: &EncoderProfile,
        container: Option<ContainerPlan>,
    ) -> Result<Self, EncoderError> {
        let bundle = load_profile_bundle(data, Some(profile.name()), false)?;
        Self::from_profile_and_bundle(profile, container, &bundle)
    }

    /// Construct the encoder from an in-memory profile bundle — no
    /// filesystem access (the threadless / wasm32-unknown-unknown entry).
    ///
    /// `index` and `files` carry the profile bytes exactly as documented on
    /// [`wem_profiles::load_profile_bundle_from_bytes`]; the index `default`
    /// profile is selected and every logical resource is SHA-256 verified on
    /// load. The output bytes are identical to the filesystem path for the
    /// same profile (see the `bytes_parity` integration test).
    pub fn from_profile_bytes(
        index: &[u8],
        files: impl IntoIterator<Item = (String, Vec<u8>)>,
    ) -> Result<Self, EncoderError> {
        Self::from_profile_bytes_with_quality(index, files, None)
    }

    /// Construct from a named profile in an in-memory bundle.
    pub fn from_profile_bytes_named(
        name: &str,
        index: &[u8],
        files: impl IntoIterator<Item = (String, Vec<u8>)>,
    ) -> Result<Self, EncoderError> {
        Self::from_profile_bytes_named_with_quality(name, index, files, None)
    }

    /// The bytes entry with an explicit quality factor: the same
    /// verification as [`from_profile_bytes`](Self::from_profile_bytes),
    /// with the quality bound to the assembled profile and forwarded to
    /// the analysis-resource assembly (None keeps the historical bytes).
    pub fn from_profile_bytes_with_quality(
        index: &[u8],
        files: impl IntoIterator<Item = (String, Vec<u8>)>,
        quality: Option<f64>,
    ) -> Result<Self, EncoderError> {
        Self::from_profile_bytes_selected(index, files, None, quality)
    }

    /// Construct from a named profile in an in-memory bundle and optionally
    /// bind a quality factor.
    pub fn from_profile_bytes_named_with_quality(
        name: &str,
        index: &[u8],
        files: impl IntoIterator<Item = (String, Vec<u8>)>,
        quality: Option<f64>,
    ) -> Result<Self, EncoderError> {
        Self::from_profile_bytes_selected(index, files, Some(name), quality)
    }

    fn from_profile_bytes_selected(
        index: &[u8],
        files: impl IntoIterator<Item = (String, Vec<u8>)>,
        name: Option<&str>,
        quality: Option<f64>,
    ) -> Result<Self, EncoderError> {
        let bundle = match load_profile_bundle_from_bytes(index, files, name, true) {
            Ok(bundle) => bundle,
            Err(ProfileError::ProfileNotInIndex { profile }) => {
                return Err(EncoderError::ProfileNotFound { requested: profile });
            }
            Err(error) => return Err(EncoderError::Internal(InternalError::Profile(error))),
        };
        let mut profile = bundle.to_encoder_profile()?;
        if let Some(quality) = quality {
            profile = profile
                .with_quality(quality)
                .map_err(|error| EncoderError::Internal(InternalError::Profile(error)))?;
        }
        Self::from_profile_and_bundle(&profile, None, &bundle)
    }

    /// Shared construction from one profile identity + one verified bundle;
    /// both the filesystem and the in-memory entries funnel through here so
    /// the cross-checks cannot drift between them.
    pub(crate) fn from_profile_and_bundle(
        profile: &EncoderProfile,
        container: Option<ContainerPlan>,
        bundle: &ProfileBundle,
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

        if bundle.key() != profile.key() {
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
        let setup_sha = bundle.setup()?.sha256().to_string();
        if profile.setup_sha256() != setup_sha {
            return Err(EncoderError::StateError {
                message: format!(
                    "selected profile {} differs from installed profile setup",
                    profile.name()
                ),
            });
        }
        // A profile whose setup is registered but whose analysis resources
        // (psychoacoustic tables, frozen tables) are still pending may be
        // resolved and inspected, but it must not encode: refuse with a
        // clear pending error instead of a mid-assembly fault.
        if analysis_resources_pending(bundle) {
            return Err(EncoderError::StateError {
                message: format!(
                    "profile '{}': analysis (psychoacoustic) resources pending; encoding unavailable",
                    profile.name()
                ),
            });
        }
        let setup_packet = profile.setup_packet()?;
        let resources =
            assemble_encoder_profile_resources(bundle, Some(&setup_packet), profile.quality())?;

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

/// Whether the profile's analysis (psychoacoustic) resources are still
/// pending: the profile carries its setup and codebooks but not the
/// psychoacoustic tables the analysis pipeline assembles.
fn analysis_resources_pending(bundle: &ProfileBundle) -> bool {
    const REQUIRED: &[&str] = &[
        "psychoacoustics.short-seed",
        "psychoacoustics.short-profiles",
        "psychoacoustics.long-base",
        "psychoacoustics.long-modes",
    ];
    REQUIRED
        .iter()
        .any(|key| bundle.runtime_manifest().resource(key).is_err())
}

/// Resolve an installed profile by PCM geometry
/// (Python `resolve_wem_profile`; used by the CLI when no profile is named).
pub fn resolve_profile_by_geometry(
    channels: i64,
    sample_rate: i64,
) -> Result<Encoder, EncoderError> {
    let registry = embedded_registry()?;
    let profile = registry.resolve_geometry(channels, sample_rate)?;
    Encoder::from_profile_model(profile, None)
}

/// Resolve a profile by PCM geometry from an explicit profile data tree.
pub fn resolve_profile_by_geometry_in(
    data: &DataDir,
    channels: i64,
    sample_rate: i64,
) -> Result<Encoder, EncoderError> {
    let registry: ProfileRegistry = wem_profiles::registry::installed_registry(data)?;
    let profile = registry.resolve_geometry(channels, sample_rate)?;
    Encoder::from_profile_model_in(data, profile, None)
}

/// Map a profile-resolution failure onto the caller-facing error class.
///
/// A selection that names no installed profile, or more than one, is a
/// caller-facing resolution failure (`WEM_ERR_PROFILE_NOT_FOUND`), not an
/// internal fault; an unrecognized generation code violates this revision's
/// contract (`WEM_ERR_FORMAT_UNSUPPORTED`); anything else stays internal.
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

fn load_named_bundle(data: &DataDir, name: &str) -> Result<ProfileBundle, EncoderError> {
    match load_profile_bundle(data, Some(name), false) {
        Ok(bundle) => Ok(bundle),
        Err(ProfileError::ProfileNotInIndex { .. }) | Err(ProfileError::UnknownProfile { .. }) => {
            Err(EncoderError::ProfileNotFound {
                requested: name.to_string(),
            })
        }
        Err(error) => Err(EncoderError::Internal(InternalError::Profile(error))),
    }
}

fn load_embedded_named_bundle(name: &str) -> Result<ProfileBundle, EncoderError> {
    match load_embedded_profile_bundle(Some(name)) {
        Ok(bundle) => Ok(bundle),
        Err(ProfileError::ProfileNotInIndex { .. }) | Err(ProfileError::UnknownProfile { .. }) => {
            Err(EncoderError::ProfileNotFound {
                requested: name.to_string(),
            })
        }
        Err(error) => Err(EncoderError::Internal(InternalError::Profile(error))),
    }
}
