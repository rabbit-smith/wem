//! The public error enums are exhaustively matchable — and stay that way.
//!
//! `docs/reference/standards.md` (Errors) states the rule: a public error enum
//! carries no `#[non_exhaustive]`, on purpose, because adding a variant is a
//! breaking change — the compiler must tell every caller that a new failure
//! mode exists. This file is that rule made compiler-enforced. Each function
//! below matches one public error enum with **no `_` arm**, from an
//! integration test crate, which is exactly the caller's position: it sees the
//! public surface and nothing else.
//!
//! Adding a variant to any of these enums now fails to compile here, in the
//! same commit that adds it, and the failure names the new failure mode.
//!
//! Two surfaces are deliberately *not* covered:
//!
//! * `wem_capi::WemError` — the C ABI error-code table. That surface has no
//!   exhaustive matching by design (`include/wem.h` promises the values are
//!   stable and append-only, never renumbered or reused), so adding a code is
//!   not a breaking change and a match here would assert the opposite.
//! * The `#[non_exhaustive]`-free enums of a dependency outside this
//!   workspace: there are none.
//!
//! The conversion sites are the reason the primitives are listed too: the
//! kernel's own code turns a `FloorFitError` into a `PacketError` at the call
//! site (`wem-core/src/pack.rs`), so a caller of this crate is a caller of
//! that enum as well.

// The matches are the test; nothing needs to run them. `dead_code` is off for
// exactly that reason, so a variant list can never be "unused" into silence.
#![allow(dead_code)]

use wem_analysis::config::AnalysisError;
use wem_container::error::ContainerError;
use wem_core::error::{EncoderError, InternalError};
use wem_profiles::error::ProfileError;
use wem_scheduling::{PlannerError, SelectorError};
use wem_vorbis::bitio::BitError;
use wem_vorbis::codebook::CodebookError;
use wem_vorbis::floor::Floor1Error;
use wem_vorbis::floor_fit::FloorFitError;
use wem_vorbis::packet_encoder::PacketError;
use wem_vorbis::residue::ResidueError;
use wem_vorbis::setup::SetupError;

/// `wem_core::error::EncoderError` — 6 variants.
pub fn encoder_error(value: &EncoderError) {
    match value {
        EncoderError::ProfileNotFound { .. } => {}
        EncoderError::StateError { .. } => {}
        EncoderError::GeometryMismatch { .. } => {}
        EncoderError::InputTooShort { .. } => {}
        EncoderError::FormatUnsupported { .. } => {}
        EncoderError::Internal(..) => {}
    }
}

/// `wem_core::error::InternalError` — 6 variants.
pub fn internal_error(value: &InternalError) {
    match value {
        InternalError::Profile(..) => {}
        InternalError::Analysis(..) => {}
        InternalError::Packet(..) => {}
        InternalError::Container(..) => {}
        InternalError::Invariant { .. } => {}
        InternalError::Io { .. } => {}
    }
}

/// `wem_profiles::error::ProfileError` — 30 variants.
pub fn profile_error(value: &ProfileError) {
    match value {
        ProfileError::ProfileKeyNonPositive => {}
        ProfileError::ProfileKeyFieldEmpty { .. } => {}
        ProfileError::ProfileGeometryMismatch => {}
        ProfileError::ProfileSetupIdentityMismatch => {}
        ProfileError::ProfileBlockSizesMalformed => {}
        ProfileError::ProfileBlockSizesMismatch => {}
        ProfileError::BundleMissingVorbisSetup => {}
        ProfileError::RegistryDuplicateKey { .. } => {}
        ProfileError::UnknownProfileKey { .. } => {}
        ProfileError::UnknownWwiseVersion { .. } => {}
        ProfileError::UnsupportedWwiseGeneration { .. } => {}
        ProfileError::NoProfileForSelection { .. } => {}
        ProfileError::AmbiguousProfileSelection { .. } => {}
        ProfileError::SelectionGeometryNonPositive => {}
        ProfileError::UnknownBookTable { .. } => {}
        ProfileError::BookIdOutOfRange { .. } => {}
        ProfileError::LongVariantModeInvalid { .. } => {}
        ProfileError::QualityCurvesSchemaUnexpected { .. } => {}
        ProfileError::QualityCurvesTooFewBreakpoints => {}
        ProfileError::QualityCurvesBreakpointsNotIncreasing => {}
        ProfileError::QualityCurvesCurveLengthMismatch { .. } => {}
        ProfileError::QualityCurvesValueNonFinite { .. } => {}
        ProfileError::QualityCurvesEmptyCurves => {}
        ProfileError::QualityValueNonFinite => {}
        ProfileError::QualityCurvesResourceMissing { .. } => {}
        ProfileError::QualityCurveParameterUnsupported { .. } => {}
        ProfileError::QualityCurvesSemanticsIncomplete => {}
        ProfileError::Analysis(..) => {}
        ProfileError::Codebook(..) => {}
        ProfileError::Bit(..) => {}
    }
}

/// `wem_analysis::config::AnalysisError` — 150 variants.
pub fn analysis_error(value: &AnalysisError) {
    match value {
        AnalysisError::UnsupportedGeometry { .. } => {}
        AnalysisError::MalformedField { .. } => {}
        AnalysisError::IncompleteResources { .. } => {}
        AnalysisError::PsyLookGeometry { .. } => {}
        AnalysisError::PsyLookRowLength { .. } => {}
        AnalysisError::FrozenLnDomainMiss { .. } => {}
        AnalysisError::LongSeedGeometry => {}
        AnalysisError::LongSeedOctaveGeometry => {}
        AnalysisError::LongSeedLabelsMalformed => {}
        AnalysisError::LongSeedToneBanksMalformed => {}
        AnalysisError::LongFloorGeometry => {}
        AnalysisError::LongFloorScalars { .. } => {}
        AnalysisError::FftSizeInvalid { .. } => {}
        AnalysisError::SamplesShort { .. } => {}
        AnalysisError::FrozenTwiddlesMissing { .. } => {}
        AnalysisError::FrozenWindowHalfMismatch { .. } => {}
        AnalysisError::WindowSizeInvalid { .. } => {}
        AnalysisError::FrozenWindowDomainMiss { .. } => {}
        AnalysisError::PsyWindowSize { .. } => {}
        AnalysisError::TransientWindowGeometry { .. } => {}
        AnalysisError::TransientMdctGeometry { .. } => {}
        AnalysisError::BlockSizeInvalid { .. } => {}
        AnalysisError::BufferBlockOutOfRange => {}
        AnalysisError::WindowBlockSizeInvalid => {}
        AnalysisError::WindowStateIndexOutOfRange { .. } => {}
        AnalysisError::WindowIntervalsIncompatible => {}
        AnalysisError::LpcOrderInvalid { .. } => {}
        AnalysisError::LpcSamplesShort { .. } => {}
        AnalysisError::LpcCoefficientsEmpty => {}
        AnalysisError::LpcPrimeLengthMismatch { .. } => {}
        AnalysisError::LpcCountNegative { .. } => {}
        AnalysisError::LpcPrefillInvalid { .. } => {}
        AnalysisError::LpcBatchInvalid { .. } => {}
        AnalysisError::LpcSourceShort { .. } => {}
        AnalysisError::DetectorPcmEmpty => {}
        AnalysisError::DetectorPcmShort { .. } => {}
        AnalysisError::DetectorTerminalNegative { .. } => {}
        AnalysisError::DetectorPrefixNonPositive { .. } => {}
        AnalysisError::DetectorHopWindowInvalid { .. } => {}
        AnalysisError::DetectorQuantaOutOfRange { .. } => {}
        AnalysisError::FramePlansNotContiguous => {}
        AnalysisError::FramePlanIntervalMismatch => {}
        AnalysisError::FramePlanTransitionsDiffer => {}
        AnalysisError::PcmFeederEmpty => {}
        AnalysisError::InputConditionerChannelCountMismatch { .. } => {}
        AnalysisError::PcmFeederShort { .. } => {}
        AnalysisError::PcmChannelsUnequal { .. } => {}
        AnalysisError::DetectorChannelCountMismatch { .. } => {}
        AnalysisError::DetectorQuantumSamplesMismatch { .. } => {}
        AnalysisError::PsyEnergyRingSlots { .. } => {}
        AnalysisError::PsyBandStateRings { .. } => {}
        AnalysisError::PsyConfigRowShort { .. } => {}
        AnalysisError::PsyMaskShortForBands { .. } => {}
        AnalysisError::TransientMaskSamples { .. } => {}
        AnalysisError::PsySpectrumBinsOdd { .. } => {}
        AnalysisError::SessionChannelsNonPositive { .. } => {}
        AnalysisError::SessionSampleRateNonPositive { .. } => {}
        AnalysisError::SessionBlockSizeMismatch { .. } => {}
        AnalysisError::AnalysisFrameNotContiguous { .. } => {}
        AnalysisError::AdjacentAnalysisFrameModesDiffer => {}
        AnalysisError::ShortAnalysisWindowGeometry { .. } => {}
        AnalysisError::LongAnalysisWindowGeometry { .. } => {}
        AnalysisError::AnalysisWindowSamplesMismatch => {}
        AnalysisError::ManualIngestionMixed => {}
        AnalysisError::ModeSelectionNotFresh => {}
        AnalysisError::ModeSelectionPcmInvalid { .. } => {}
        AnalysisError::TransitionCodeMissing { .. } => {}
        AnalysisError::ShortVectorsLength { .. } => {}
        AnalysisError::ShortModeOutOfRange { .. } => {}
        AnalysisError::ShortModeBiasOutOfRange { .. } => {}
        AnalysisError::ShortGroupWorkLength { .. } => {}
        AnalysisError::ShortFrameChannelCountMismatch => {}
        AnalysisError::ShortVariantInvalid { .. } => {}
        AnalysisError::ShortFollowingModeInvalid { .. } => {}
        AnalysisError::ShortChannelStateGeometry => {}
        AnalysisError::ShortAnalyzerChannelsNonPositive { .. } => {}
        AnalysisError::ShortAnalyzerProfiles { .. } => {}
        AnalysisError::ShortAnalyzerChannelStateCount { .. } => {}
        AnalysisError::LongVariantInvalid { .. } => {}
        AnalysisError::LongAnalysisEmpty => {}
        AnalysisError::LongAnalysisFrameSize { .. } => {}
        AnalysisError::LongAnalysisTableBins { .. } => {}
        AnalysisError::LongAnalysisScratchCount { .. } => {}
        AnalysisError::LongAnalysisStreamChannels { .. } => {}
        AnalysisError::LongMdctLookGeometry { .. } => {}
        AnalysisError::LongAnalysisBothScratchAndStream => {}
        AnalysisError::ShortAnalysisEmpty => {}
        AnalysisError::ShortAnalysisChannelCount { .. } => {}
        AnalysisError::ShortAnalysisFrameSize { .. } => {}
        AnalysisError::ShortAnalysisGroupWorkCount { .. } => {}
        AnalysisError::ShortMdctLookGeometry { .. } => {}
        AnalysisError::LongStateBridgeGeometry => {}
        AnalysisError::LongStateBridgeCount { .. } => {}
        AnalysisError::LongToShortHistoryLength { .. } => {}
        AnalysisError::ShortToLongHistoryLength { .. } => {}
        AnalysisError::FloorEnvelopeScratchNonPositive { .. } => {}
        AnalysisError::FloorEnvelopeCurveLengthMismatch { .. } => {}
        AnalysisError::FloorEnvelopeSourceLengthMismatch { .. } => {}
        AnalysisError::FloorEnvelopeModeUnsupported { .. } => {}
        AnalysisError::FirstFloorEnvelopeBufferMismatch { .. } => {}
        AnalysisError::FirstFloorEnvelopePeakBranch => {}
        AnalysisError::LongFloorEnvelopeGeometry { .. } => {}
        AnalysisError::LongFloorEnvelopePeakBranch => {}
        AnalysisError::FirstLongFloorEnvelopePeakBranch => {}
        AnalysisError::LongFloorEnvelopeNoLook => {}
        AnalysisError::QualityExtrapolationWithoutValue => {}
        AnalysisError::PsyCurveLengthMismatch { .. } => {}
        AnalysisError::PsyLookCurveLengthMismatch { .. } => {}
        AnalysisError::PsyIntervalTableShort { .. } => {}
        AnalysisError::PsyIntervalEndpointOutOfRange { .. } => {}
        AnalysisError::PsyBaseSelectorLengthMismatch { .. } => {}
        AnalysisError::PsyLookMissingAth { .. } => {}
        AnalysisError::PsySeedSpectrumLength { .. } => {}
        AnalysisError::PsyLookMissingToneCurves => {}
        AnalysisError::ToneCurveBankShort { .. } => {}
        AnalysisError::ToneCurvePostArrayShort => {}
        AnalysisError::ToneCurvePostsShort => {}
        AnalysisError::ToneCurveBandBankShort { .. } => {}
        AnalysisError::SeedLoopInputLengthMismatch { .. } => {}
        AnalysisError::SeedPositionOutOfRange { .. } => {}
        AnalysisError::SeedSurfaceShort { .. } => {}
        AnalysisError::OctaveFloorLengthMismatch => {}
        AnalysisError::RelaxWidthNonPositive => {}
        AnalysisError::RelaxLengthMismatch => {}
        AnalysisError::RebaseLengthMismatch { .. } => {}
        AnalysisError::ShortTemporalBins { .. } => {}
        AnalysisError::ShortTemporalLength { .. } => {}
        AnalysisError::LongSeedLogFftLength { .. } => {}
        AnalysisError::LongRemapBins { .. } => {}
        AnalysisError::LongRemapMode2Bins { .. } => {}
        AnalysisError::LongActiveSpanInvalid { .. } => {}
        AnalysisError::LongRemapLutShort { .. } => {}
        AnalysisError::LongRemapVariantBins { .. } => {}
        AnalysisError::LongFrameVariantInvalid { .. } => {}
        AnalysisError::FftCurveEmpty => {}
        AnalysisError::SeedTotalLinesNonPositive { .. } => {}
        AnalysisError::SpecmaxGeometry { .. } => {}
        AnalysisError::SeedCursorOutOfRange { .. } => {}
        AnalysisError::EnvelopeScratchLengthNonPositive { .. } => {}
        AnalysisError::EnvelopeStageLengthMismatch { .. } => {}
        AnalysisError::EnvelopeStageModeUnsupported { .. } => {}
        AnalysisError::FloorTransitionBins { .. } => {}
        AnalysisError::FirstEnvelopeBuffersMismatch { .. } => {}
        AnalysisError::FirstEnvelopeEnteredPeakBranch => {}
        AnalysisError::LongEnvelopeStateBins => {}
        AnalysisError::ShortKernelWords { .. } => {}
        AnalysisError::ShortAnalysisCannotApplyLongTransition { .. } => {}
        AnalysisError::StreamFeederChannelsMismatch { .. } => {}
        AnalysisError::StreamFeederSourceShort { .. } => {}
        AnalysisError::StreamFeederWindowNotReady { .. } => {}
    }
}

/// `wem_vorbis::packet_encoder::PacketError` — 26 variants.
pub fn packet_error(value: &PacketError) {
    match value {
        PacketError::ModeOutOfRange { .. } => {}
        PacketError::MappingOutOfRange { .. } => {}
        PacketError::FloorIndexOutOfRange { .. } => {}
        PacketError::ResidueIndexOutOfRange { .. } => {}
        PacketError::SubclassBookIndexOutOfRange { .. } => {}
        PacketError::MasterBookIndexOutOfRange { .. } => {}
        PacketError::MasterBookMissing { .. } => {}
        PacketError::BadYLength { .. } => {}
        PacketError::MdctTooShort { .. } => {}
        PacketError::MdctRowCount { .. } => {}
        PacketError::Codebook(..) => {}
        PacketError::Floor1(..) => {}
        PacketError::FloorFit(..) => {}
        PacketError::Residue(..) => {}
        PacketError::AnalysisChannelsMismatch { .. } => {}
        PacketError::CouplingPeakGeometry => {}
        PacketError::ChannelMuxTooShort { .. } => {}
        PacketError::FloorMapTooShort { .. } => {}
        PacketError::MissingResidueSubmap => {}
        PacketError::FloorMultiplierOutOfRange { .. } => {}
        PacketError::FloorClassOutOfRange { .. } => {}
        PacketError::CouplingChannelOutOfRange { .. } => {}
        PacketError::CouplingRowLengthMismatch => {}
        PacketError::CouplingChannelsEqual { .. } => {}
        PacketError::NonFiniteAnalysisSample { .. } => {}
        PacketError::ResidueCouplingOverflow => {}
    }
}

/// `wem_container::error::ContainerError` — 12 variants.
pub fn container_error(value: &ContainerError) {
    match value {
        ContainerError::NotRiff => {}
        ContainerError::TruncatedRiff { .. } => {}
        ContainerError::BadChunkId { .. } => {}
        ContainerError::FmtTooShort { .. } => {}
        ContainerError::FmtSize { .. } => {}
        ContainerError::BadSeekTableSize { .. } => {}
        ContainerError::PacketTooLarge { .. } => {}
        ContainerError::TruncatedPacketStream { .. } => {}
        ContainerError::MissingChunk { .. } => {}
        ContainerError::PacketWalkFailed { .. } => {}
        ContainerError::BadEndian { .. } => {}
        ContainerError::TerminalExcessTooLarge { .. } => {}
    }
}

/// `wem_vorbis::codebook::CodebookError` — 20 variants.
pub fn codebook_error(value: &CodebookError) {
    match value {
        CodebookError::RowMissingField { .. } => {}
        CodebookError::LengthlistMismatch { .. } => {}
        CodebookError::DimTooSmall { .. } => {}
        CodebookError::OverpopulatedTree { .. } => {}
        CodebookError::LengthOutOfRange { .. } => {}
        CodebookError::CodePrefixCollision { .. } => {}
        CodebookError::CodeEmbedsLeaf { .. } => {}
        CodebookError::Maptype1RequiresQuantlist => {}
        CodebookError::QuantvalsNonPositive => {}
        CodebookError::QuantlistTooShort { .. } => {}
        CodebookError::EntryOutOfRange { .. } => {}
        CodebookError::EntryUnused { .. } => {}
        CodebookError::EmptyCodebook => {}
        CodebookError::InvalidHuffmanCode => {}
        CodebookError::NotMaptype1 { .. } => {}
        CodebookError::NoVqVector { .. } => {}
        CodebookError::VqMisuse { .. } => {}
        CodebookError::TargetTooShort { .. } => {}
        CodebookError::NoUsableVqEntries => {}
        CodebookError::PackBitsOutOfRange { .. } => {}
    }
}

/// `wem_vorbis::bitio::BitError` — 2 variants.
pub fn bit_error(value: &BitError) {
    match value {
        BitError::OutOfBits => {}
        BitError::BitsOutOfRange { .. } => {}
    }
}

/// `wem_vorbis::setup::SetupError` — 2 variants.
pub fn setup_error(value: &SetupError) {
    match value {
        SetupError::FieldMissing { .. } => {}
        SetupError::Bit(..) => {}
    }
}

/// `wem_vorbis::floor::Floor1Error` — 3 variants.
pub fn floor1_error(value: &Floor1Error) {
    match value {
        Floor1Error::LengthMismatch { .. } => {}
        Floor1Error::TooFewPosts { .. } => {}
        Floor1Error::InvalidMultiplier { .. } => {}
    }
}

/// `wem_vorbis::floor_fit::FloorFitError` — 1 variants.
pub fn floor_fit_error(value: &FloorFitError) {
    match value {
        FloorFitError::CurvesTooShort => {}
    }
}

/// `wem_vorbis::residue::ResidueError` — 9 variants.
pub fn residue_error(value: &ResidueError) {
    match value {
        ResidueError::SilentRequiresEmptyCascade => {}
        ResidueError::UnsupportedClassCount { .. } => {}
        ResidueError::BookIndexOutOfRange { .. } => {}
        ResidueError::UnencodableClassword { .. } => {}
        ResidueError::InvalidPartitionSize => {}
        ResidueError::InvalidChannelCount => {}
        ResidueError::ChannelLayoutMismatch { .. } => {}
        ResidueError::ClassTablesTooShort { .. } => {}
        ResidueError::IncompatibleBookDimension { .. } => {}
    }
}

/// `wem_scheduling::planner::PlannerError` — 7 variants.
pub fn planner_error(value: &PlannerError) {
    match value {
        PlannerError::BlockSizeCount => {}
        PlannerError::BlockSizeNotEven => {}
        PlannerError::ModeNotBit => {}
        PlannerError::AppendCountNegative => {}
        PlannerError::InsufficientSamples { .. } => {}
        PlannerError::NonPositiveAdvance => {}
        PlannerError::StateDiverged => {}
    }
}

/// `wem_scheduling::selector::SelectorError` — 6 variants.
pub fn selector_error(value: &SelectorError) {
    match value {
        SelectorError::HopCapacityNonPositive => {}
        SelectorError::QueueSlotOutOfRange { .. } => {}
        SelectorError::GenerationGeometry => {}
        SelectorError::TransitionLookModesInvalid => {}
        SelectorError::MissingFlagRows => {}
        SelectorError::SurplusFlagRows => {}
    }
}

#[test]
fn a_real_fault_reaches_a_callers_own_arms() {
    // One run through the caller's view, so this file is not only a
    // compile-time guard: a real resolution failure arrives at the caller's
    // own arm, and the match that classified it stays exhaustive.
    let error = wem_core::Encoder::new(
        wem_profiles::WwiseProfile::new(wem_profiles::WwiseVersion::Wwise2013, 2, 44_100)
            .expect("positive geometry"),
    )
    .expect_err("2ch/44100 is not an installed configuration");
    match &error {
        EncoderError::ProfileNotFound { .. } => {}
        other => panic!("expected ProfileNotFound, got {other}"),
    }
    encoder_error(&error);
    internal_error(&InternalError::Invariant { message: "x" });
    profile_error(&ProfileError::QualityValueNonFinite);
    container_error(&ContainerError::NotRiff);
    packet_error(&PacketError::MasterBookMissing { class: 0 });
    codebook_error(&CodebookError::Maptype1RequiresQuantlist);
    bit_error(&BitError::OutOfBits);
    setup_error(&SetupError::FieldMissing { field: "x" });
    floor1_error(&Floor1Error::TooFewPosts { posts: 1 });
    floor_fit_error(&FloorFitError::CurvesTooShort);
    residue_error(&ResidueError::InvalidPartitionSize);
    planner_error(&PlannerError::BlockSizeCount);
    selector_error(&SelectorError::HopCapacityNonPositive);
    analysis_error(&AnalysisError::PsyCurveLengthMismatch { want: 2, got: 1 });
}
