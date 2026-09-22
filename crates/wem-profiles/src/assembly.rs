//! Build typed runtime aggregates from a profile's logical manifest
//! (Python: `profiles/assembly.py`).

use std::collections::BTreeMap;

use wem_analysis::config::{
    make_long_floor_envelope_look, make_wwise_psy_look, AnalysisProfileResources,
    LongFloorEnvelopeLook, WwisePsySeedSurface,
};
use wem_vorbis::codebook::Codebook;
use wem_vorbis::setup::{parse_setup, SetupInfo};

use crate::carrier::CompiledProfile;
use crate::codebooks::load_setup_codebooks;
use crate::error::ProfileError;
use crate::quality::{normalize_quality_factor, QualityCurves, QUALITY_SEMANTIC_SHORT_PREFIX};

/// Complete immutable codec inputs assembled from one profile manifest
/// (Python `EncoderProfileResources`).
#[derive(Debug, Clone)]
pub struct EncoderProfileResources {
    pub analysis: AnalysisProfileResources,
    pub setup_packet: Vec<u8>,
    pub setup: SetupInfo,
    pub codebooks: Vec<Codebook>,
}

/// Scalar psychoacoustic fields that the quality factor varies along the
/// profile's quality-curves table
/// (Python `_SHORT_QUALITY_FIELDS`; the override points are the same-named
/// set on the short psychoacoustic seed surface).
///
/// Long psy uses packed u32 coefficients and its mask-curve blend factors
/// are not yet pinned to these names, so it is deliberately excluded
/// (better to omit than to guess) — both implementations agree on this
/// surface until corpus calibration says otherwise.
const SHORT_QUALITY_FIELDS: &[&str] = &[
    "ath_offset",
    "ath_floor",
    "seed_ceiling",
    "max_curve_db",
    "regular_curve_bias",
    "regular_curve_cap",
    "curve_offset",
    "curve_slope",
    "curve_offset_2",
];

/// The quality state resolved for one assembly: the curves resource, its
/// evaluated control-point overrides and whether the quality extrapolated
/// past the breakpoint domain. (The raw quality value is tracked by the
/// caller, matching the Python reference which stores the un-normalized
/// factor in `AnalysisProfileResources.quality_value`.)
#[derive(Debug, Clone, PartialEq)]
struct ResolvedQuality {
    curves: Option<QualityCurves>,
    overrides: Option<BTreeMap<String, f64>>,
    extrapolated: bool,
}

impl ResolvedQuality {
    fn none() -> Self {
        Self {
            curves: None,
            overrides: None,
            extrapolated: false,
        }
    }
}

/// Resolve and evaluate the profile's quality curves for one quality value
/// (Python `_resolve_quality_curves`).
///
/// `None` keeps the historical behavior exactly: no resource lookup, no
/// overrides. A quality value requires the profile to carry the optional
/// resource; asking for it from a profile without curves is a
/// configuration error, not a silent fallback. The quality is normalized
/// onto the breakpoint axis before evaluation (spec profile-select entry).
fn resolve_quality_curves(
    source: &CompiledProfile,
    quality: Option<f64>,
) -> Result<ResolvedQuality, ProfileError> {
    let Some(quality) = quality else {
        return Ok(ResolvedQuality::none());
    };
    let Some(curves) = source.quality_curves()? else {
        return Err(ProfileError::quality(format!(
            "profile {:?} has no quality-curves resource; quality interpolation is unavailable for this profile",
            source.label(),
        )));
    };
    let normalized = normalize_quality_factor(quality);
    let (values, extrapolated) = curves.evaluate_result(normalized)?;
    Ok(ResolvedQuality {
        curves: Some(curves),
        overrides: Some(values),
        extrapolated,
    })
}

/// Return the short surface with recognized quality overrides applied
/// (Python `_apply_short_quality_overrides`).
///
/// v2 routing: each evaluated curve is looked up in the resource's
/// `semantics` map; a `short.<field>` semantic writes the interpolated
/// value onto the same-named short surface field, while the other semantic
/// forms (`no-op`, `transient.record-index-axis`) name mechanisms that
/// consume their curve elsewhere (the record family owns its index table)
/// or do not consume it at runtime. Every override write goes through the
/// f32 boundary: the short psychoacoustic scalars are stored as float32, so
/// the interpolated f64 value is rounded exactly as the reference casts it.
/// Unrecognized parameter names are rejected so a malformed curves file can
/// never silently take effect.
fn apply_short_quality_overrides(
    surface: WwisePsySeedSurface,
    values: &BTreeMap<String, f64>,
    semantics: &std::collections::BTreeMap<String, String>,
) -> Result<WwisePsySeedSurface, ProfileError> {
    let mut overrides: BTreeMap<&str, f32> = BTreeMap::new();
    for (name, value) in values {
        let Some(semantic) = semantics.get(name).map(String::as_str) else {
            continue;
        };
        let Some(field) = semantic.strip_prefix(QUALITY_SEMANTIC_SHORT_PREFIX) else {
            continue;
        };
        if !SHORT_QUALITY_FIELDS.contains(&field) {
            return Err(ProfileError::quality(format!(
                "quality curve {name:?} names an unsupported parameter"
            )));
        }
        // f32 boundary: the reference casts the interpolated value to
        // float32 when writing the short surface field.
        overrides.insert(field, *value as f32);
    }
    if overrides.is_empty() {
        return Ok(surface);
    }
    Ok(WwisePsySeedSurface {
        ath_offset: overrides
            .get("ath_offset")
            .copied()
            .unwrap_or(surface.ath_offset),
        ath_floor: overrides
            .get("ath_floor")
            .copied()
            .unwrap_or(surface.ath_floor),
        seed_ceiling: overrides
            .get("seed_ceiling")
            .copied()
            .unwrap_or(surface.seed_ceiling),
        max_curve_db: overrides
            .get("max_curve_db")
            .copied()
            .unwrap_or(surface.max_curve_db),
        curve_offset: overrides
            .get("curve_offset")
            .copied()
            .unwrap_or(surface.curve_offset),
        curve_slope: overrides
            .get("curve_slope")
            .copied()
            .unwrap_or(surface.curve_slope),
        curve_offset_2: overrides
            .get("curve_offset_2")
            .copied()
            .unwrap_or(surface.curve_offset_2),
        regular_curve_bias: overrides
            .get("regular_curve_bias")
            .copied()
            .unwrap_or(surface.regular_curve_bias),
        regular_curve_cap: overrides
            .get("regular_curve_cap")
            .copied()
            .unwrap_or(surface.regular_curve_cap),
        ..surface
    })
}

/// Resolve every analysis input from one profile source
/// (Python `assemble_analysis_resources`).
///
/// `quality` is the optional quality factor (0-10 Wwise convention).
/// `None` reproduces the historical assembly byte for byte.
pub fn assemble_analysis_resources(
    source: &CompiledProfile,
    quality: Option<f64>,
) -> Result<AnalysisProfileResources, ProfileError> {
    let short_surface = source.short_seed()?;
    let long_base = source.long_base()?;
    let mut long_variants = BTreeMap::new();
    for mode in [2, 3] {
        long_variants.insert(mode, source.long_variant(mode)?);
    }

    // The frozen tables are optional; when present they are injected into the
    // short look.
    let frozen = source.frozen_tables()?;

    // Quality interpolation is optional in the same sense: absent curves or
    // absent quality -> the historical path is taken untouched.
    let resolved_quality = resolve_quality_curves(source, quality)?;
    let quality_values = resolved_quality.overrides;

    let mut short_surface = short_surface;
    if let Some(values) = &quality_values {
        let semantics = match &resolved_quality.curves {
            Some(curves) => curves.semantics(),
            None => &std::collections::BTreeMap::new(),
        };
        short_surface = apply_short_quality_overrides(short_surface, values, semantics)?;
    }

    let mdct_looks = source.mdct_looks()?;
    let transient = source.transient_tables(quality)?;
    let short_profiles = source.short_profiles()?;
    let input_conditioner = source.input_conditioner()?;

    let short_look = make_wwise_psy_look(
        &short_surface,
        None,
        frozen.as_ref().map(|t| &t.coordinate_ln),
    )
    .map_err(ProfileError::Analysis)?;

    let mut long_floor_looks = BTreeMap::new();
    for (mode, table) in &long_variants {
        let look: LongFloorEnvelopeLook =
            make_long_floor_envelope_look(table).map_err(ProfileError::Analysis)?;
        long_floor_looks.insert(*mode, look);
    }

    AnalysisProfileResources::new(
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
        quality, // raw quality factor, as in the Python reference
        resolved_quality.extrapolated,
    )
    .map_err(ProfileError::Analysis)
}

/// Resolve every analysis and Vorbis input from one profile identity
/// (Python `assemble_encoder_profile_resources`).
///
/// `quality` is forwarded to the analysis-resource assembly; `None` keeps
/// the historical behavior exactly.
pub fn assemble_encoder_profile_resources(
    source: &CompiledProfile,
    setup_packet: Option<&[u8]>,
    quality: Option<f64>,
) -> Result<EncoderProfileResources, ProfileError> {
    // Argument evaluation order mirrors the Python implementation.
    let packet = match setup_packet {
        Some(bytes) => bytes.to_vec(),
        None => source.setup_packet()?,
    };
    let setup = parse_setup(&packet, source.key().channels()).map_err(ProfileError::Bit)?;

    // A profile carries only the residue tables its setup references: t219
    // (6ch) and/or t282 (2ch/48k), rather than a fixed t97+t219 pair.
    let tables = source.book_tables()?;

    let analysis = assemble_analysis_resources(source, quality)?;
    let codebooks = load_setup_codebooks(&setup.book_ids, &tables)?;

    Ok(EncoderProfileResources {
        analysis,
        setup_packet: packet,
        setup,
        codebooks,
    })
}
