//! Orchestration-layer errors (wem-core).
//!
//! Each variant is one stable class of the kernel's error-code table,
//! pinned for the cross-language shells in `include/wem.h` (the C ABI
//! error-code table, 1:1 with these variants; see crates/AGENTS.md,
//! "C ABI surface").
//!
//! Every public path returns one of these; none panics on input-derived
//! conditions (crates/AGENTS.md).

use wem_analysis::config::AnalysisError;
use wem_container::error::ContainerError;
use wem_profiles::error::ProfileError;
use wem_vorbis::bitio::BitError;
use wem_vorbis::floor::Floor1Error;
use wem_vorbis::packet_decoder::PacketDecodeError;
use wem_vorbis::packet_encoder::PacketError;

/// Terminal encoder failure carrying the kernel error class.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum EncoderError {
    /// No installed profile matches the requested selection.
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
            InternalError::Analysis(e) => write!(f, "analysis: {e}"),
            InternalError::Packet(e) => write!(f, "packet: {e}"),
            InternalError::Container(e) => write!(f, "container: {e}"),
            InternalError::Invariant { message } => write!(f, "invariant violated: {message}"),
            InternalError::Io { message } => write!(f, "io: {message}"),
        }
    }
}

impl std::error::Error for EncoderError {
    /// The wrapped kernel-stage failure: without this the underlying cause
    /// ("why is this an INTERNAL?") is unreachable from the error value.
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            EncoderError::Internal(inner) => Some(inner),
            EncoderError::ProfileNotFound { .. }
            | EncoderError::StateError { .. }
            | EncoderError::GeometryMismatch { .. }
            | EncoderError::InputTooShort { .. }
            | EncoderError::FormatUnsupported { .. } => None,
        }
    }
}

impl std::error::Error for InternalError {
    /// The stage error this fault wraps; `Invariant` and `Io` carry their
    /// diagnostic inline and have no nested cause.
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            InternalError::Profile(e) => Some(e),
            InternalError::Analysis(e) => Some(e),
            InternalError::Packet(e) => Some(e),
            InternalError::Container(e) => Some(e),
            InternalError::Invariant { .. } | InternalError::Io { .. } => None,
        }
    }
}

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

// ---------------------------------------------------------------------------
// Decode
// ---------------------------------------------------------------------------

/// Terminal decode failure carrying the kernel error class.
///
/// Deliberately its own enum rather than variants on [`EncoderError`]: the
/// decode direction has failure modes the encode direction cannot reach (a
/// bitstream that does not parse) and lacks ones it can (no PCM geometry to
/// disagree with, no minimum length), so extending `EncoderError` would give
/// every encode caller a set of arms that can never be produced. Both enums
/// are exhaustively matchable without `#[non_exhaustive]`, per the errors
/// norm.
///
/// The class split is *by whether parsing succeeded*: a container, a setup
/// packet or an audio packet that fails to parse is
/// [`DecoderError`]'s malformed-input family; a container that parses cleanly
/// but names a configuration the compiled carrier does not hold is the
/// unsupported-configuration family; a request outside the lifecycle is
/// [`DecoderError::StateError`]; and anything left is a defect in this
/// library. `crates/wem-capi` maps those four families onto
/// `WEM_ERR_INPUT_MALFORMED`, `WEM_ERR_FORMAT_UNSUPPORTED`,
/// `WEM_ERR_STATE_ERROR` and `WEM_ERR_INTERNAL` (include/wem.h).
///
/// Every variant carries the values it observed, and every variant that wraps
/// a kernel-stage or codec-stage failure keeps it reachable through
/// [`std::error::Error::source`].
#[derive(Debug, Clone, PartialEq)]
pub enum DecoderError {
    /// The RIFF/WAVE framing, a chunk payload, or the size-prefixed packet
    /// stream failed to parse. Malformed input.
    Container(ContainerError),
    /// The setup packet ran out of bits while being parsed. Malformed input.
    Setup { source: BitError },
    /// The setup packet parsed but did not close: it stopped short of its own
    /// length, or its trailing byte padding was not zero. Malformed input.
    SetupPadding {
        /// Bit position the parse stopped at.
        end_bit: u64,
        /// The packet's length in bits.
        total_bits: u64,
        /// Trailing bits after the last field.
        pad_bits: u64,
        /// What those trailing bits held.
        pad_value: u64,
    },
    /// The pushed bytes ended before the container did. Malformed input.
    Truncated {
        /// Bytes of the data payload consumed before the stream ran out.
        stream_offset: u64,
        /// What the session was waiting for when it ran out.
        need: &'static str,
    },
    /// The data payload ended without a setup packet, so the container holds
    /// no configuration to decode with. Malformed input.
    MissingSetup {
        /// Declared extent of the data payload that carried no packets.
        data_size: usize,
    },
    /// The container parses, but its format tag is not the Wwise Vorbis one
    /// this revision decodes.
    NotWwiseVorbis {
        /// The tag the container's `fmt ` chunk carried.
        format_tag: u16,
    },
    /// The container's geometry names no configuration the compiled carrier
    /// holds. A configuration this revision does not support, never a
    /// substituted default.
    ConfigurationUnsupported {
        /// Channel count the container declared.
        channels: i64,
        /// Sample rate the container declared.
        sample_rate: i64,
        /// The carrier's own resolution failure (it lists what is installed).
        source: ProfileError,
    },
    /// The container's setup packet is not the one the carrier holds for the
    /// container's geometry: a WEM from a configuration this revision does
    /// not carry. This is the whole of the version question — there is no
    /// version field to test (include/wem.h, and the design proposal's
    /// "the generation is not a checked field").
    SetupNotCarried {
        /// Length of the setup packet the container carried.
        container_len: usize,
        /// Length of the setup packet the carrier holds.
        carried_len: usize,
        /// First byte that differs, when both are long enough to compare.
        first_difference: Option<usize>,
    },
    /// The container's declared block sizes disagree with the resolved
    /// profile's. Malformed input: the geometry is the profile's identity, so
    /// a disagreement is not a decode attempt. Malformed input.
    BlockSizeMismatch {
        /// Block sizes the container's `fmt ` chunk declared.
        container: [i64; 2],
        /// Block sizes the resolved profile carries.
        profile: [i64; 2],
    },
    /// An audio packet failed to parse. Malformed input.
    Packet {
        /// Zero-based index of the packet within the audio packet stream.
        index: u64,
        /// The packet codec's own failure, with its bit positions.
        source: PacketDecodeError,
    },
    /// A residue stopped on a codeword the codebook does not assign while the
    /// packet still had bits left to read: a bitstream defect rather than the
    /// format's own early end.
    ///
    /// `Codebook::decode` reports a failed bit read and an unassigned tree
    /// branch as one error, so the two cannot be told apart from its return
    /// value. They *can* be told apart from the reader's state: a read that
    /// ran out leaves no bits behind, while an unassigned branch is only
    /// reached after a successful bit read, so bits remain. That is the test
    /// this variant reports, and it is deliberately one-directional — a
    /// missing branch taken on the packet's very last bit leaves no bits
    /// either, and is accepted as an early end. See
    /// [`crate::decoder`] for the full policy. Malformed input.
    ResidueBitstreamDefect {
        /// Zero-based index of the packet within the audio packet stream.
        index: u64,
        /// Which residue submap stopped.
        submap: usize,
        /// Bit position the residue stopped at.
        bit_position: u64,
        /// The packet's length in bits.
        packet_bits: u64,
    },
    /// The container's packet stream does not cover the frame count the
    /// container declares. Malformed input.
    FrameCountMismatch {
        /// `dw_total_pcm_frames` from the container.
        declared: u64,
        /// Frames the packet stream and its geometry actually cover.
        synthesized: u64,
    },
    /// A request arrived outside the `Init -> push* -> Finish` lifecycle.
    StateError { message: String },
    /// A floor1 step failed against the resolved setup rather than against
    /// the bitstream: the packet's post row and the setup's post list
    /// disagree in length, or the setup's multiplier has no quant range. With
    /// a carrier-verified setup this is a defect in this library's compiled
    /// data, not a rejection of the caller's input.
    Floor1 {
        /// Channel the floor step ran for.
        channel: usize,
        /// The codec's own failure.
        source: Floor1Error,
    },
    /// An internal fault while decoding.
    Internal(InternalError),
}

impl std::fmt::Display for DecoderError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            DecoderError::Container(e) => write!(f, "container: {e}"),
            DecoderError::Setup { source } => write!(f, "setup packet: {source}"),
            DecoderError::SetupPadding {
                end_bit,
                total_bits,
                pad_bits,
                pad_value,
            } => write!(
                f,
                "setup packet did not close: parsed to bit {end_bit} of {total_bits}, \
                 with {pad_bits} trailing bits holding {pad_value}"
            ),
            DecoderError::Truncated {
                stream_offset,
                need,
            } => write!(
                f,
                "input ended {stream_offset} bytes into the data payload, waiting for {need}"
            ),
            DecoderError::MissingSetup { data_size } => write!(
                f,
                "the {data_size}-byte data payload carries no setup packet"
            ),
            DecoderError::NotWwiseVorbis { format_tag } => write!(
                f,
                "not a Wwise Vorbis container: format tag 0x{format_tag:04X}"
            ),
            DecoderError::ConfigurationUnsupported {
                channels,
                sample_rate,
                source,
            } => write!(
                f,
                "no configuration for {channels}ch/{sample_rate}Hz: {source}"
            ),
            DecoderError::SetupNotCarried {
                container_len,
                carried_len,
                first_difference,
            } => match first_difference {
                Some(offset) => write!(
                    f,
                    "the container's setup packet is not the one this build carries: \
                     {container_len} bytes vs {carried_len}, first difference at byte {offset}"
                ),
                None => write!(
                    f,
                    "the container's setup packet is not the one this build carries: \
                     {container_len} bytes vs {carried_len}"
                ),
            },
            DecoderError::BlockSizeMismatch { container, profile } => write!(
                f,
                "container block sizes {container:?} disagree with the profile's {profile:?}"
            ),
            DecoderError::Packet { index, source } => {
                write!(f, "audio packet {index}: {source}")
            }
            DecoderError::ResidueBitstreamDefect {
                index,
                submap,
                bit_position,
                packet_bits,
            } => write!(
                f,
                "audio packet {index} residue submap {submap} stopped on an unassigned \
                 codeword at bit {bit_position} of {packet_bits}"
            ),
            DecoderError::FrameCountMismatch {
                declared,
                synthesized,
            } => write!(
                f,
                "container declares {declared} PCM frames but its packet stream \
                 synthesizes {synthesized}"
            ),
            DecoderError::StateError { message } => write!(f, "{message}"),
            DecoderError::Floor1 { channel, source } => {
                write!(f, "channel {channel} floor1: {source}")
            }
            DecoderError::Internal(inner) => write!(f, "decoder fault: {inner}"),
        }
    }
}

impl std::error::Error for DecoderError {
    /// The wrapped kernel-stage failure: without this the underlying cause
    /// ("why is this malformed, why is this an INTERNAL?") is unreachable from
    /// the error value.
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            DecoderError::Container(e) => Some(e),
            DecoderError::Setup { source } => Some(source),
            DecoderError::ConfigurationUnsupported { source, .. } => Some(source),
            DecoderError::Packet { source, .. } => Some(source),
            DecoderError::Floor1 { source, .. } => Some(source),
            DecoderError::Internal(inner) => Some(inner),
            DecoderError::SetupPadding { .. }
            | DecoderError::Truncated { .. }
            | DecoderError::MissingSetup { .. }
            | DecoderError::NotWwiseVorbis { .. }
            | DecoderError::SetupNotCarried { .. }
            | DecoderError::BlockSizeMismatch { .. }
            | DecoderError::ResidueBitstreamDefect { .. }
            | DecoderError::FrameCountMismatch { .. }
            | DecoderError::StateError { .. } => None,
        }
    }
}

impl From<ContainerError> for DecoderError {
    fn from(error: ContainerError) -> Self {
        DecoderError::Container(error)
    }
}

impl From<AnalysisError> for DecoderError {
    fn from(error: AnalysisError) -> Self {
        DecoderError::Internal(InternalError::Analysis(error))
    }
}

impl From<ProfileError> for DecoderError {
    fn from(error: ProfileError) -> Self {
        DecoderError::Internal(InternalError::Profile(error))
    }
}
