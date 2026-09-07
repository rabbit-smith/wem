//! Streaming encode session for the wwise.v1 contract.
//!
//! Mirrors the `wwise.v1` `Encode` request lifecycle
//! (`proto/wwise/v1/encode.proto`): exactly one `Init` (profile selection)
//! as the first request, zero or more PCM `chunk`s, then exactly one
//! `Finish`. Every lifecycle violation is reported as an
//! [`EncoderError`](crate::error::EncoderError) variant — none panics, and
//! none changes output bytes for valid streams.
//!
//! The future `wem-server` (gRPC) and PyO3 shells are thin wrappers over
//! this session; the reply side is a packet sequence (seq 0 = setup packet,
//! then audio packets) followed by the container summary, which
//! [`EncodeResult`](crate::encoder::EncodeResult) plus
//! [`load_wem_parts_bytes`](wem_container::load_wem_parts_bytes) provide.
//!
//! # Memory characteristics (documented, by design)
//!
//! This v1 reference implementation accumulates all PCM chunks in memory
//! until `finish`, then runs one complete encode. Chunk boundaries never
//! affect the output bytes (the frame-boundary state machine lives in the
//! analysis session), so this matches the contract; a future revision may
//! make the pipeline incrementally streaming without changing this API
//! shape.

use wem_profiles::data::DataDir;
use wem_profiles::registry::installed_registry;

use crate::encoder::{EncodeResult, Encoder, Pcm16, MIN_PCM_FRAMES};
use crate::error::EncoderError;

/// Profile selection reference (wwise.v1 `ProfileRef`).
///
/// `TemplateRef` is rejected in v1 (reserved for a future revision); this
/// type intentionally only carries the profile-reference fields.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProfileRef {
    /// Hard identity assertion: must exactly match an installed profile's
    /// setup SHA-256 (lowercase hex).
    pub setup_sha256: String,
    /// Soft cross-check: compared against the resolved profile name; a
    /// mismatch is a state error (the v1 reference server rejects it).
    pub name: Option<String>,
}

impl ProfileRef {
    /// A reference that asserts both the setup digest and the name.
    pub fn with_name(setup_sha256: impl Into<String>, name: impl Into<String>) -> Self {
        Self {
            setup_sha256: setup_sha256.into(),
            name: Some(name.into()),
        }
    }
}

/// One streaming encode session (wwise.v1 `Encode` request lifecycle).
///
/// Build with [`StreamSession::new`] and drive `init_profile` ->
/// `push_pcm_chunk`* -> `finish`, or use the convenience constructor
/// [`StreamSession::for_profile_ref`].
pub struct StreamSession {
    initialized: bool,
    finished: bool,
    encoder: Option<Encoder>,
    /// Accumulated raw little-endian PCM bytes (documented O(n) memory).
    pcm: Vec<u8>,
}

impl StreamSession {
    /// Create an uninitialized session (the v1 stream before `Init`).
    pub fn new() -> Self {
        Self {
            initialized: false,
            finished: false,
            encoder: None,
            pcm: Vec::new(),
        }
    }

    /// Open the session on one installed profile (v1 `Init`).
    ///
    /// - `PROFILE_NOT_FOUND` when no installed profile's setup SHA-256 matches.
    /// - `STATE_ERROR` on a second Init or a soft `name` cross-check failure.
    pub fn init_profile(&mut self, ref_: &ProfileRef) -> Result<(), EncoderError> {
        if self.initialized || self.finished {
            return Err(EncoderError::StateError {
                message: "Init must be the first request".into(),
            });
        }
        let data = DataDir::from_env()?;
        let registry = installed_registry(&data)?;
        let wanted = ref_.setup_sha256.to_lowercase();
        let candidate = registry
            .list()
            .into_iter()
            .find(|profile| profile.setup_sha256() == wanted);
        let profile = match candidate {
            Some(profile) => profile,
            None => {
                return Err(EncoderError::ProfileNotFound {
                    requested: ref_.setup_sha256.clone(),
                })
            }
        };
        if let Some(name) = &ref_.name {
            if name != profile.name() {
                return Err(EncoderError::StateError {
                    message: format!(
                        "profile name mismatch: setup_sha256 resolves to \
                         '{}', not '{name}'",
                        profile.name()
                    ),
                });
            }
        }
        let encoder = Encoder::from_profile_model(&profile, None)?;
        self.encoder = Some(encoder);
        self.initialized = true;
        Ok(())
    }

    /// Convenience constructor: `new()` + `init_profile()`.
    pub fn for_profile_ref(ref_: &ProfileRef) -> Result<Self, EncoderError> {
        let mut session = Self::new();
        session.init_profile(ref_)?;
        Ok(session)
    }

    /// Push one chunk of little-endian signed-16 interleaved PCM bytes
    /// (v1 `PcmChunk`, `PCM_SAMPLE_LAYOUT_SIGNED_16_INTERLEAVED`).
    ///
    /// `STATE_ERROR` before Init or after Finish; `GEOMETRY_MISMATCH` when
    /// the chunk carries a trailing partial PCM frame.
    pub fn push_pcm_chunk(&mut self, data: &[u8]) -> Result<(), EncoderError> {
        if !self.initialized {
            return Err(EncoderError::StateError {
                message: "Init is required before chunks".into(),
            });
        }
        if self.finished {
            return Err(EncoderError::StateError {
                message: "chunks are not allowed after Finish".into(),
            });
        }
        let channels = self
            .encoder
            .as_ref()
            .map(|encoder| encoder.profile().channels() as usize)
            .ok_or_else(|| EncoderError::StateError {
                message: "Init is required before chunks".into(),
            })?;
        if !data.len().is_multiple_of(2 * channels) {
            return Err(EncoderError::GeometryMismatch {
                message: "chunk carries a trailing partial PCM frame".into(),
            });
        }
        self.pcm.extend_from_slice(data);
        Ok(())
    }

    /// Mark the end of the PCM stream and run the complete encode
    /// (v1 `Finish`).
    ///
    /// `STATE_ERROR` when Init did not run or Finish already did;
    /// `INPUT_TOO_SHORT` below the 4096-frame minimum; otherwise the full
    /// encode result (container bytes + stats). The session is terminal
    /// after this call regardless of the outcome, matching the stream
    /// contract.
    pub fn finish(&mut self) -> Result<EncodeResult, EncoderError> {
        if !self.initialized || self.finished {
            return Err(EncoderError::StateError {
                message: "Finish must come exactly once, after Init".into(),
            });
        }
        self.finished = true;
        let encoder = self
            .encoder
            .as_ref()
            .ok_or_else(|| EncoderError::StateError {
                message: "Init is required before chunks".into(),
            })?;
        let channels = encoder.profile().channels() as usize;
        let frame_count = (self.pcm.len() / (2 * channels)) as u32;
        if frame_count < MIN_PCM_FRAMES {
            return Err(EncoderError::InputTooShort {
                want: MIN_PCM_FRAMES,
                got: frame_count,
            });
        }
        let pcm = Pcm16::from_interleaved_le(encoder.profile().sample_rate(), channels, &self.pcm)?;
        encoder.encode_pcm(&pcm)
    }

    /// The accumulated PCM bytes so far (streaming observability).
    pub fn pcm_bytes(&self) -> &[u8] {
        &self.pcm
    }
}

impl Default for StreamSession {
    fn default() -> Self {
        Self::new()
    }
}
