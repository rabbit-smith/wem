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

/// Round an f64 to its f32-representable value (Python `_f32`).
#[inline]
pub fn f32_round(value: f64) -> f32 {
    value as f32
}

/// Python `_f32` returning f64: round to the f32-representable value and
/// promote back to f64. This is the normative float boundary in the
/// bit-exact kernel; every Python `_f32(...)` maps to one call here.
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
    SessionBlockSizeMismatch {
        got: [i64; 2],
    },
    /// "analysis frames must be contiguous: expected index {}, got {}"
    AnalysisFrameNotContiguous { expected: i64, got: i64 },
    /// "adjacent analysis frame modes differ"
    AdjacentAnalysisFrameModesDiffer,
    /// "short analysis expects a 256-sample scheduled window"
    ShortAnalysisWindowGeometry {
        want: i64,
    },
    /// "long analysis expects a 2048-sample scheduled window"
    LongAnalysisWindowGeometry {
        want: i64,
    },
    /// "analysis window samples differ from its scheduled mode"
    AnalysisWindowSamplesMismatch,
    /// "manual transient ingestion cannot be mixed with mode generation"
    ManualIngestionMixed,
    /// "mode selection requires a fresh stream selector"
    ModeSelectionNotFresh,
    /// "mode selection PCM must be equal-length and at least 4096 samples"
    ModeSelectionPcmInvalid { frames: i64 },
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
    ShortAnalyzerChannelsNonPositive {
        channels: i64,
    },
    /// "short psychoacoustic analyzer needs exactly two profiles"
    ShortAnalyzerProfiles { want: i64, got: i64 },
    /// "short psychoacoustic channel-state count differs"
    ShortAnalyzerChannelStateCount {
        want: i64,
    },
    /// "long psychoacoustic variant must be 0 or 1"
    LongVariantInvalid { variant: i64 },
    /// "long analysis needs at least one channel frame"
    LongAnalysisEmpty,
    /// "long analysis expects 2048-sample windowed frames"
    LongAnalysisFrameSize { want: i64 },
    /// "long analysis table must have 1024 output bins"
    LongAnalysisTableBins { want: i64 },
    /// "long analysis scratch count must equal channel count"
    LongAnalysisScratchCount {
        want: i64,
    },
    /// "long analysis stream channel count differs from frames"
    LongAnalysisStreamChannels {
        want: i64,
    },
    /// "long MDCT look differs from analysis geometry"
    LongMdctLookGeometry { want: i64 },
    /// "long analysis accepts either scratch or shared stream, not both"
    LongAnalysisBothScratchAndStream,
    /// "short analysis needs at least one channel frame"
    ShortAnalysisEmpty,
    /// "short analysis channel count differs from state owner"
    ShortAnalysisChannelCount {
        want: i64,
    },
    /// "short analysis expects 256-sample windowed frames"
    ShortAnalysisFrameSize { want: i64 },
    /// "short analysis group-work count differs from channels"
    ShortAnalysisGroupWorkCount {
        want: i64,
    },
    /// "short MDCT look differs from analysis geometry"
    ShortMdctLookGeometry { want: i64 },
    /// "long state bridge needs one 1024-bin raw curve per channel"
    LongStateBridgeGeometry,
    /// "long state bridge needs one 1024-bin raw curve per channel" (count)
    LongStateBridgeCount { want: i64 },
    /// "long-to-short floor reduction expects 1024 bins"
    LongToShortHistoryLength {
        want_raw: i64,
        want_state: i64,
    },
    /// "short-to-long floor expansion expects 128 bins"
    ShortToLongHistoryLength { want: i64 },
    /// "floor-envelope stage scratch length must be positive"
    FloorEnvelopeScratchNonPositive { n: i64 },
    /// "floor-envelope stage curves must have equal lengths"
    FloorEnvelopeCurveLengthMismatch {
        n: i64,
    },
    /// "floor-envelope stage source curve must have equal length"
    FloorEnvelopeSourceLengthMismatch {
        n: i64,
    },
    /// "floor-envelope stage regular port currently targets the regular branch"
    FloorEnvelopeModeUnsupported { mode: i64 },
    /// "first regular floor-envelope stage buffers must share one length"
    FirstFloorEnvelopeBufferMismatch { n: i64 },
    /// "first regular floor-envelope stage call entered peak branch"
    FirstFloorEnvelopePeakBranch,
    /// "long regular floor-envelope stage needs 1024 state bins and 128-or-1024 history bins"
    LongFloorEnvelopeGeometry {
        n: i64,
    },
    /// "fresh long regular floor-envelope stage call entered peak branch"
    LongFloorEnvelopePeakBranch,
    /// "cleared first regular floor-envelope stage call entered peak branch"
    FirstLongFloorEnvelopePeakBranch,
    /// "floor-envelope stage requires look or table"
    LongFloorEnvelopeNoLook,
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
    SeedLoopInputLengthMismatch {
        want: i64,
    },
    /// "seed position falls outside total octave lines"
    SeedPositionOutOfRange {
        pos: i64,
        total: i64,
    },
    /// "seed surface is shorter than total octave lines"
    SeedSurfaceShort {
        want: i64,
    },
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
    LongRemapLutShort {
        got: i64,
    },
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
}


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
    pub frozen: Option<FrozenMathTables>,
}

impl AnalysisProfileResources {
    /// Validate the aggregate (Python `__post_init__`): required MDCT looks
    /// {128, 256, 2048}, two short profiles, and long variants {2, 3}.
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
        frozen: Option<FrozenMathTables>,
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
        Ok(Self {
            mdct_looks,
            transient,
            short_profiles,
            short_surface,
            short_look,
            long_base,
            long_variants,
            long_floor_looks,
            frozen,
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
    if n != 128 || sample_rate != 44100 {
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
    if table.n != 1024 || table.sample_rate != 44100 {
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
        side_gain: 1.0,
    })
}
