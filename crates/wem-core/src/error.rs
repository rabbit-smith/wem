//! Orchestration-layer errors (wem-core).
//!
//! Each variant is one stable class of the kernel's error-code contract:
//! variant identity and order are part of the cross-language contract, and
//! the language shells (the C ABI core surface, the PyO3 binding) map them
//! one-to-one.
//!
//! Every public path returns one of these; none panics on input-derived
//! conditions (crates/AGENTS.md).

use wem_analysis::config::AnalysisError;
use wem_container::error::ContainerError;
use wem_profiles::error::ProfileError;
use wem_vorbis::packet_encoder::PacketError;

/// Terminal encoder failure carrying the kernel error class.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum EncoderError {
    /// No installed profile matches the requested setup digest.
    ProfileNotFound { requested: String },
    /// A request arrived outside the Init -> chunk -> Finish lifecycle, or
    /// a soft profile cross-check failed.
    StateError { message: String },
    /// Requested PCM channels or sample rate differ from the selected
    /// profile geometry.
    GeometryMismatch { message: String },
    /// The accumulated PCM stream is shorter than the 4096-frame minimum.
    InputTooShort { want: u32, got: u32 },
    /// The requested sample layout is not supported by this revision.
    FormatUnsupported { message: String },
    /// An internal fault while encoding.
    Internal(InternalError),
}

/// Kernel-stage failures surfaced as `Internal`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum InternalError {
    /// Profile resource / bundle / registry failure.
    Profile(ProfileError),
    /// Analysis-session or psychoacoustic pipeline failure.
    Analysis(AnalysisError),
    /// Vorbis packet assembly failure.
    Packet(PacketError),
    /// Container composition failure.
    Container(ContainerError),
    /// A structural invariant inside the orchestration layer failed.
    Invariant { message: &'static str },
    /// Input file I/O or decode failure.
    Io { message: String },
}

impl std::fmt::Display for EncoderError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            EncoderError::ProfileNotFound { requested } => {
                write!(f, "no installed profile matches {requested:?}")
            }
            EncoderError::StateError { message } => write!(f, "{message}"),
            EncoderError::GeometryMismatch { message } => write!(f, "{message}"),
            EncoderError::InputTooShort { want, got } => write!(
                f,
                "PCM input must contain at least {want} frames, got {got}"
            ),
            EncoderError::FormatUnsupported { message } => write!(f, "{message}"),
            EncoderError::Internal(inner) => write!(f, "encoder fault: {inner}"),
        }
    }
}

impl std::fmt::Display for InternalError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            InternalError::Profile(e) => write!(f, "profile: {e}"),
            InternalError::Analysis(e) => write!(f, "analysis: {e:?}"),
            InternalError::Packet(e) => write!(f, "packet: {e}"),
            InternalError::Container(e) => write!(f, "container: {e}"),
            InternalError::Invariant { message } => write!(f, "invariant violated: {message}"),
            InternalError::Io { message } => write!(f, "io: {message}"),
        }
    }
}

impl std::error::Error for EncoderError {}

impl std::error::Error for InternalError {}

impl From<ProfileError> for EncoderError {
    fn from(error: ProfileError) -> Self {
        EncoderError::Internal(InternalError::Profile(error))
    }
}

impl From<AnalysisError> for EncoderError {
    fn from(error: AnalysisError) -> Self {
        EncoderError::Internal(InternalError::Analysis(error))
    }
}

impl From<PacketError> for EncoderError {
    fn from(error: PacketError) -> Self {
        EncoderError::Internal(InternalError::Packet(error))
    }
}

impl From<ContainerError> for EncoderError {
    fn from(error: ContainerError) -> Self {
        EncoderError::Internal(InternalError::Container(error))
    }
}
