//! Immutable typed inputs consumed by the analysis domain.
//!
//! Mirrors Python `wwise_wem/analysis/config.py`. The profile layer owns
//! decoding and validating packaged calibration files; analysis receives only
//! the value objects defined here and never selects or opens a resource.

use std::collections::HashMap;

pub const TONE_LEVEL_COUNT: i64 = 8;
pub const TONE_BAND_COUNT: i64 = 17;
pub const EHMER_OFFSET: i64 = 16;
pub const TONE_REFERENCE_LEVEL_DB: f32 = 30.0;
pub const NEGATIVE_INFINITY_DB: f32 = -9999.0;
pub const SPECTRUM_PEAK_DECAY_DB_PER_SECOND: f32 = 6.0;
/// Calibration sample rates accepted by the psychoacoustic loaders
/// (Python `CALIBRATION_SAMPLE_RATES`). Each resource declares its own
/// geometry; the loaders check that declared geometry against this set
/// instead of one hard-coded rate.
pub const CALIBRATION_SAMPLE_RATES: [i64; 2] = [44100, 48000];

/// Round an f64 to its f32-representable value (Python `_f32`), keeping the
/// f32: the spelling for statements whose result is stored as an f32 word
/// (the materialized `WwisePsyLook` fields).
#[inline]
pub fn f32_round(value: f64) -> f32 {
    value as f32
}

/// Python `_f32` returning f64: round to the f32-representable value and
/// promote back to f64.
///
/// The boundary has three spellings in this crate, and a Python `_f32(...)`
/// maps to the one that matches how the value is stored — not always to this
/// one:
///
/// - this function, where the rounded value flows on as an f64 (analysis,
///   transient and psychoacoustic statements);
/// - [`crate::dsp::x87::f32_round`], on the bit-exact geometry port
///   (`dsp/psy_geom.rs`), which spells the same rounding with the name of the
///   `fstps` site it reproduces;
/// - [`f32_round`], where the result is stored as an f32.
///
/// They are one rounding under three names, not three definitions of the
/// boundary's value.
#[inline]
pub fn f32_of(value: f64) -> f64 {
    value as f32 as f64
}

/// Interpret a stored u32 bit pattern as an IEEE-754 float32 (Python `_u32_f32`).
#[inline]
pub fn u32_to_f32(bits: u32) -> f32 {
    f32::from_bits(bits)
}

/// Interpret a stored u32 as a signed 32-bit value (Python `_signed_u32`).
#[inline]
pub fn signed_u32(value: u32) -> i64 {
    value as i32 as i64
}

/// Structural validation errors for analysis-side aggregates.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum AnalysisError {
    /// MDCT/transient/psychoacoustic geometry is unsupported.
    UnsupportedGeometry { reason: &'static str },
    /// A required field or section is missing or malformed.
    MalformedField { reason: &'static str },
    /// AnalysisProfileResources lacks required MDCT looks / psy variants.
    IncompleteResources { reason: &'static str },
    /// `make_wwise_psy_look`: short psychoacoustic geometry unsupported.
    PsyLookGeometry { n: i64, sample_rate: i64 },
    /// `make_wwise_psy_look`: row length mismatch.
    PsyLookRowLength { got: usize, want: usize },
    /// `make_wwise_psy_look`: coordinate outside the frozen ln domain.
    FrozenLnDomainMiss { frequency_bits: u64 },
    /// Long seed table geometry disagreement.
    LongSeedGeometry,
    /// Long seed table octave geometry invalid.
    LongSeedOctaveGeometry,
    /// Long seed group labels malformed.
    LongSeedLabelsMalformed,
    /// Long seed tone-bank geometry malformed.
    LongSeedToneBanksMalformed,
    /// Long floor-envelope table geometry unsupported.
    LongFloorGeometry,
    /// Long floor-envelope scalar fields invalid.
    LongFloorScalars { len: usize, value: i64 },
    // ------------------------------------------------------------------
    // runtime rejection conditions (Python ValueError/RuntimeError family
    // in analysis/*) — one variant per normative rejection site
    // ------------------------------------------------------------------
    /// "FFT size must be an even power of two >= 2"
    FftSizeInvalid { n: i64 },
    /// "need {need} samples, got {got}"
    SamplesShort { need: i64, got: i64 },
    /// "FFT stage fell outside the frozen twiddle domain"
    FrozenTwiddlesMissing { length: i64 },
    /// "frozen window half differs from the requested size"
    FrozenWindowHalfMismatch { size: i64, want: i64, got: i64 },
    /// "window size must be a positive even number"
    WindowSizeInvalid { n: i64 },
    /// "block window fell outside the frozen window domain"
    FrozenWindowDomainMiss { size: i64 },
    /// "psycho window needs at least two samples"
    PsyWindowSize { n: i64 },
    /// "transient window geometry differs from MDCT size"
    TransientWindowGeometry { tables_n: i64, n: i64 },
    /// "transient MDCT look geometry differs from samples"
    TransientMdctGeometry { look_n: i64, n: i64 },
    /// "current block size must be a positive even number"
    BlockSizeInvalid { n: i64 },
    /// "buffer does not contain the requested analysis block"
    BufferBlockOutOfRange,
    /// "block sizes must be positive even numbers"
    WindowBlockSizeInvalid,
    /// "window state index out of range"
    WindowStateIndexOutOfRange { index: i64 },
    /// "incompatible block/window sizes"
    WindowIntervalsIncompatible,
    /// "LPC order must be positive"
    LpcOrderInvalid { order: i64 },
    /// "LPC input must be longer than its order"
    LpcSamplesShort { order: i64, got: i64 },
    /// "LPC coefficients must not be empty"
    LpcCoefficientsEmpty,
    /// "LPC prime length must equal coefficient order"
    LpcPrimeLengthMismatch { want: i64, got: i64 },
    /// "LPC prediction count must not be negative"
    LpcCountNegative { count: i64 },
    /// "priming prefill must be positive"
    LpcPrefillInvalid { prefill: i64 },
    /// "priming batch must be longer than LPC order"
    LpcBatchInvalid { batch: i64, order: i64 },
    /// "source does not contain the first priming batch"
    LpcSourceShort { want: i64, got: i64 },
    /// "transient detector needs at least one PCM channel"
    DetectorPcmEmpty,
    /// "transient detector PCM must be equal-length and at least 4096 samples"
    DetectorPcmShort { frames: i64 },
    /// "transient-detector terminal prediction count must be non-negative"
    DetectorTerminalNegative { terminal: i64 },
    /// "transient-detector prefix length must be positive"
    DetectorPrefixNonPositive { prefix: i64 },
    /// "transient-detector hop/window must be positive"
    DetectorHopWindowInvalid { hop: i64, window: i64 },
    /// "requested {requested} transient quanta; only {available} available"
    DetectorQuantaOutOfRange { requested: i64, available: i64 },
    /// "frame plans must be contiguous and zero-based"
    FramePlansNotContiguous,
    /// "frame plan interval differs from its block mode"
    FramePlanIntervalMismatch,
    /// "adjacent frame plan transitions differ"
    FramePlanTransitionsDiffer,
    /// "PCM feeder needs at least one channel"
    PcmFeederEmpty,
    /// "input conditioner channel count differs"
    InputConditionerChannelCountMismatch { want: i64, got: i64 },
    /// "PCM feeder needs 4096 samples for Wwise LPC priming"
    PcmFeederShort { frames: i64 },
    /// "all PCM channels must have the same frame count"
    PcmChannelsUnequal { want: i64, got: i64 },
    /// "transient quantum channel count differs from detector"
    DetectorChannelCountMismatch { want: i64 },
    /// "transient quantum must contain 128 samples per channel"
    DetectorQuantumSamplesMismatch { want: i64 },
    /// "psycho energy ring must contain 15 slots"
    PsyEnergyRingSlots { want: usize, got: usize },
    /// "psycho band state must contain twelve bands"
    PsyBandStateRings { want: usize },
    /// "psycho config row must contain fields 0..25"
    PsyConfigRowShort { want: usize },
    /// "psycho mask is shorter than the band table"
    PsyMaskShortForBands { want: i64, got: i64 },
    /// "transient mask requires 128 samples"
    TransientMaskSamples { want: i64, got: i64 },
    /// "psycho spectrum must contain an even number of bins"
    PsySpectrumBinsOdd { bins: i64 },
    /// "stream needs at least one channel"
    SessionChannelsNonPositive { channels: i64 },
    /// "sample rate must be positive"
    SessionSampleRateNonPositive { sample_rate: i64 },
    /// "the checked stream state expects 256/2048 blocks"
    SessionBlockSizeMismatch { got: [i64; 2] },
    /// "analysis frames must be contiguous: expected index {}, got {}"
    AnalysisFrameNotContiguous { expected: i64, got: i64 },
    /// "adjacent analysis frame modes differ"
    AdjacentAnalysisFrameModesDiffer,
    /// "short analysis expects a 256-sample scheduled window"
    ShortAnalysisWindowGeometry { want: i64 },
    /// "long analysis expects a 2048-sample scheduled window"
    LongAnalysisWindowGeometry { want: i64 },
    /// "analysis window samples differ from its scheduled mode"
    AnalysisWindowSamplesMismatch,
    /// "manual transient ingestion cannot be mixed with mode generation"
    ManualIngestionMixed,
    /// "mode selection requires a fresh stream selector"
    ModeSelectionNotFresh,
    /// "mode selection PCM must be equal-length and at least 4096 samples"
    ModeSelectionPcmInvalid { frames: i64 },
    /// A selected frame has no transition code captured at mode-scan time.
    TransitionCodeMissing { index: i64, recorded: usize },
    /// "short psychoacoustic vectors must each contain 128 values"
    ShortVectorsLength { want: i64 },
    /// "short psychoacoustic mode is outside the curve table"
    ShortModeOutOfRange { mode: i64, curves: i64 },
    /// "short psychoacoustic mode is outside the bias table"
    ShortModeBiasOutOfRange { mode: i64 },
    /// "short psychoacoustic group work has the wrong length"
    ShortGroupWorkLength { want: i64, got: i64 },
    /// "short psychoacoustic frame channel count differs"
    ShortFrameChannelCountMismatch,
    /// "short psychoacoustic variant must be 0 or 1"
    ShortVariantInvalid { variant: i64 },
    /// "short psychoacoustic following mode must be 0 or 1"
    ShortFollowingModeInvalid { following: i64 },
    /// "short psychoacoustic channel state has wrong geometry"
    ShortChannelStateGeometry,
    /// "short psychoacoustic analyzer needs at least one channel"
    ShortAnalyzerChannelsNonPositive { channels: i64 },
    /// "short psychoacoustic analyzer needs exactly two profiles"
    ShortAnalyzerProfiles { want: i64, got: i64 },
    /// "short psychoacoustic channel-state count differs"
    ShortAnalyzerChannelStateCount { want: i64 },
    /// "long psychoacoustic variant must be 0 or 1"
    LongVariantInvalid { variant: i64 },
    /// "long analysis needs at least one channel frame"
    LongAnalysisEmpty,
    /// "long analysis expects 2048-sample windowed frames"
    LongAnalysisFrameSize { want: i64 },
    /// "long analysis table must have 1024 output bins"
    LongAnalysisTableBins { want: i64 },
    /// "long analysis scratch count must equal channel count"
    LongAnalysisScratchCount { want: i64 },
    /// "long analysis stream channel count differs from frames"
    LongAnalysisStreamChannels { want: i64 },
    /// "long MDCT look differs from analysis geometry"
    LongMdctLookGeometry { want: i64 },
    /// "long analysis accepts either scratch or shared stream, not both"
    LongAnalysisBothScratchAndStream,
    /// "short analysis needs at least one channel frame"
    ShortAnalysisEmpty,
    /// "short analysis channel count differs from state owner"
    ShortAnalysisChannelCount { want: i64 },
    /// "short analysis expects 256-sample windowed frames"
    ShortAnalysisFrameSize { want: i64 },
    /// "short analysis group-work count differs from channels"
    ShortAnalysisGroupWorkCount { want: i64 },
    /// "short MDCT look differs from analysis geometry"
    ShortMdctLookGeometry { want: i64 },
    /// "long state bridge needs one 1024-bin raw curve per channel"
    LongStateBridgeGeometry,
    /// "long state bridge needs one 1024-bin raw curve per channel" (count)
    LongStateBridgeCount { want: i64 },
    /// "long-to-short floor reduction expects 1024 bins"
    LongToShortHistoryLength { want_raw: i64, want_state: i64 },
    /// "short-to-long floor expansion expects 128 bins"
    ShortToLongHistoryLength { want: i64 },
    /// "floor-envelope stage scratch length must be positive"
    FloorEnvelopeScratchNonPositive { n: i64 },
    /// "floor-envelope stage curves must have equal lengths"
    FloorEnvelopeCurveLengthMismatch { n: i64 },
    /// "floor-envelope stage source curve must have equal length"
    FloorEnvelopeSourceLengthMismatch { n: i64 },
    /// "floor-envelope stage regular port currently targets the regular branch"
    FloorEnvelopeModeUnsupported { mode: i64 },
    /// "first regular floor-envelope stage buffers must share one length"
    FirstFloorEnvelopeBufferMismatch { n: i64 },
    /// "first regular floor-envelope stage call entered peak branch"
    FirstFloorEnvelopePeakBranch,
    /// "long regular floor-envelope stage needs 1024 state bins and 128-or-1024 history bins"
    LongFloorEnvelopeGeometry { n: i64 },
    /// "fresh long regular floor-envelope stage call entered peak branch"
    LongFloorEnvelopePeakBranch,
    /// "cleared first regular floor-envelope stage call entered peak branch"
    FirstLongFloorEnvelopePeakBranch,
    /// "floor-envelope stage requires look or table"
    LongFloorEnvelopeNoLook,
    /// "quality extrapolation requires a quality value"
    QualityExtrapolationWithoutValue,
    /// "psycho curves must have the same length"
    PsyCurveLengthMismatch { want: i64, got: i64 },
    /// "psycho look and curve length differ"
    PsyLookCurveLengthMismatch { look_n: i64, got: i64 },
    /// "psycho interval table is shorter than the curve"
    PsyIntervalTableShort { want: i64, got: i64 },
    /// "psycho interval endpoint out of range"
    PsyIntervalEndpointOutOfRange { start: i64, end: i64, n: i64 },
    /// "psycho base and selector curves must have the same length"
    PsyBaseSelectorLengthMismatch { want: i64, got: i64 },
    /// "psycho look is missing ATH state"
    PsyLookMissingAth { n: i64 },
    /// "psycho seed spectrum and look length differ"
    PsySeedSpectrumLength { want: i64, got: i64 },
    /// "psycho look is missing ATH or tone-curve state"
    PsyLookMissingToneCurves,
    /// "tone-curve level bank is shorter than the reference encoder bank"
    ToneCurveBankShort { want: i64, got: i64 },
    /// "tone-curve post array needs start and end"
    ToneCurvePostArrayShort,
    /// "tone-curve posts are shorter than their end"
    ToneCurvePostsShort,
    /// "tone-curve band bank is shorter than 17"
    ToneCurveBandBankShort { want: i64, got: i64 },
    /// "seed-loop inputs must have equal bin counts"
    SeedLoopInputLengthMismatch { want: i64 },
    /// "seed position falls outside total octave lines"
    SeedPositionOutOfRange { pos: i64, total: i64 },
    /// "seed surface is shorter than total octave lines"
    SeedSurfaceShort { want: i64 },
    /// "octave and floor curves must have equal bin counts"
    OctaveFloorLengthMismatch,
    /// "history width table contains a nonpositive value"
    RelaxWidthNonPositive,
    /// "history/raw/widths must have equal length"
    RelaxLengthMismatch,
    /// "state/history length must equal n"
    RebaseLengthMismatch { want: i64 },
    /// "short temporal history expects 128 bins"
    ShortTemporalBins { want: i64 },
    /// "short temporal history curves must have 128 bins"
    ShortTemporalLength { want: i64 },
    /// "long floor-seed stage logFFT geometry differs from seed look"
    LongSeedLogFftLength { want: i64, got: i64 },
    /// "long psychoacoustic remap expects 1024 bins"
    LongRemapBins { want: i64 },
    /// "long psychoacoustic remap mode 2 expects 1024 bins"
    LongRemapMode2Bins { want: i64 },
    /// "long psycho table has an invalid mode-2 active span"
    LongActiveSpanInvalid { active: i64 },
    /// "long psycho table has no 40-entry remap LUT"
    LongRemapLutShort { got: i64 },
    /// "long psychoacoustic remap expects 1024 bins" (variant builder)
    LongRemapVariantBins { want: i64 },
    /// "long analysis variant must be 0 or 1"
    LongFrameVariantInvalid { variant: i64 },
    /// "FFT curve must contain at least one bin"
    FftCurveEmpty,
    /// "total octave lines must be positive"
    SeedTotalLinesNonPositive { total: i64 },
    /// "spectrum peak decay geometry is invalid"
    SpecmaxGeometry { block_bins: i64, sample_rate: i64 },
    /// "seed cursor left the seeded surface"
    SeedCursorOutOfRange { cursor: i64, total: i64 },
    /// "floor-envelope scratch length must be positive"
    EnvelopeScratchLengthNonPositive { n: i64 },
    /// "floor-envelope stage curves must have equal lengths"
    EnvelopeStageLengthMismatch { want: i64 },
    /// "floor-envelope stage regular port currently targets the regular branch"
    EnvelopeStageModeUnsupported { mode: i64 },
    /// "floor-envelope transition history expects a specific bin count"
    FloorTransitionBins { want: i64, got: i64 },
    /// "first regular floor-envelope stage buffers must share one length"
    FirstEnvelopeBuffersMismatch { want: i64 },
    /// "cleared first regular floor-envelope stage call entered peak branch"
    FirstEnvelopeEnteredPeakBranch,
    /// "long regular floor-envelope stage needs 1024 state bins and 128-or-1024 history bins"
    LongEnvelopeStateBins,
    /// "short psychoacoustic kernel must contain six words"
    ShortKernelWords { got: i64 },
    /// "short analysis cannot apply a long transition code"
    ShortAnalysisCannotApplyLongTransition { transition: i64 },
    /// "stream feeder push channels differ from the profile"
    StreamFeederChannelsMismatch { want: i64, got: i64 },
    /// "stream feeder source is shorter than the LPC boundary batch"
    StreamFeederSourceShort { frames: i64 },
    /// "stream frame window references samples not yet retained"
    StreamFeederWindowNotReady { want_from: i64, want_to: i64 },
    /// The host refused to start the analysis channel pool's workers.
    ///
    /// The one variant here with no counterpart in the Python reference: it
    /// reports a host resource failure rather than a rejected input. The cause
    /// travels as its rendered message because this enum is `PartialEq + Eq`
    /// and exists in builds without rayon, so the host's own error value cannot
    /// be held here; `Display` carries the observed worker count and the cause
    /// together.
    PoolUnavailable { workers: usize, cause: String },
}

impl std::fmt::Display for AnalysisError {
    /// One message per variant: the normative rejection condition this
    /// variant stands for, with the observed values that make it actionable.
    /// A caller reads this text; it is never the `Debug` rendering of the
    /// error value.
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        use AnalysisError::*;
        match self {
            UnsupportedGeometry { reason } => {
                write!(f, "unsupported analysis geometry: {reason}")
            }
            MalformedField { reason } => write!(f, "malformed analysis field: {reason}"),
            IncompleteResources { reason } => {
                write!(f, "incomplete analysis resources: {reason}")
            }
            PsyLookGeometry { n, sample_rate } => write!(
                f,
                "psychoacoustic look geometry is unsupported for n={n} at {sample_rate}Hz"
            ),
            PsyLookRowLength { got, want } => write!(
                f,
                "psychoacoustic look row has {got} values, expected {want}"
            ),
            FrozenLnDomainMiss { frequency_bits } => write!(
                f,
                "frequency bits {frequency_bits:#010x} have no frozen ln entry"
            ),
            LongSeedGeometry => {
                write!(f, "long seed table geometry disagrees with the analysis geometry")
            }
            LongSeedOctaveGeometry => write!(f, "long seed table octave geometry is invalid"),
            LongSeedLabelsMalformed => write!(f, "long seed group labels are malformed"),
            LongSeedToneBanksMalformed => {
                write!(f, "long seed tone-bank geometry is malformed")
            }
            LongFloorGeometry => write!(f, "long floor-envelope table geometry is unsupported"),
            LongFloorScalars { len, value } => write!(
                f,
                "long floor-envelope scalar fields are invalid (length {len}, value {value})"
            ),
            FftSizeInvalid { n } => {
                write!(f, "FFT size must be an even power of two >= 2, got {n}")
            }
            SamplesShort { need, got } => write!(f, "need {need} samples, got {got}"),
            FrozenTwiddlesMissing { length } => write!(
                f,
                "FFT stage fell outside the frozen twiddle domain (length {length})"
            ),
            FrozenWindowHalfMismatch { size, want, got } => write!(
                f,
                "frozen window half differs from the requested size: size={size}, want={want}, got={got}"
            ),
            WindowSizeInvalid { n } => {
                write!(f, "window size must be a positive even number, got {n}")
            }
            FrozenWindowDomainMiss { size } => write!(
                f,
                "block window fell outside the frozen window domain (size {size})"
            ),
            PsyWindowSize { n } => {
                write!(f, "psycho window needs at least two samples, got {n}")
            }
            TransientWindowGeometry { tables_n, n } => write!(
                f,
                "transient window geometry differs from the MDCT size: tables_n={tables_n}, n={n}"
            ),
            TransientMdctGeometry { look_n, n } => write!(
                f,
                "transient MDCT look geometry differs from the samples: look_n={look_n}, n={n}"
            ),
            BlockSizeInvalid { n } => write!(
                f,
                "current block size must be a positive even number, got {n}"
            ),
            BufferBlockOutOfRange => {
                write!(f, "buffer does not contain the requested analysis block")
            }
            WindowBlockSizeInvalid => write!(f, "block sizes must be positive even numbers"),
            WindowStateIndexOutOfRange { index } => {
                write!(f, "window state index out of range: {index}")
            }
            WindowIntervalsIncompatible => write!(f, "incompatible block/window sizes"),
            LpcOrderInvalid { order } => write!(f, "LPC order must be positive, got {order}"),
            LpcSamplesShort { order, got } => write!(
                f,
                "LPC input must be longer than its order {order}, got {got} samples"
            ),
            LpcCoefficientsEmpty => write!(f, "LPC coefficients must not be empty"),
            LpcPrimeLengthMismatch { want, got } => write!(
                f,
                "LPC prime length must equal the coefficient order {want}, got {got}"
            ),
            LpcCountNegative { count } => write!(
                f,
                "LPC prediction count must not be negative, got {count}"
            ),
            LpcPrefillInvalid { prefill } => {
                write!(f, "priming prefill must be positive, got {prefill}")
            }
            LpcBatchInvalid { batch, order } => write!(
                f,
                "priming batch must be longer than the LPC order {order}, got {batch}"
            ),
            LpcSourceShort { want, got } => write!(
                f,
                "source does not contain the first priming batch: want {want}, got {got}"
            ),
            DetectorPcmEmpty => {
                write!(f, "transient detector needs at least one PCM channel")
            }
            DetectorPcmShort { frames } => write!(
                f,
                "transient detector PCM must be equal-length and at least 4096 samples, got {frames}"
            ),
            DetectorTerminalNegative { terminal } => write!(
                f,
                "transient-detector terminal prediction count must be non-negative, got {terminal}"
            ),
            DetectorPrefixNonPositive { prefix } => write!(
                f,
                "transient-detector prefix length must be positive, got {prefix}"
            ),
            DetectorHopWindowInvalid { hop, window } => write!(
                f,
                "transient-detector hop/window must be positive: hop={hop}, window={window}"
            ),
            DetectorQuantaOutOfRange {
                requested,
                available,
            } => write!(
                f,
                "requested {requested} transient quanta; only {available} available"
            ),
            FramePlansNotContiguous => write!(f, "frame plans must be contiguous and zero-based"),
            FramePlanIntervalMismatch => {
                write!(f, "frame plan interval differs from its block mode")
            }
            FramePlanTransitionsDiffer => write!(f, "adjacent frame plan transitions differ"),
            PcmFeederEmpty => write!(f, "PCM feeder needs at least one channel"),
            InputConditionerChannelCountMismatch { want, got } => write!(
                f,
                "input conditioner channel count differs: want {want}, got {got}"
            ),
            PcmFeederShort { frames } => write!(
                f,
                "PCM feeder needs 4096 samples for Wwise LPC priming, got {frames}"
            ),
            PcmChannelsUnequal { want, got } => write!(
                f,
                "all PCM channels must have the same frame count: want {want}, got {got}"
            ),
            DetectorChannelCountMismatch { want } => write!(
                f,
                "transient quantum channel count differs from the detector: want {want}"
            ),
            DetectorQuantumSamplesMismatch { want } => write!(
                f,
                "transient quantum must contain {want} samples per channel"
            ),
            PsyEnergyRingSlots { want, got } => write!(
                f,
                "psycho energy ring must contain {want} slots, got {got}"
            ),
            PsyBandStateRings { want } => {
                write!(f, "psycho band state must contain {want} bands")
            }
            PsyConfigRowShort { want } => {
                write!(f, "psycho config row must contain fields 0..{want}")
            }
            PsyMaskShortForBands { want, got } => write!(
                f,
                "psycho mask is shorter than the band table: want {want}, got {got}"
            ),
            TransientMaskSamples { want, got } => write!(
                f,
                "transient mask requires {want} samples, got {got}"
            ),
            PsySpectrumBinsOdd { bins } => write!(
                f,
                "psycho spectrum must contain an even number of bins, got {bins}"
            ),
            SessionChannelsNonPositive { channels } => {
                write!(f, "stream needs at least one channel, got {channels}")
            }
            SessionSampleRateNonPositive { sample_rate } => {
                write!(f, "sample rate must be positive, got {sample_rate}")
            }
            SessionBlockSizeMismatch { got } => write!(
                f,
                "the checked stream state expects 256/2048 blocks, got {got:?}"
            ),
            AnalysisFrameNotContiguous { expected, got } => write!(
                f,
                "analysis frames must be contiguous: expected index {expected}, got {got}"
            ),
            AdjacentAnalysisFrameModesDiffer => {
                write!(f, "adjacent analysis frame modes differ")
            }
            ShortAnalysisWindowGeometry { want } => write!(
                f,
                "short analysis expects a {want}-sample scheduled window"
            ),
            LongAnalysisWindowGeometry { want } => write!(
                f,
                "long analysis expects a {want}-sample scheduled window"
            ),
            AnalysisWindowSamplesMismatch => {
                write!(f, "analysis window samples differ from its scheduled mode")
            }
            ManualIngestionMixed => write!(
                f,
                "manual transient ingestion cannot be mixed with mode generation"
            ),
            ModeSelectionNotFresh => {
                write!(f, "mode selection requires a fresh stream selector")
            }
            ModeSelectionPcmInvalid { frames } => write!(
                f,
                "mode selection PCM must be equal-length and at least 4096 samples, got {frames}"
            ),
            TransitionCodeMissing { index, recorded } => write!(
                f,
                "no transition code was captured at mode-scan time for frame {index} (recorded {recorded})"
            ),
            ShortVectorsLength { want } => write!(
                f,
                "short psychoacoustic vectors must each contain {want} values"
            ),
            ShortModeOutOfRange { mode, curves } => write!(
                f,
                "short psychoacoustic mode {mode} is outside the curve table ({curves} curves)"
            ),
            ShortModeBiasOutOfRange { mode } => write!(
                f,
                "short psychoacoustic mode {mode} is outside the bias table"
            ),
            ShortGroupWorkLength { want, got } => write!(
                f,
                "short psychoacoustic group work has the wrong length: want {want}, got {got}"
            ),
            ShortFrameChannelCountMismatch => {
                write!(f, "short psychoacoustic frame channel count differs")
            }
            ShortVariantInvalid { variant } => write!(
                f,
                "short psychoacoustic variant must be 0 or 1, got {variant}"
            ),
            ShortFollowingModeInvalid { following } => write!(
                f,
                "short psychoacoustic following mode must be 0 or 1, got {following}"
            ),
            ShortChannelStateGeometry => write!(
                f,
                "short psychoacoustic channel state has the wrong geometry"
            ),
            ShortAnalyzerChannelsNonPositive { channels } => write!(
                f,
                "short psychoacoustic analyzer needs at least one channel, got {channels}"
            ),
            ShortAnalyzerProfiles { want, got } => write!(
                f,
                "short psychoacoustic analyzer needs exactly {want} profiles, got {got}"
            ),
            ShortAnalyzerChannelStateCount { want } => write!(
                f,
                "short psychoacoustic channel-state count differs: want {want}"
            ),
            LongVariantInvalid { variant } => write!(
                f,
                "long psychoacoustic variant must be 0 or 1, got {variant}"
            ),
            LongAnalysisEmpty => write!(f, "long analysis needs at least one channel frame"),
            LongAnalysisFrameSize { want } => {
                write!(f, "long analysis expects {want}-sample windowed frames")
            }
            LongAnalysisTableBins { want } => {
                write!(f, "long analysis table must have {want} output bins")
            }
            LongAnalysisScratchCount { want } => write!(
                f,
                "long analysis scratch count must equal the channel count ({want})"
            ),
            LongAnalysisStreamChannels { want } => write!(
                f,
                "long analysis stream channel count differs from the frames: want {want}"
            ),
            LongMdctLookGeometry { want } => write!(
                f,
                "long MDCT look differs from the analysis geometry: want {want}"
            ),
            LongAnalysisBothScratchAndStream => write!(
                f,
                "long analysis accepts either scratch or a shared stream, not both"
            ),
            ShortAnalysisEmpty => write!(f, "short analysis needs at least one channel frame"),
            ShortAnalysisChannelCount { want } => write!(
                f,
                "short analysis channel count differs from its state owner: want {want}"
            ),
            ShortAnalysisFrameSize { want } => {
                write!(f, "short analysis expects {want}-sample windowed frames")
            }
            ShortAnalysisGroupWorkCount { want } => write!(
                f,
                "short analysis group-work count differs from the channels: want {want}"
            ),
            ShortMdctLookGeometry { want } => write!(
                f,
                "short MDCT look differs from the analysis geometry: want {want}"
            ),
            LongStateBridgeGeometry => write!(
                f,
                "long state bridge needs one 1024-bin raw curve per channel"
            ),
            LongStateBridgeCount { want } => write!(
                f,
                "long state bridge needs {want} 1024-bin raw curves"
            ),
            LongToShortHistoryLength {
                want_raw,
                want_state,
            } => write!(
                f,
                "long-to-short floor reduction expects {want_raw} raw and {want_state} state bins"
            ),
            ShortToLongHistoryLength { want } => {
                write!(f, "short-to-long floor expansion expects {want} bins")
            }
            FloorEnvelopeScratchNonPositive { n } => write!(
                f,
                "floor-envelope stage scratch length must be positive, got {n}"
            ),
            FloorEnvelopeCurveLengthMismatch { n } => write!(
                f,
                "floor-envelope stage curves must have equal lengths ({n})"
            ),
            FloorEnvelopeSourceLengthMismatch { n } => write!(
                f,
                "floor-envelope stage source curve must have the same length ({n})"
            ),
            FloorEnvelopeModeUnsupported { mode } => write!(
                f,
                "floor-envelope stage regular port targets the regular branch only, got mode {mode}"
            ),
            FirstFloorEnvelopeBufferMismatch { n } => write!(
                f,
                "first regular floor-envelope stage buffers must share one length ({n})"
            ),
            FirstFloorEnvelopePeakBranch => write!(
                f,
                "first regular floor-envelope stage call entered the peak branch"
            ),
            LongFloorEnvelopeGeometry { n } => write!(
                f,
                "long regular floor-envelope stage needs 1024 state bins and 128-or-1024 history bins (n={n})"
            ),
            LongFloorEnvelopePeakBranch => write!(
                f,
                "fresh long regular floor-envelope stage call entered the peak branch"
            ),
            FirstLongFloorEnvelopePeakBranch => write!(
                f,
                "cleared first long regular floor-envelope stage call entered the peak branch"
            ),
            LongFloorEnvelopeNoLook => {
                write!(f, "floor-envelope stage requires a look or a table")
            }
            QualityExtrapolationWithoutValue => {
                write!(f, "quality extrapolation requires a quality value")
            }
            PsyCurveLengthMismatch { want, got } => write!(
                f,
                "psycho curves must have the same length: want {want}, got {got}"
            ),
            PsyLookCurveLengthMismatch { look_n, got } => write!(
                f,
                "psycho look and curve length differ: look_n={look_n}, got {got}"
            ),
            PsyIntervalTableShort { want, got } => write!(
                f,
                "psycho interval table is shorter than the curve: want {want}, got {got}"
            ),
            PsyIntervalEndpointOutOfRange { start, end, n } => write!(
                f,
                "psycho interval endpoint out of range: {start}..{end} of {n}"
            ),
            PsyBaseSelectorLengthMismatch { want, got } => write!(
                f,
                "psycho base and selector curves must have the same length: want {want}, got {got}"
            ),
            PsyLookMissingAth { n } => {
                write!(f, "psycho look of {n} bins is missing its ATH state")
            }
            PsySeedSpectrumLength { want, got } => write!(
                f,
                "psycho seed spectrum and look length differ: want {want}, got {got}"
            ),
            PsyLookMissingToneCurves => {
                write!(f, "psycho look is missing ATH or tone-curve state")
            }
            ToneCurveBankShort { want, got } => write!(
                f,
                "tone-curve level bank is shorter than the reference bank: want {want}, got {got}"
            ),
            ToneCurvePostArrayShort => {
                write!(f, "tone-curve post array needs a start and an end")
            }
            ToneCurvePostsShort => write!(f, "tone-curve posts are shorter than their end"),
            ToneCurveBandBankShort { want, got } => write!(
                f,
                "tone-curve band bank is shorter than {want}: got {got}"
            ),
            SeedLoopInputLengthMismatch { want } => write!(
                f,
                "seed-loop inputs must have equal bin counts ({want})"
            ),
            SeedPositionOutOfRange { pos, total } => write!(
                f,
                "seed position {pos} falls outside {total} total octave lines"
            ),
            SeedSurfaceShort { want } => {
                write!(f, "seed surface is shorter than {want} total octave lines")
            }
            OctaveFloorLengthMismatch => {
                write!(f, "octave and floor curves must have equal bin counts")
            }
            RelaxWidthNonPositive => {
                write!(f, "history width table contains a nonpositive value")
            }
            RelaxLengthMismatch => write!(f, "history/raw/widths must have equal length"),
            RebaseLengthMismatch { want } => {
                write!(f, "state/history length must equal {want}")
            }
            ShortTemporalBins { want } => {
                write!(f, "short temporal history expects {want} bins")
            }
            ShortTemporalLength { want } => {
                write!(f, "short temporal history curves must have {want} bins")
            }
            LongSeedLogFftLength { want, got } => write!(
                f,
                "long floor-seed stage logFFT geometry differs from the seed look: want {want}, got {got}"
            ),
            LongRemapBins { want } => {
                write!(f, "long psychoacoustic remap expects {want} bins")
            }
            LongRemapMode2Bins { want } => {
                write!(f, "long psychoacoustic remap mode 2 expects {want} bins")
            }
            LongActiveSpanInvalid { active } => write!(
                f,
                "long psycho table has an invalid mode-2 active span ({active})"
            ),
            LongRemapLutShort { got } => write!(
                f,
                "long psycho table has no 40-entry remap LUT, got {got}"
            ),
            LongRemapVariantBins { want } => write!(
                f,
                "long psychoacoustic remap variant expects {want} bins"
            ),
            LongFrameVariantInvalid { variant } => write!(
                f,
                "long analysis variant must be 0 or 1, got {variant}"
            ),
            FftCurveEmpty => write!(f, "FFT curve must contain at least one bin"),
            SeedTotalLinesNonPositive { total } => {
                write!(f, "total octave lines must be positive, got {total}")
            }
            SpecmaxGeometry {
                block_bins,
                sample_rate,
            } => write!(
                f,
                "spectrum peak decay geometry is invalid: block_bins={block_bins}, sample_rate={sample_rate}"
            ),
            SeedCursorOutOfRange { cursor, total } => write!(
                f,
                "seed cursor {cursor} left the seeded surface of {total} lines"
            ),
            EnvelopeScratchLengthNonPositive { n } => write!(
                f,
                "floor-envelope scratch length must be positive, got {n}"
            ),
            EnvelopeStageLengthMismatch { want } => write!(
                f,
                "floor-envelope stage curves must have equal lengths ({want})"
            ),
            EnvelopeStageModeUnsupported { mode } => write!(
                f,
                "floor-envelope stage regular port targets the regular branch only, got mode {mode}"
            ),
            FloorTransitionBins { want, got } => write!(
                f,
                "floor-envelope transition history expects {want} bins, got {got}"
            ),
            FirstEnvelopeBuffersMismatch { want } => write!(
                f,
                "first regular floor-envelope stage buffers must share one length ({want})"
            ),
            FirstEnvelopeEnteredPeakBranch => write!(
                f,
                "cleared first regular floor-envelope stage call entered the peak branch"
            ),
            LongEnvelopeStateBins => write!(
                f,
                "long regular floor-envelope stage needs 1024 state bins and 128-or-1024 history bins"
            ),
            ShortKernelWords { got } => write!(
                f,
                "short psychoacoustic kernel must contain six words, got {got}"
            ),
            ShortAnalysisCannotApplyLongTransition { transition } => write!(
                f,
                "short analysis cannot apply a long transition code ({transition})"
            ),
            StreamFeederChannelsMismatch { want, got } => write!(
                f,
                "stream feeder push channels differ from the profile: want {want}, got {got}"
            ),
            StreamFeederSourceShort { frames } => write!(
                f,
                "stream feeder source is shorter than the LPC boundary batch ({frames})"
            ),
            StreamFeederWindowNotReady {
                want_from,
                want_to,
            } => write!(
                f,
                "stream frame window references samples not yet retained: want {want_from}..{want_to}"
            ),
            PoolUnavailable { workers, cause } => write!(
                f,
                "the analysis channel pool could not start {workers} workers: {cause}"
            ),
        }
    }
}

impl std::error::Error for AnalysisError {}

// ---------------------------------------------------------------------------
// MDCT and transient typed models
// ---------------------------------------------------------------------------

/// One static MDCT bank (Python `MdctLook`).
#[derive(Debug, Clone, PartialEq)]
pub struct MdctLook {
    pub n: i64,
    pub log2n: i64,
    /// Trig coefficients, length `n + n/4`, stored as the profile's f32 bits.
    pub trig: Vec<f32>,
    /// Bit-reversal pairs for the n/8 butterfly inputs.
    pub bitrev: Vec<i64>,
    pub scale: f32,
}

/// One transient band descriptor (Python `TransientBandConfig`).
#[derive(Debug, Clone, PartialEq)]
pub struct TransientBandConfig {
    pub offset: i64,
    pub weights: Vec<f32>,
    pub scale: f32,
}

/// The checked n=128 static detector table (Python `TransientDetectorTables`).
#[derive(Debug, Clone, PartialEq)]
pub struct TransientDetectorTables {
    pub n: i64,
    pub bias: f32,
    pub window: Vec<f32>,
    pub config: Vec<f32>,
    pub bands: Vec<TransientBandConfig>,
}

// ---------------------------------------------------------------------------
// Short psychoacoustic typed models
// ---------------------------------------------------------------------------

/// One of the two short-block psychoacoustic profiles (Python `ShortPsyProfile`).
#[derive(Debug, Clone, PartialEq)]
pub struct ShortPsyProfile {
    pub key: String,
    /// f64 values, exactly as the Python loader stores them.
    pub candidate_bias_by_mode: Vec<f64>,
    pub curve_cap: f64,
    pub group_enabled: i64,
    pub candidate_bound: i64,
    pub group_span: i64,
    pub band_limits: [i64; 3],
    pub side_gain: f64,
    pub peak_cutoff: i64,
    pub blend_weight: f64,
    pub mask_curves: Vec<Vec<f64>>,
}

/// The complete short psychoacoustic seed surface (Python `WwisePsySeedSurface`).
#[derive(Debug, Clone, PartialEq)]
pub struct WwisePsySeedSurface {
    pub n: i64,
    pub sample_rate: i64,
    pub ath_offset: f32,
    pub ath_floor: f32,
    pub seed_ceiling: f32,
    pub max_curve_db: f32,
    pub curve_offset: f32,
    pub curve_slope: f32,
    pub curve_offset_2: f32,
    pub curve_attenuation: Vec<f32>,
    pub ath: Vec<f32>,
    pub octave: Vec<i64>,
    pub first_octave: i64,
    pub shift_octave: i64,
    pub eighth_octave_lines: i64,
    pub total_octave_lines: i64,
    pub tone_curves: Vec<Vec<Vec<f32>>>,
    pub row: Vec<f32>,
    pub envelope_low: Vec<f32>,
    pub envelope_high: Vec<f32>,
    pub mask_curve: Vec<f32>,
    pub interval_table: Vec<i64>,
    pub short_limit: i64,
    pub noise_fixed_window: i64,
    pub regular_curve_bias: f32,
    pub regular_curve_cap: f32,
    pub remap_curve_offsets: Vec<i64>,
    pub remap_low_by_index: Vec<f32>,
    pub remap_high_by_index: Vec<f32>,
}

/// Materialized short psychoacoustic lookup (Python `WwisePsyLook`).
#[derive(Debug, Clone, PartialEq)]
pub struct WwisePsyLook {
    pub n: i64,
    pub sample_rate: i64,
    pub row: Vec<f64>,
    pub envelope: Vec<f32>,
    pub second_envelope: Vec<f32>,
    pub short_limit: i64,
    pub max_curve_db: f32,
    pub ath_offset: f32,
    pub ath_floor: f32,
    pub seed_ceiling: f32,
    pub regular_curve_bias: f32,
    pub regular_curve_cap: f32,
    pub interval_table: Vec<i64>,
    pub noise_fixed_window: i64,
    pub ath: Vec<f32>,
    pub octave: Vec<i64>,
    pub first_octave: i64,
    pub shift_octave: i64,
    pub total_octave_lines: i64,
    pub eighth_octave_lines: i64,
    pub curve_attenuation: Vec<f32>,
    pub tone_curves: Vec<Vec<Vec<f32>>>,
    pub mask_curves: (Vec<f32>, Vec<f32>, Vec<f32>),
}

// ---------------------------------------------------------------------------
// Long psychoacoustic typed models
// ---------------------------------------------------------------------------

/// The fixed 1024-bin Wwise psychoacoustic tables (Python `WwisePsyLongTables`).
#[derive(Debug, Clone, PartialEq)]
pub struct WwisePsyLongTables {
    pub sample_rate: i64,
    pub n: i64,
    pub profile_key: String,
    pub analysis_profile_u32: Vec<u32>,
    pub analysis_interval_u32: Vec<u32>,
    pub analysis_curves: Vec<Vec<f32>>,
    pub analysis_field_19_curve: Vec<f32>,
    pub seed_outer_u32: Vec<u32>,
    pub seed_profile_u32: Vec<u32>,
    pub seed_base_curve: Vec<f32>,
    pub seed_group_labels_u32: Vec<u32>,
    pub seed_tone_banks: Vec<Vec<Vec<f32>>>,
}

/// Long floor-seed look derived from one long table (Python `WwisePsyLongSeedLook`).
#[derive(Debug, Clone, PartialEq)]
pub struct WwisePsyLongSeedLook {
    pub n: i64,
    pub sample_rate: i64,
    pub profile_key: String,
    pub ath_offset: f32,
    pub ath_floor: f32,
    pub seed_ceiling: f32,
    pub max_curve_db: f32,
    pub base_curve: Vec<f32>,
    pub group_labels: Vec<i64>,
    pub first_octave: i64,
    pub shift_octave: i64,
    pub eighth_octave_lines: i64,
    pub total_octave_lines: i64,
    pub tone_banks: Vec<Vec<Vec<f32>>>,
}

/// Long floor-envelope look (Python `LongFloorEnvelopeLook`).
#[derive(Debug, Clone, PartialEq)]
pub struct LongFloorEnvelopeLook {
    pub n: i64,
    pub mask_curve: Vec<f32>,
    pub curve_bias: f32,
    pub curve_cap: f32,
    pub history_start: i64,
    pub side_gain: f32,
}

// ---------------------------------------------------------------------------
// Frozen transcendental tables
// ---------------------------------------------------------------------------

/// Frozen transcendental results owned by one exact profile
/// (Python `FrozenMathTables`).
///
/// Keys and values are IEEE bit patterns restored from the profile payload:
/// `coordinate_ln` maps bits(frequency) to ln(frequency); `fft_twiddles` maps
/// an FFT stage length to its (cos, sin) of -2*pi/length; each `window_halves`
/// row is the post-patch first half of vorbis_window(n) with the array's
/// symmetry supplying the tail.
#[derive(Debug, Clone, PartialEq)]
pub struct FrozenMathTables {
    /// coordinate frequency bits (f64 to_bits) -> ln(frequency).
    pub coordinate_ln: HashMap<u64, f64>,
    /// FFT stage length -> (cos(angle), sin(angle)) with angle = -2*pi/length.
    pub fft_twiddles: HashMap<i64, (f64, f64)>,
    /// Block size -> first half of the Vorbis window as f32 values.
    pub window_halves: HashMap<i64, Vec<f32>>,
}

// ---------------------------------------------------------------------------
// Assembly aggregate
// ---------------------------------------------------------------------------

/// Profile-selected DC filter configuration before scheduling and analysis.
#[derive(Debug, Clone, PartialEq)]
pub struct InputConditionerConfig {
    pub dc_filter_coefficient: f32,
}

impl InputConditionerConfig {
    pub fn new(dc_filter_coefficient: f32) -> Result<Self, AnalysisError> {
        if !(0.0..1.0).contains(&dc_filter_coefficient) || dc_filter_coefficient == 0.0 {
            return Err(AnalysisError::MalformedField {
                reason: "DC filter coefficient must be between zero and one",
            });
        }
        Ok(Self {
            dc_filter_coefficient,
        })
    }
}

/// Immutable resources injected into one analysis session
/// (Python `AnalysisProfileResources`).
#[derive(Debug, Clone, PartialEq)]
pub struct AnalysisProfileResources {
    pub mdct_looks: std::collections::BTreeMap<i64, MdctLook>,
    pub transient: TransientDetectorTables,
    pub short_profiles: Vec<ShortPsyProfile>,
    pub short_surface: WwisePsySeedSurface,
    pub short_look: WwisePsyLook,
    pub long_base: WwisePsyLongTables,
    pub long_variants: std::collections::BTreeMap<i64, WwisePsyLongTables>,
    pub long_floor_looks: std::collections::BTreeMap<i64, LongFloorEnvelopeLook>,
    pub input_conditioner: Option<InputConditionerConfig>,
    pub frozen: Option<FrozenMathTables>,
    /// Normalized quality value on the profile's breakpoint axis
    /// (`None` = historical behavior, no quality interpolation).
    pub quality_value: Option<f64>,
    /// Whether the quality fell outside the recorded control points.
    pub quality_extrapolated: bool,
}

impl AnalysisProfileResources {
    /// Validate the aggregate (Python `__post_init__`): required MDCT looks
    /// {128, 256, 2048}, two short profiles, long variants {2, 3}, and the
    /// quality-extrapolation honesty rule.
    #[allow(clippy::too_many_arguments)] // 1:1 with the Python constructor
    pub fn new(
        mdct_looks: std::collections::BTreeMap<i64, MdctLook>,
        transient: TransientDetectorTables,
        short_profiles: Vec<ShortPsyProfile>,
        short_surface: WwisePsySeedSurface,
        short_look: WwisePsyLook,
        long_base: WwisePsyLongTables,
        long_variants: std::collections::BTreeMap<i64, WwisePsyLongTables>,
        long_floor_looks: std::collections::BTreeMap<i64, LongFloorEnvelopeLook>,
        input_conditioner: Option<InputConditionerConfig>,
        frozen: Option<FrozenMathTables>,
        quality_value: Option<f64>,
        quality_extrapolated: bool,
    ) -> Result<Self, AnalysisError> {
        for required in [128, 256, 2048] {
            if !mdct_looks.contains_key(&required) {
                return Err(AnalysisError::IncompleteResources {
                    reason: "analysis resources lack required MDCT looks",
                });
            }
        }
        if short_profiles.len() != 2 || !long_variants.keys().copied().eq([2, 3]) {
            return Err(AnalysisError::IncompleteResources {
                reason: "analysis resources lack psychoacoustic variants",
            });
        }
        if quality_extrapolated && quality_value.is_none() {
            return Err(AnalysisError::QualityExtrapolationWithoutValue);
        }
        Ok(Self {
            mdct_looks,
            transient,
            short_profiles,
            short_surface,
            short_look,
            long_base,
            long_variants,
            long_floor_looks,
            input_conditioner,
            frozen,
            quality_value,
            quality_extrapolated,
        })
    }

    /// Expose the frozen transcendental tables (Python `_frozen_twiddles`).
    pub fn frozen_twiddles(&self) -> Result<&FrozenMathTables, AnalysisError> {
        self.frozen
            .as_ref()
            .ok_or(AnalysisError::IncompleteResources {
                reason: "analysis resources lack frozen math tables",
            })
    }
}

// ---------------------------------------------------------------------------
// Pure look factories (Python `make_*` functions in analysis/config.py)
// ---------------------------------------------------------------------------

/// Build the materialized short psychoacoustic look (Python
/// `make_wwise_psy_look`). `frozen_ln` maps coordinate-frequency f64 bits to
/// the frozen ln value; when `None`, host `ln` is used (non-frozen
/// development/test path only — production profiles always supply the table).
pub fn make_wwise_psy_look(
    surface: &WwisePsySeedSurface,
    row: Option<&[f64]>,
    frozen_ln: Option<&HashMap<u64, f64>>,
) -> Result<WwisePsyLook, AnalysisError> {
    let n = surface.n;
    let sample_rate = surface.sample_rate;
    if n != 128 || !CALIBRATION_SAMPLE_RATES.contains(&sample_rate) {
        return Err(AnalysisError::PsyLookGeometry { n, sample_rate });
    }
    let selected_row: Vec<f64> = match row {
        Some(values) => values.to_vec(),
        None => surface.row.iter().map(|v| *v as f64).collect(),
    };
    if selected_row.len() != surface.row.len() {
        return Err(AnalysisError::PsyLookRowLength {
            got: selected_row.len(),
            want: surface.row.len(),
        });
    }

    let coordinate = |index: i64| -> Result<(i64, f32), AnalysisError> {
        let frequency = (index as f64 + 0.5) * (sample_rate as f64) / ((2 * n) as f64);
        let ln_value: f64 = match frozen_ln {
            None => frequency.ln(),
            Some(table) => table.get(&frequency.to_bits()).copied().ok_or(
                AnalysisError::FrozenLnDomainMiss {
                    frequency_bits: frequency.to_bits(),
                },
            )?,
        };
        let mut value = f32_round(2.0 * (ln_value * 1.442695021629333 - 5.965784072875977));
        value = value.clamp(0.0, 16.0);
        let base = value as i64;
        Ok((base, f32_round(value as f64 - base as f64)))
    };

    let interpolate = |offset: usize| -> Result<Vec<f32>, AnalysisError> {
        let mut values = Vec::with_capacity(n as usize);
        for index in 0..n {
            let (_base, fraction) = coordinate(index)?;
            values.push(f32_round(
                fraction as f64 * selected_row[offset + 1]
                    + (1.0 - fraction as f64) * selected_row[offset],
            ));
        }
        Ok(values)
    };

    let mut envelope = Vec::with_capacity(n as usize);
    for index in 0..n {
        let (base, fraction) = coordinate(index)?;
        envelope.push(f32_round(
            fraction as f64 * surface.envelope_high[base as usize] as f64
                + (1.0 - fraction as f64) * surface.envelope_low[base as usize] as f64,
        ));
    }

    let interpolated = [interpolate(33)?, interpolate(50)?, interpolate(67)?];

    Ok(WwisePsyLook {
        n,
        sample_rate,
        row: selected_row,
        envelope,
        second_envelope: surface.mask_curve.clone(),
        short_limit: surface.short_limit,
        max_curve_db: surface.max_curve_db,
        ath_offset: surface.ath_offset,
        ath_floor: surface.ath_floor,
        seed_ceiling: surface.seed_ceiling,
        regular_curve_bias: surface.regular_curve_bias,
        regular_curve_cap: surface.regular_curve_cap,
        interval_table: surface.interval_table.clone(),
        noise_fixed_window: surface.noise_fixed_window,
        ath: surface.ath.clone(),
        octave: surface.octave.clone(),
        first_octave: surface.first_octave,
        shift_octave: surface.shift_octave,
        total_octave_lines: surface.total_octave_lines,
        eighth_octave_lines: surface.eighth_octave_lines,
        curve_attenuation: surface.curve_attenuation.clone(),
        tone_curves: surface.tone_curves.clone(),
        mask_curves: (
            interpolated[0].clone(),
            surface.mask_curve.clone(),
            interpolated[2].clone(),
        ),
    })
}

/// Build the long floor-seed look (Python `make_wwise_long_seed_look`).
pub fn make_wwise_long_seed_look(
    table: &WwisePsyLongTables,
) -> Result<WwisePsyLongSeedLook, AnalysisError> {
    let outer = &table.seed_outer_u32;
    let profile = &table.seed_profile_u32;
    if table.n != 1024 || !CALIBRATION_SAMPLE_RATES.contains(&table.sample_rate) {
        return Err(AnalysisError::LongSeedGeometry);
    }
    if outer[0] as i64 != table.n || outer[11] as i64 != table.sample_rate {
        return Err(AnalysisError::LongSeedGeometry);
    }
    let first_octave = signed_u32(outer[7]);
    let shift_octave = i64::from(outer[8]);
    let eighth_octave_lines = i64::from(outer[9]);
    let total_octave_lines = i64::from(outer[10]);
    if !(first_octave < 0
        && (1..=31).contains(&shift_octave)
        && eighth_octave_lines > 0
        && (1..=0x100000).contains(&total_octave_lines))
    {
        return Err(AnalysisError::LongSeedOctaveGeometry);
    }
    let labels: Vec<i64> = table
        .seed_group_labels_u32
        .iter()
        .map(|value| signed_u32(*value))
        .collect();
    if labels.len() != table.n as usize || labels.windows(2).any(|pair| pair[0] > pair[1]) {
        return Err(AnalysisError::LongSeedLabelsMalformed);
    }
    if table.seed_tone_banks.len() != TONE_BAND_COUNT as usize
        || !table
            .seed_tone_banks
            .iter()
            .all(|bank| bank.len() == TONE_LEVEL_COUNT as usize)
    {
        return Err(AnalysisError::LongSeedToneBanksMalformed);
    }
    Ok(WwisePsyLongSeedLook {
        n: table.n,
        sample_rate: table.sample_rate,
        profile_key: table.profile_key.clone(),
        ath_offset: u32_to_f32(profile[1]),
        ath_floor: u32_to_f32(profile[2]),
        seed_ceiling: u32_to_f32(profile[8]),
        max_curve_db: u32_to_f32(profile[165]),
        base_curve: table.seed_base_curve.clone(),
        group_labels: labels,
        first_octave,
        shift_octave,
        eighth_octave_lines,
        total_octave_lines,
        tone_banks: table.seed_tone_banks.clone(),
    })
}

/// Build the long floor-envelope look (Python `make_long_floor_envelope_look`).
pub fn make_long_floor_envelope_look(
    table: &WwisePsyLongTables,
) -> Result<LongFloorEnvelopeLook, AnalysisError> {
    if table.n != 1024 || table.analysis_curves.len() < 2 {
        return Err(AnalysisError::LongFloorGeometry);
    }
    let profile = &table.analysis_profile_u32;
    if profile.len() <= 167 {
        return Err(AnalysisError::LongFloorScalars {
            len: profile.len(),
            value: -1,
        });
    }
    Ok(LongFloorEnvelopeLook {
        n: table.n,
        mask_curve: table.analysis_curves[1].clone(),
        curve_bias: u32_to_f32(profile[4]),
        curve_cap: u32_to_f32(profile[27]),
        history_start: i64::from(profile[167]),
        side_gain: u32_to_f32(table.seed_outer_u32[16]),
    })
}
