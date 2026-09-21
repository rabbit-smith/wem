//! Single error type for all profile/resource operations.
//!
//! Each variant maps one-to-one to a Python rejection condition
//! (`ValueError` family in `wwise_wem/profiles/*`); the wording may differ,
//! the reject/accept behavior must not.

/// Error raised by any profile resource, bundle, registry, or table loader.
///
/// Variants are append-only: a shipped variant is never renumbered, renamed,
/// or repurposed, so a consumer can match on them exhaustively across
/// revisions. Variants without a producing site are the exception — they are
/// removed rather than kept as dead surface: `NoProfileForGeometry`,
/// `AmbiguousProfileGeometry`, `TemplateSetupNoInstalledProfile` and
/// `UnknownProfile` went that way, superseded by the structured-selection
/// errors `NoProfileForSelection` / `AmbiguousProfileSelection`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ProfileError {
    // ------------------------------------------------------------------
    // data directory / resource identity
    // ------------------------------------------------------------------
    /// Unsafe package-relative resource path (empty, backslash, absolute,
    /// or empty/`.`/`..` segment).
    UnsafePath { path: String },
    /// SHA-256 field is not 64 lowercase-hex digits.
    InvalidSha256 { value: String },
    /// Resource file missing or unreadable.
    MissingResource { path: String },
    /// Resource bytes do not match the manifest/index SHA-256.
    ShaMismatch {
        path: String,
        expected: String,
        actual: String,
    },
    /// Resource is not valid JSON.
    InvalidJson { path: String },
    // ------------------------------------------------------------------
    // index.json
    // ------------------------------------------------------------------
    /// Index top-level is not an object.
    IndexNotObject { path: String },
    /// Index schema unsupported (Python: "unsupported profile index schema").
    UnsupportedIndexSchema { schema: String },
    /// Index has no usable `default` entry.
    IndexDefaultMissing,
    /// Index `profiles` section is not an object.
    IndexProfilesNotObject,
    /// Selected profile absent from the index (Python: "profile ... is
    /// absent from profile index").
    ProfileNotInIndex { profile: String },
    /// Index profile entry is not an object.
    IndexProfileNotObject { profile: String },
    /// Index profile entry lacks a non-empty `manifest` path.
    IndexManifestPathMissing,
    /// Index profile entry lacks a non-empty `sha256`.
    IndexProfileShaMissing,
    // ------------------------------------------------------------------
    // manifest.json / runtime manifest
    // ------------------------------------------------------------------
    /// Manifest top-level is not an object.
    ManifestNotObject { path: String },
    /// Manifest schema unsupported (Python: "unsupported profile manifest
    /// schema").
    UnsupportedManifestSchema { schema: String },
    /// Manifest `resources` section is not an object.
    ManifestResourcesNotObject,
    /// Manifest `resources` section is empty.
    ManifestResourcesEmpty,
    /// A resource logical name is empty.
    ResourceNameEmpty,
    /// A resource entry is not an object.
    ResourceEntryNotObject { name: String },
    /// A resource entry lacks a non-empty `path`.
    ResourcePathMissing { name: String },
    /// A resource entry lacks a non-empty `sha256`.
    ResourceShaMissing { name: String },
    /// Resource absent from the profile manifest
    /// (Python: "resource {key} is absent from profile manifest").
    ResourceNotFound { key: String },
    /// key / container_metadata / block_sizes malformed (Python: "profile
    /// manifest fields are malformed: ...").
    ManifestFieldMalformed { reason: String },
    // ------------------------------------------------------------------
    // ProfileBundle construction
    // ------------------------------------------------------------------
    /// Bundle name empty.
    BundleNameEmpty,
    /// Profile geometry differs from container metadata.
    BundleGeometryMismatch,
    /// Block sizes differ from container metadata powers.
    BundleBlockSizesMismatch,
    /// Manifest lacks the `vorbis.setup` resource.
    BundleMissingVorbisSetup,
    /// key.quality_setup_identity differs from setup SHA-256.
    BundleSetupIdentityMismatch,
    /// Index-selected name differs from the manifest name.
    IndexNameMismatch { selected: String, name: String },
    // ------------------------------------------------------------------
    // ProfileKey
    // ------------------------------------------------------------------
    /// channels/sample_rate non-positive.
    ProfileKeyNonPositive,
    /// Non-default geometry with only a partial identity triple.
    /// Generation / layout / identity field is empty.
    ProfileKeyFieldEmpty { field: &'static str },
    // ------------------------------------------------------------------
    // ContainerMetadata / EncoderProfile
    // ------------------------------------------------------------------
    /// Container metadata field missing.
    ContainerFieldMissing { field: &'static str },
    /// Container metadata field is not an integer.
    ContainerFieldNotInteger { field: &'static str },
    /// Container metadata field negative.
    ContainerFieldNegative { field: &'static str },
    /// nChannels / nSamplesPerSec non-positive.
    ContainerGeometryNonPositive,
    /// Encoder profile name empty.
    ProfileNameEmpty,
    /// Profile key geometry differs from container metadata.
    ProfileGeometryMismatch,
    /// key quality/setup identity differs from setup SHA-256.
    ProfileSetupIdentityMismatch,
    /// Block sizes not two positive sizes.
    ProfileBlockSizesMalformed,
    /// Block sizes differ from container metadata.
    ProfileBlockSizesMismatch,
    // ------------------------------------------------------------------
    // registry
    // ------------------------------------------------------------------
    /// Duplicate profile key in the registry.
    RegistryDuplicateKey { key: String },
    /// Duplicate profile name in the registry.
    RegistryDuplicateName { name: String },
    /// Unknown exact profile key (Python: "unknown WEM profile key").
    UnknownProfileKey { key: String },
    /// resolve by channels without a sample rate.
    ResolveSampleRateRequired,
    /// Runtime manifest differs from the installed bundle.
    InstalledBundleMismatch { profile: String },
    // ------------------------------------------------------------------
    // structured profile selection (WwiseVersion / WwiseProfile)
    // ------------------------------------------------------------------
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
    // book tables / codebook resolution
    // ------------------------------------------------------------------
    /// Unknown book table name (Python: "missing decoded codebook table").
    UnknownBookTable { table: String },
    /// Book table row count / shape malformed.
    BookTableMalformed {
        table: String,
        expected_count: usize,
    },
    /// Book table row missing a required field.
    BookRowMissingField { table: String, field: &'static str },
    /// Book id outside the installed tables
    /// (Python: "book id is outside the installed codebook tables").
    BookIdOutOfRange { book_id: i64 },
    // ------------------------------------------------------------------
    // typed resource loaders
    // ------------------------------------------------------------------
    /// Frozen-math schema unexpected.
    FrozenSchemaUnexpected { schema: String },
    /// Frozen-math entry malformed (not a 2-item hex pair etc).
    FrozenEntryMalformed { section: &'static str },
    /// Frozen fft_twiddles length is not an integer string.
    FrozenTwiddleLengthMalformed { value: String },
    /// Frozen window half length disagrees with its block size.
    FrozenWindowHalvesMismatch { size: i64 },
    /// Frozen-math payload missing a section.
    FrozenSectionMissing,
    /// MDCT trig schema unsupported.
    MdctSchemaUnsupported { schema: String },
    /// MDCT `profiles` section malformed.
    MdctProfilesMalformed,
    /// MDCT profile entry malformed (not an object / bad key).
    MdctProfileMalformed { key: String },
    /// MDCT base64 payload malformed (bad alphabet / padding).
    MdctBase64Malformed { n: i64 },
    /// MDCT word_count / payload length mismatch.
    MdctWordCountMismatch { n: i64 },
    /// MDCT profile set is not exactly {128, 256, 512, 1024, 2048}.
    MdctProfileSetIncomplete,
    /// Transient table schema/geometry changed.
    TransientSchemaChanged,
    /// Transient window length wrong.
    TransientWindowMalformed,
    /// Transient config length wrong.
    TransientConfigMalformed,
    /// Transient band table length wrong.
    TransientBandsMalformed,
    /// Transient descriptor row malformed.
    TransientRowMalformed { band: usize },
    /// Transient top-level field missing.
    TransientFieldMissing { field: &'static str },
    /// Short psychoacoustic table schema/geometry changed.
    ShortProfilesSchemaChanged,
    /// Short psychoacoustic table must contain two profiles.
    ShortProfilesRowCount,
    /// Short profile mask curves malformed.
    ShortProfileMaskCurvesMalformed { profile: usize },
    /// Short profile band limits malformed.
    ShortProfileBandLimitsMalformed { profile: usize },
    /// Short profile field missing/malformed.
    ShortProfileFieldMalformed { profile: usize, field: &'static str },
    /// Short seed-surface schema unexpected.
    ShortSeedSchemaUnexpected { schema: String },
    /// Short seed-surface section missing.
    ShortSeedSectionMissing { section: &'static str },
    /// Short seed-surface geometry unsupported.
    ShortSeedGeometryUnsupported,
    /// Short seed-surface tone shape unexpected.
    ShortSeedToneShapeUnexpected,
    /// Short seed-surface tone RLE entry malformed.
    ShortSeedToneRleEntryMalformed,
    /// Short seed-surface tone RLE count invalid.
    ShortSeedToneRleCountInvalid,
    /// Short seed-surface tone data malformed (length / non-finite).
    ShortSeedToneDataMalformed,
    /// Short seed-surface field missing/malformed.
    ShortSeedFieldMalformed { field: &'static str },
    /// Long psychoacoustic-table schema unexpected.
    LongTablesSchemaUnexpected { schema: String },
    /// Long psychoacoustic-table geometry unexpected.
    LongTablesGeometryUnexpected,
    /// Long psychoacoustic-table profile key missing.
    LongTablesProfileKeyMissing,
    /// Long psychoacoustic-table section missing/malformed.
    LongTablesSectionMissing { section: &'static str },
    /// Long psychoacoustic-table tone bands malformed.
    LongTablesToneBandsMalformed { band: usize },
    /// Long psychoacoustic-table analysis curves malformed.
    LongTablesCurvesMalformed,
    /// Long psychoacoustic-table field missing/malformed.
    LongTablesFieldMalformed { field: &'static str },
    /// Long analysis mode must be 2 or 3.
    LongVariantModeInvalid { mode: i64 },
    /// Long analysis variant table schema/geometry unexpected.
    LongVariantSchemaUnexpected { schema: String },
    /// Long analysis table lacks a mode.
    LongVariantMissingMode { mode: i64 },
    /// Long variant profile_u32 malformed.
    LongVariantProfileMalformed,
    /// Long variant interval_u32 malformed.
    LongVariantIntervalsMalformed,
    /// Long variant curves malformed.
    LongVariantCurvesMalformed,
    /// Long variant field-19 curve malformed.
    LongVariantField19Malformed,
    // ------------------------------------------------------------------
    // quality-curves resource (optional; absent -> historical behavior)
    // ------------------------------------------------------------------
    /// Quality-curves schema unexpected.
    QualityCurvesSchemaUnexpected { schema: String },
    /// Quality-curves interpolation marker unsupported.
    QualityCurvesInterpolationUnsupported { interpolation: String },
    /// Quality-curves `breakpoints` is not an array.
    QualityCurvesBreakpointsNotArray,
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
    /// Quality-curves `curves` is not an object.
    QualityCurvesCurvesNotObject,
    /// The quality value is not a finite number.
    QualityValueNonFinite,
    /// A profile without the quality-curves resource asked for quality.
    QualityCurvesResourceMissing { profile: String },
    /// A quality curve names an unsupported override parameter.
    QualityCurveParameterUnsupported { name: String },
    /// Quality-curves `semantics` is not an object.
    QualityCurvesSemanticsNotObject,
    /// Quality-curves semantics do not cover the curve names exactly.
    QualityCurvesSemanticsIncomplete,
    // ------------------------------------------------------------------
    // transient record family (wem.transient-record-family.v1)
    // ------------------------------------------------------------------
    /// Record-family geometry (n/sample_rate) changed.
    TransientRecordFamilyGeometryChanged,
    /// Record-family must carry exactly six records.
    TransientRecordFamilyRecordCount,
    /// One record of the family is malformed.
    TransientRecordFamilyRecord { index: usize },
    /// A record-family f64 table is malformed.
    TransientRecordFamilyCurve { field: &'static str },
    /// Record-family band/stride word table malformed.
    TransientRecordFamilyWords { field: &'static str },
    /// Record-family window construction constants malformed.
    TransientRecordFamilyWindowConstants,
    /// Record-family default record index out of domain.
    TransientRecordFamilyDefaultIndex,
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
            UnsafePath { path } => write!(f, "unsafe package resource path: {path:?}"),
            InvalidSha256 { value } => write!(f, "resource SHA-256 must contain 64 hex digits: {value:?}"),
            MissingResource { path } => write!(f, "missing package resource {path}"),
            ShaMismatch { path, expected, actual } => {
                write!(f, "resource {path} SHA-256 differs: expected {expected}, actual {actual}")
            }
            InvalidJson { path } => write!(f, "resource {path} is not valid JSON"),
            IndexNotObject { path } => write!(f, "profile bundle index must be an object: {path}"),
            UnsupportedIndexSchema { schema } => write!(f, "unsupported profile index schema {schema:?}"),
            IndexDefaultMissing => write!(f, "profile bundle index.default must be non-empty text"),
            IndexProfilesNotObject => write!(f, "profile bundle index.profiles must be an object"),
            ProfileNotInIndex { profile } => write!(f, "profile {profile:?} is absent from profile index"),
            IndexProfileNotObject { profile } => write!(f, "profile bundle index profile {profile} must be an object"),
            IndexManifestPathMissing => write!(f, "profile bundle index profile manifest must be non-empty text"),
            IndexProfileShaMissing => write!(f, "profile bundle index profile sha256 must be non-empty text"),
            ManifestNotObject { path } => write!(f, "profile bundle manifest must be an object: {path}"),
            UnsupportedManifestSchema { schema } => write!(f, "unsupported profile manifest schema {schema:?}"),
            ManifestResourcesNotObject => write!(f, "profile bundle manifest resources must be an object"),
            ManifestResourcesEmpty => write!(f, "profile manifest must contain at least one resource"),
            ResourceNameEmpty => write!(f, "profile bundle resource logical name must be non-empty text"),
            ResourceEntryNotObject { name } => write!(f, "profile bundle resource {name} must be an object"),
            ResourcePathMissing { name } => write!(f, "profile bundle resource {name}.path must be non-empty text"),
            ResourceShaMissing { name } => write!(f, "profile bundle resource {name}.sha256 must be non-empty text"),
            ResourceNotFound { key } => write!(f, "resource {key} is absent from profile manifest"),
            ManifestFieldMalformed { reason } => write!(f, "profile manifest fields are malformed: {reason}"),
            BundleNameEmpty => write!(f, "profile bundle name must not be empty"),
            BundleGeometryMismatch => write!(f, "profile bundle geometry differs from container metadata"),
            BundleBlockSizesMismatch => write!(f, "profile bundle block sizes differ from container metadata"),
            BundleMissingVorbisSetup => write!(f, "profile manifest is missing vorbis.setup"),
            BundleSetupIdentityMismatch => write!(f, "profile bundle setup identity differs from setup SHA-256"),
            IndexNameMismatch { selected, name } => write!(f, "profile index name {selected:?} differs from profile manifest {name:?}"),
            ProfileKeyNonPositive => write!(f, "profile channels and sample rate must be positive"),
            ProfileKeyFieldEmpty { field } => write!(f, "profile {field} must not be empty"),
            ContainerFieldMissing { field } => write!(f, "container metadata field missing: {field}"),
            ContainerFieldNotInteger { field } => write!(f, "container metadata field must be an integer: {field}"),
            ContainerFieldNegative { field } => write!(f, "container metadata field must be non-negative: {field}"),
            ContainerGeometryNonPositive => write!(f, "container geometry must be positive"),
            ProfileNameEmpty => write!(f, "profile name must not be empty"),
            ProfileGeometryMismatch => write!(f, "profile key geometry differs from container metadata"),
            ProfileSetupIdentityMismatch => write!(f, "profile key quality/setup identity differs from setup SHA-256"),
            ProfileBlockSizesMalformed => write!(f, "profile block sizes must contain two positive sizes"),
            ProfileBlockSizesMismatch => write!(f, "profile block sizes differ from container metadata"),
            RegistryDuplicateKey { key } => write!(f, "duplicate profile key: {key}"),
            RegistryDuplicateName { name } => write!(f, "duplicate profile name: {name}"),
            UnknownProfileKey { key } => write!(f, "unknown WEM profile key {key}"),
            ResolveSampleRateRequired => write!(f, "sample_rate is required when resolving by channels"),
            InstalledBundleMismatch { profile } => write!(f, "profile {profile} differs from installed profile bundle"),
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
            BookTableMalformed { table, expected_count } => {
                write!(f, "decoded codebook table {table:?} must contain {expected_count} object rows")
            }
            BookRowMissingField { table, field } => write!(f, "decoded codebook table {table:?} row missing field {field}"),
            BookIdOutOfRange { book_id } => write!(f, "book id is outside the installed codebook tables: {book_id}"),
            FrozenSchemaUnexpected { schema } => write!(f, "unexpected frozen-math schema: {schema:?}"),
            FrozenEntryMalformed { section } => write!(f, "frozen-math {section} entry is malformed"),
            FrozenTwiddleLengthMalformed { value } => write!(f, "frozen fft_twiddles length is not an integer: {value:?}"),
            FrozenWindowHalvesMismatch { size } => write!(f, "frozen window halves disagree with their block size: {size}"),
            FrozenSectionMissing => write!(f, "frozen-math payload is missing a section"),
            MdctSchemaUnsupported { schema } => write!(f, "unsupported Wwise MDCT trig-table schema: {schema:?}"),
            MdctProfilesMalformed => write!(f, "Wwise MDCT trig-table profiles are malformed"),
            MdctProfileMalformed { key } => write!(f, "Wwise MDCT trig profile is malformed: {key}"),
            MdctBase64Malformed { n } => write!(f, "malformed Wwise MDCT trig table base64 for n={n}"),
            MdctWordCountMismatch { n } => write!(f, "malformed Wwise MDCT trig table for n={n}"),
            MdctProfileSetIncomplete => write!(f, "Wwise MDCT trig table profile set is incomplete"),
            TransientSchemaChanged => write!(f, "transient table schema changed"),
            TransientWindowMalformed => write!(f, "transient window must contain 128 float words"),
            TransientConfigMalformed => write!(f, "transient config must contain 26 float words"),
            TransientBandsMalformed => write!(f, "transient descriptor table must contain twelve bands"),
            TransientRowMalformed { band } => write!(f, "transient descriptor row {band} is malformed"),
            TransientFieldMissing { field } => write!(f, "transient table field missing: {field}"),
            ShortProfilesSchemaChanged => write!(f, "short psychoacoustic table schema/geometry changed"),
            ShortProfilesRowCount => write!(f, "short psychoacoustic table must contain two profiles"),
            ShortProfileMaskCurvesMalformed { profile } => write!(f, "short psychoacoustic profile {profile} has malformed mask curves"),
            ShortProfileBandLimitsMalformed { profile } => write!(f, "short psychoacoustic profile {profile} has malformed band limits"),
            ShortProfileFieldMalformed { profile, field } => write!(f, "short psychoacoustic profile {profile} field malformed: {field}"),
            ShortSeedSchemaUnexpected { schema } => write!(f, "unexpected short seed-surface schema: {schema:?}"),
            ShortSeedSectionMissing { section } => write!(f, "short seed surface is missing a required section: {section}"),
            ShortSeedGeometryUnsupported => write!(f, "short seed surface has unsupported geometry"),
            ShortSeedToneShapeUnexpected => write!(f, "short seed surface has an unexpected tone shape"),
            ShortSeedToneRleEntryMalformed => write!(f, "short seed surface has malformed tone RLE"),
            ShortSeedToneRleCountInvalid => write!(f, "short seed surface has invalid tone RLE count"),
            ShortSeedToneDataMalformed => write!(f, "short seed surface has malformed tone data"),
            ShortSeedFieldMalformed { field } => write!(f, "short seed surface field malformed: {field}"),
            LongTablesSchemaUnexpected { schema } => write!(f, "unexpected long psychoacoustic-table schema: {schema:?}"),
            LongTablesGeometryUnexpected => write!(f, "long psychoacoustic table has unexpected geometry"),
            LongTablesProfileKeyMissing => write!(f, "long psychoacoustic table has no profile key"),
            LongTablesSectionMissing { section } => write!(f, "long psychoacoustic table lacks section: {section}"),
            LongTablesToneBandsMalformed { band } => write!(f, "long psychoacoustic table tone bands malformed: {band}"),
            LongTablesCurvesMalformed => write!(f, "long psychoacoustic table analysis.curves must contain three rows"),
            LongTablesFieldMalformed { field } => write!(f, "long psychoacoustic table field malformed: {field}"),
            LongVariantModeInvalid { mode } => write!(f, "long analysis mode must be 2 or 3: {mode}"),
            LongVariantSchemaUnexpected { schema } => write!(f, "unexpected long analysis variant table: {schema:?}"),
            LongVariantMissingMode { mode } => write!(f, "long analysis table lacks mode {mode}"),
            LongVariantProfileMalformed => write!(f, "malformed long variant profile"),
            LongVariantIntervalsMalformed => write!(f, "malformed long variant intervals"),
            LongVariantCurvesMalformed => write!(f, "malformed long variant curves"),
            LongVariantField19Malformed => write!(f, "malformed long variant field-19 curve"),
            QualityCurvesSchemaUnexpected { schema } => write!(f, "unexpected quality-curves schema: {schema:?}"),
            QualityCurvesInterpolationUnsupported { interpolation } => write!(f, "unsupported quality-curves interpolation: {interpolation:?}"),
            QualityCurvesBreakpointsNotArray => write!(f, "quality-curves breakpoints must be an array"),
            QualityCurvesTooFewBreakpoints => write!(f, "quality-curves needs at least two breakpoints"),
            QualityCurvesBreakpointsNotIncreasing => write!(f, "quality-curves breakpoints must be strictly increasing"),
            QualityCurvesCurveLengthMismatch { name, want, got } => write!(f, "quality curve {name:?} must contain {want} values, found {got}"),
            QualityCurvesValueNonFinite { name } => write!(f, "quality curve {name:?} must be finite (NaN/inf rejected)"),
            QualityCurvesEmptyCurves => write!(f, "quality-curves must define at least one curve"),
            QualityCurvesCurvesNotObject => write!(f, "quality-curves must define a non-empty curves object"),
            QualityValueNonFinite => write!(f, "quality must be a finite number"),
            QualityCurvesResourceMissing { profile } => write!(f, "profile {profile:?} has no quality-curves resource; quality interpolation is unavailable for this profile"),
            QualityCurveParameterUnsupported { name } => write!(f, "quality curve {name:?} names an unsupported parameter"),
            QualityCurvesSemanticsNotObject => write!(f, "quality-curves must define a semantics object"),
            QualityCurvesSemanticsIncomplete => write!(f, "quality-curves semantics must cover every curve name exactly"),
            TransientRecordFamilyGeometryChanged => write!(f, "transient record-family geometry changed"),
            TransientRecordFamilyRecordCount => write!(f, "transient record-family must carry six records"),
            TransientRecordFamilyRecord { index } => write!(f, "transient record {index} is malformed"),
            TransientRecordFamilyCurve { field } => write!(f, "transient record-family {field} is malformed"),
            TransientRecordFamilyWords { field } => write!(f, "transient record-family {field} is malformed"),
            TransientRecordFamilyWindowConstants => write!(f, "transient record-family window constants are malformed"),
            TransientRecordFamilyDefaultIndex => write!(f, "transient record-family default record index is out of domain"),
            Analysis(e) => write!(f, "analysis error: {e:?}"),
            Codebook(e) => write!(f, "codebook error: {e:?}"),
            Bit(e) => write!(f, "bitstream error: {e}"),
        }
    }
}

impl std::error::Error for ProfileError {}
