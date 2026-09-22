//! Single error type for all profile identity, selection and table resolution.
//!
//! Every variant has exactly one producing site in this crate. A variant that
//! loses its producer is removed rather than kept as dead surface: the
//! serialized transport that produced the resource-intake variants (index,
//! manifest, digest, path and document-shape failures) is gone, and so are
//! they.

/// Error raised by profile identity, selection, registry or table resolution.
///
/// This enum carries no `#[non_exhaustive]`, on purpose: a caller may match it
/// exhaustively, so adding a variant is a deliberate breaking change — the
/// compiler must tell every caller that a new failure mode exists. Removing a
/// variant that no longer has a producing site follows the same rule and is
/// equally deliberate (docs/reference/standards.md, Errors).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ProfileError {
    // ------------------------------------------------------------------
    // profile identity (ProfileKey / EncoderProfile)
    // ------------------------------------------------------------------
    /// channels/sample_rate non-positive.
    ProfileKeyNonPositive,
    /// Non-default geometry with only a partial identity triple.
    /// Generation / layout field is empty.
    ProfileKeyFieldEmpty { field: &'static str },
    /// Profile key geometry differs from container metadata.
    ProfileGeometryMismatch,
    /// Block sizes not two positive sizes.
    ProfileBlockSizesMalformed,
    /// Block sizes differ from container metadata.
    ProfileBlockSizesMismatch,
    /// A profile's setup availability disagrees with whether it carries a
    /// setup packet.
    BundleMissingVorbisSetup,
    // ------------------------------------------------------------------
    // registry and structured selection (WwiseVersion / WwiseProfile)
    // ------------------------------------------------------------------
    /// Duplicate profile key in the registry.
    RegistryDuplicateKey { key: String },
    /// Unknown exact profile key.
    UnknownProfileKey { key: String },
    /// A cross-language `WemVersion` code outside this revision's table.
    UnknownWwiseVersion { code: u32 },
    /// A profile-key `generation` string outside this revision's table.
    UnsupportedWwiseGeneration { generation: String },
    /// No installed profile satisfies the structured selection.
    NoProfileForSelection {
        version: String,
        channels: i64,
        sample_rate: i64,
        installed: String,
    },
    /// More than one installed profile satisfies the structured selection.
    AmbiguousProfileSelection {
        version: String,
        channels: i64,
        sample_rate: i64,
        names: String,
    },
    /// A selection's geometry is not positive.
    SelectionGeometryNonPositive,
    // ------------------------------------------------------------------
    // book tables and long analysis modes
    // ------------------------------------------------------------------
    /// Unknown book table name.
    UnknownBookTable { table: String },
    /// Book id outside the installed tables.
    BookIdOutOfRange { book_id: i64 },
    /// Long analysis mode must be 2 or 3.
    LongVariantModeInvalid { mode: i64 },
    // ------------------------------------------------------------------
    // quality-curves table (optional; absent -> historical behavior)
    // ------------------------------------------------------------------
    /// Quality-curves schema unexpected.
    QualityCurvesSchemaUnexpected { schema: String },
    /// Quality-curves has fewer than two breakpoints.
    QualityCurvesTooFewBreakpoints,
    /// Quality-curves breakpoints are not strictly increasing.
    QualityCurvesBreakpointsNotIncreasing,
    /// A quality curve's length differs from the breakpoint count.
    QualityCurvesCurveLengthMismatch {
        name: String,
        want: usize,
        got: usize,
    },
    /// A quality curve carries a non-finite value.
    QualityCurvesValueNonFinite { name: String },
    /// Quality-curves carries no curves.
    QualityCurvesEmptyCurves,
    /// The quality value is not a finite number.
    QualityValueNonFinite,
    /// A profile without the quality-curves resource asked for quality.
    QualityCurvesResourceMissing { profile: String },
    /// A quality curve names an unsupported override parameter.
    QualityCurveParameterUnsupported { name: String },
    /// Quality-curves semantics do not cover the curve names exactly.
    QualityCurvesSemanticsIncomplete,
    // ------------------------------------------------------------------
    // downstream algorithm errors (forwarded)
    // ------------------------------------------------------------------
    /// wem-analysis structural validation failed.
    Analysis(wem_analysis::config::AnalysisError),
    /// wem-vorbis codebook construction/parse failed.
    Codebook(wem_vorbis::codebook::CodebookError),
    /// wem-vorbis bitstream access failed during setup parsing.
    Bit(wem_vorbis::bitio::BitError),
}

impl std::fmt::Display for ProfileError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        // One line per variant; keep stable for logs and tests.
        use ProfileError::*;
        match self {
            ProfileKeyNonPositive => write!(f, "profile channels and sample rate must be positive"),
            ProfileKeyFieldEmpty { field } => write!(f, "profile {field} must not be empty"),
            ProfileGeometryMismatch => write!(f, "profile key geometry differs from container metadata"),
            ProfileBlockSizesMalformed => write!(f, "profile block sizes must contain two positive sizes"),
            ProfileBlockSizesMismatch => write!(f, "profile block sizes differ from container metadata"),
            BundleMissingVorbisSetup => write!(f, "profile manifest is missing vorbis.setup"),
            RegistryDuplicateKey { key } => write!(f, "duplicate profile key: {key}"),
            UnknownProfileKey { key } => write!(f, "unknown WEM profile key {key}"),
            UnknownWwiseVersion { code } => write!(f, "unknown Wwise version code {code}"),
            UnsupportedWwiseGeneration { generation } => write!(f, "unsupported Wwise generation {generation:?}"),
            NoProfileForSelection { version, channels, sample_rate, installed } => {
                write!(f, "no installed Wwise {version} profile for {channels}ch/{sample_rate}Hz; installed: {installed}")
            }
            AmbiguousProfileSelection { version, channels, sample_rate, names } => {
                write!(f, "Wwise {version} profile selection {channels}ch/{sample_rate}Hz is ambiguous: {names}")
            }
            SelectionGeometryNonPositive => write!(f, "selected channels and sample rate must be positive"),
            UnknownBookTable { table } => write!(f, "missing decoded codebook table {table:?}"),
            BookIdOutOfRange { book_id } => write!(f, "book id is outside the installed codebook tables: {book_id}"),
            LongVariantModeInvalid { mode } => write!(f, "long analysis mode must be 2 or 3: {mode}"),
            QualityCurvesSchemaUnexpected { schema } => write!(f, "unexpected quality-curves schema: {schema:?}"),
            QualityCurvesTooFewBreakpoints => write!(f, "quality-curves needs at least two breakpoints"),
            QualityCurvesBreakpointsNotIncreasing => write!(f, "quality-curves breakpoints must be strictly increasing"),
            QualityCurvesCurveLengthMismatch { name, want, got } => write!(f, "quality curve {name:?} must contain {want} values, found {got}"),
            QualityCurvesValueNonFinite { name } => write!(f, "quality curve {name:?} must be finite (NaN/inf rejected)"),
            QualityCurvesEmptyCurves => write!(f, "quality-curves must define at least one curve"),
            QualityValueNonFinite => write!(f, "quality must be a finite number"),
            QualityCurvesResourceMissing { profile } => write!(f, "profile {profile:?} has no quality-curves resource; quality interpolation is unavailable for this profile"),
            QualityCurveParameterUnsupported { name } => write!(f, "quality curve {name:?} names an unsupported parameter"),
            QualityCurvesSemanticsIncomplete => write!(f, "quality-curves semantics must cover every curve name exactly"),
            Analysis(e) => write!(f, "analysis error: {e:?}"),
            Codebook(e) => write!(f, "codebook error: {e:?}"),
            Bit(e) => write!(f, "bitstream error: {e}"),
        }
    }
}

impl std::error::Error for ProfileError {
    /// The wrapped analysis/codebook/bitstream failure; a data error with an
    /// inline message has no nested cause of its own.
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            ProfileError::Analysis(e) => Some(e),
            ProfileError::Codebook(e) => Some(e),
            ProfileError::Bit(e) => Some(e),
            _ => None,
        }
    }
}
