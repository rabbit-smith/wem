//! Load and evaluate the optional per-profile quality-interpolation curves
//! (Python: `profiles/quality.py`).
//!
//! Wwise quality is not a distinct profile: the same geometry and setup share
//! one configuration, and the psychoacoustic parameters vary as linear
//! interpolants along a fixed breakpoint table. This module owns that
//! optional resource (`analysis/quality-curves.json`, schema
//! `wem.quality-curves.v2`), its completeness validation, the immutable
//! value object, and the single interpolation kernel.
//!
//! Schema v2 adds the per-curve `semantics` map: each descriptor curve
//! (`descNN.<field>`, recorded exactly as captured in the paired build) is
//! mapped onto the mechanism it drives. The recognized semantic forms are
//! `short.<field>` (a short psychoacoustic surface override), `no-op`
//! (recorded control points without a runtime consumer), and
//! `transient.record-index-axis` (the curve belongs to the temporal
//! transient record-index mechanism, owned by the record family). The
//! override resolution in the assembly layer keys off these semantics, so
//! the curve names stay exactly as recorded in the paired build.
//!
//! # Authoritative formula (spec-aligned)
//!
//! The kernel follows the converted-encoder behavior pinned by
//! `corpus/extracted/quality-formula-spec.md`:
//!
//! * normalization (profile-selection entry):
//!   `qnorm = quality / 10.0 + 1e-7`, clamped to `0.9998999834060669`
//!   (the float32 constant promoted to double) whenever it reaches 1.0;
//! * fractional breakpoint index: within the domain,
//!   `frac = i + (q - bp[i]) / (bp[i+1] - bp[i])`; at or past the last
//!   breakpoint, `frac = N - 0.001` (so the `(N-1, N)` segment carries the
//!   value and the stored control point `N` is never read directly);
//! * interpolation: each curve is read as
//!   `(1 - f) * P[i] + f * P[i+1]` with `f = frac - i`, evaluated in
//!   float64 exactly as the two-step (index write, then subtract) form;
//! * honesty: quality strictly below the first or strictly above the last
//!   control point is reported as extrapolated.

use std::collections::BTreeMap;

use crate::error::ProfileError;
use crate::resources::ResourceRef;

/// Quality-curves resource schema (Python `QUALITY_CURVES_SCHEMA`).
pub const QUALITY_CURVES_SCHEMA: &str = "wem.quality-curves.v2";
/// Quality-curves interpolation marker (Python `QUALITY_CURVES_INTERPOLATION`).
pub const QUALITY_CURVES_INTERPOLATION: &str = "linear-frac";
/// Manifest logical name for the optional quality-curves resource
/// (Python `QUALITY_CURVES_RESOURCE`).
pub const QUALITY_CURVES_RESOURCE: &str = "analysis.quality-curves";
/// Per-curve semantic form: no runtime consumer (Python `QUALITY_SEMANTIC_NO_OP`).
pub const QUALITY_SEMANTIC_NO_OP: &str = "no-op";
/// Per-curve semantic form: the transient record-index axis, owned by the
/// record family (Python `QUALITY_SEMANTIC_TRANSIENT_RECORD_INDEX_AXIS`).
pub const QUALITY_SEMANTIC_TRANSIENT_RECORD_INDEX_AXIS: &str = "transient.record-index-axis";
/// Prefix of the short-psy surface override semantic form
/// (`short.<field>`, Python `QUALITY_SEMANTIC_SHORT_PREFIX`).
pub const QUALITY_SEMANTIC_SHORT_PREFIX: &str = "short.";

/// Spec normalization addend (float64; Python `QUALITY_NORMALIZE_ADDEND`).
pub const QUALITY_NORMALIZE_ADDEND: f64 = 1e-7;
/// Spec normalization clamp: the float32 constant promoted to float64
/// (Python `QUALITY_NORMALIZE_CLAMP`).
pub const QUALITY_NORMALIZE_CLAMP: f64 = 0.9998999834060669;

/// Immutable quality-interpolation tables for one exact profile
/// (Python `QualityCurves`).
///
/// `breakpoints` is a strictly increasing control-point domain (the
/// normalized quality axis). `curves` maps a stable parameter name to the
/// value taken at each breakpoint. `semantics` (v2) maps every curve name
/// onto the semantic of the mechanism it drives; it must cover the curve
/// names exactly, so a malformed curves file can never take partial effect.
/// Both have equal length and are
/// validated at construction.
#[derive(Debug, Clone, PartialEq)]
pub struct QualityCurves {
    schema: String,
    breakpoints: Vec<f64>,
    curves: BTreeMap<String, Vec<f64>>,
    semantics: BTreeMap<String, String>,
}

impl QualityCurves {
    /// Validate and construct (Python `QualityCurves.__post_init__`).
    pub fn new(
        schema: String,
        breakpoints: Vec<f64>,
        curves: BTreeMap<String, Vec<f64>>,
        semantics: BTreeMap<String, String>,
    ) -> Result<Self, ProfileError> {
        if schema != QUALITY_CURVES_SCHEMA {
            return Err(ProfileError::QualityCurvesSchemaUnexpected { schema });
        }
        if breakpoints.len() < 2 {
            return Err(ProfileError::QualityCurvesTooFewBreakpoints);
        }
        if breakpoints.windows(2).any(|pair| pair[0] >= pair[1]) {
            return Err(ProfileError::QualityCurvesBreakpointsNotIncreasing);
        }
        if curves.is_empty() {
            return Err(ProfileError::QualityCurvesEmptyCurves);
        }
        for (name, values) in &curves {
            if values.len() != breakpoints.len() {
                return Err(ProfileError::QualityCurvesCurveLengthMismatch {
                    name: name.clone(),
                    want: breakpoints.len(),
                    got: values.len(),
                });
            }
            if values.iter().any(|value| !value.is_finite()) {
                return Err(ProfileError::QualityCurvesValueNonFinite { name: name.clone() });
            }
        }
        if semantics.is_empty() || semantics.len() != curves.len() {
            return Err(ProfileError::QualityCurvesSemanticsIncomplete);
        }
        for (name, semantic) in &semantics {
            if semantic.is_empty() || !curves.contains_key(name) {
                return Err(ProfileError::QualityCurvesSemanticsIncomplete);
            }
        }
        Ok(Self {
            schema,
            breakpoints,
            curves,
            semantics,
        })
    }

    pub fn schema(&self) -> &str {
        &self.schema
    }

    pub fn breakpoints(&self) -> &[f64] {
        &self.breakpoints
    }

    /// Curve names in stable (sorted) order.
    pub fn curve_names(&self) -> impl Iterator<Item = &str> {
        self.curves.keys().map(String::as_str)
    }

    pub fn curves(&self) -> &BTreeMap<String, Vec<f64>> {
        &self.curves
    }

    /// Per-curve semantics map (v2): curve name -> mechanism semantic.
    pub fn semantics(&self) -> &BTreeMap<String, String> {
        &self.semantics
    }

    /// Interpolate every curve at `quality` and return a values map
    /// (Python `evaluate`).
    pub fn evaluate(&self, quality: f64) -> Result<BTreeMap<String, f64>, ProfileError> {
        Ok(self.evaluate_result(quality)?.0)
    }

    /// Like [`evaluate`](Self::evaluate), also reporting whether the quality
    /// fell outside the recorded control points (Python `evaluate_result`).
    pub fn evaluate_result(
        &self,
        quality: f64,
    ) -> Result<(BTreeMap<String, f64>, bool), ProfileError> {
        if !quality.is_finite() {
            return Err(ProfileError::QualityValueNonFinite);
        }
        let mut values = BTreeMap::new();
        let mut extrapolated = false;
        for (name, samples) in &self.curves {
            let (value, outside) = linear_frac(&self.breakpoints, samples, quality);
            extrapolated = extrapolated || outside;
            values.insert(name.clone(), value);
        }
        Ok((values, extrapolated))
    }
}

/// The single interpolation kernel behind the whole mechanism
/// (Python `_linear_frac`; the reserved alignment hook for the
/// authoritative quality formula).
///
/// Returns `(value, extrapolated)`. Quality strictly below the first or
/// strictly above the last breakpoint uses the spec clamp rules and is
/// reported as extrapolated; the two-step fractional-index arithmetic
/// mirrors the reference behavior so both implementations agree bit for
/// bit.
pub fn linear_frac(breakpoints: &[f64], samples: &[f64], quality: f64) -> (f64, bool) {
    let n = breakpoints.len() - 1;
    let low = breakpoints[0];
    let high = breakpoints[n];
    if quality <= low {
        return (samples[0], quality < low);
    }
    if quality >= high {
        // Spec: at/past the last breakpoint the fractional index is
        // clamped to N - 0.001 so the (N-1, N) segment is used.
        let frac = n as f64 - 0.001;
        let i = n - 1;
        let f = frac - i as f64;
        let value = (1.0 - f) * samples[i] + f * samples[i + 1];
        return (value, quality > high);
    }
    // Segment search (spec profile-select loop), then the two-step
    // fractional index the reference writes and re-reads.
    let mut i = 0usize;
    while i < n && quality >= breakpoints[i + 1] {
        i += 1;
    }
    let frac = i as f64 + (quality - breakpoints[i]) / (breakpoints[i + 1] - breakpoints[i]);
    let f = frac - i as f64;
    let value = (1.0 - f) * samples[i] + f * samples[i + 1];
    (value, false)
}

/// Normalize a Wwise quality factor to the breakpoint axis
/// (spec profile-selection entry; Python `normalize_quality_factor`).
///
/// `quality` is the 0-10 factor; the result is the value the breakpoint
/// tables and curves are recorded on.
pub fn normalize_quality_factor(quality: f64) -> f64 {
    let normalized = quality / 10.0 + QUALITY_NORMALIZE_ADDEND;
    if normalized >= 1.0 {
        QUALITY_NORMALIZE_CLAMP
    } else {
        normalized
    }
}

/// Load the optional quality-curves resource (Python `load_quality_curves`).
///
/// A missing reference (profile without the resource) is handled by the
/// caller; a present resource is checksum-verified by the manifest and
/// fully validated here.
pub fn load_quality_curves(ref_: &ResourceRef) -> Result<QualityCurves, ProfileError> {
    let payload = ref_.read_json()?;
    let payload = match payload {
        serde_json::Value::Object(map) => map,
        _ => {
            return Err(ProfileError::QualityCurvesSchemaUnexpected {
                schema: String::new(),
            })
        }
    };
    let schema = payload
        .get("schema")
        .and_then(serde_json::Value::as_str)
        .unwrap_or("");
    if schema != QUALITY_CURVES_SCHEMA {
        return Err(ProfileError::QualityCurvesSchemaUnexpected {
            schema: schema.to_string(),
        });
    }
    let interpolation = payload
        .get("interpolation")
        .and_then(serde_json::Value::as_str)
        .unwrap_or("");
    if interpolation != QUALITY_CURVES_INTERPOLATION {
        return Err(ProfileError::QualityCurvesInterpolationUnsupported {
            interpolation: interpolation.to_string(),
        });
    }
    let breakpoints_value = payload
        .get("breakpoints")
        .and_then(serde_json::Value::as_array)
        .ok_or(ProfileError::QualityCurvesBreakpointsNotArray)?;
    let breakpoints = breakpoints_value
        .iter()
        .map(finite_f64)
        .collect::<Result<Vec<f64>, _>>()?;
    let raw_curves = payload
        .get("curves")
        .and_then(serde_json::Value::as_object)
        .ok_or(ProfileError::QualityCurvesCurvesNotObject)?;
    let mut curves = BTreeMap::new();
    for (name, values) in raw_curves {
        let samples = values
            .as_array()
            .ok_or_else(|| ProfileError::QualityCurvesCurveLengthMismatch {
                name: name.clone(),
                want: breakpoints.len(),
                got: 0,
            })?
            .iter()
            .map(finite_f64)
            .collect::<Result<Vec<f64>, _>>()
            .map_err(|_| ProfileError::QualityCurvesValueNonFinite { name: name.clone() })?;
        curves.insert(name.clone(), samples);
    }
    let semantics_raw = payload
        .get("semantics")
        .and_then(serde_json::Value::as_object)
        .ok_or(ProfileError::QualityCurvesSemanticsNotObject)?;
    let mut semantics = BTreeMap::new();
    for (name, semantic) in semantics_raw {
        let semantic = semantic
            .as_str()
            .ok_or(ProfileError::QualityCurvesSemanticsNotObject)?;
        semantics.insert(name.clone(), semantic.to_string());
    }
    QualityCurves::new(
        QUALITY_CURVES_SCHEMA.to_string(),
        breakpoints,
        curves,
        semantics,
    )
}

fn finite_f64(value: &serde_json::Value) -> Result<f64, ProfileError> {
    let number = value
        .as_f64()
        .ok_or_else(|| ProfileError::QualityCurvesValueNonFinite {
            name: String::new(),
        })?;
    if number.is_finite() {
        Ok(number)
    } else {
        Err(ProfileError::QualityCurvesValueNonFinite {
            name: String::new(),
        })
    }
}

#[cfg(test)]
mod tests {
    //! Spec-aligned kernel and normalization pins. The expected values are
    //! shared with the Python parity tests (same vectors, same doubles):
    //! the two implementations must agree bit for bit.

    use super::*;

    /// Draft-profile control points (2ch/48000 band, 13 breakpoints).
    const BP: &[f64] = &[
        -0.2, -0.1, 0.0, 0.1, 0.2, 0.3, 0.4, 0.5, 0.6, 0.7, 0.8, 0.9, 1.0,
    ];
    const DESC31: &[f64] = &[
        12.9, 13.8, 14.7, 15.6, 16.5, 17.1, 18.0, 19.5, 48.0, 999.0, 999.0, 999.0, 999.0,
    ];

    #[test]
    fn normalization_matches_the_spec_pinned_values() {
        assert_eq!(normalize_quality_factor(0.0), 1e-7);
        assert_eq!(normalize_quality_factor(4.0), 0.4000001);
        assert_eq!(normalize_quality_factor(9.0), 0.9000001);
        // The clamp kicks in for quality >= 10 and stays put.
        assert_eq!(normalize_quality_factor(10.0), QUALITY_NORMALIZE_CLAMP);
        assert_eq!(normalize_quality_factor(100.0), QUALITY_NORMALIZE_CLAMP);
        assert_eq!(QUALITY_NORMALIZE_CLAMP, 0.9998999834060669);
    }

    #[test]
    fn kernel_in_domain_segments_are_exact() {
        // (bp=[0,4,8], samples=[10,20,30]) shared with the Python suite.
        let bp = [0.0, 4.0, 8.0];
        let s = [10.0, 20.0, 30.0];
        assert_eq!(linear_frac(&bp, &s, 2.0), (15.0, false));
        assert_eq!(linear_frac(&bp, &s, 6.0), (25.0, false));
        // Interior control points return the stored value.
        assert_eq!(linear_frac(&bp, &s, 4.0), (20.0, false));
        assert_eq!(linear_frac(&bp, &s, 0.0), (10.0, false));
        // At the last breakpoint the 0.001 clamp formula applies.
        assert_eq!(linear_frac(&bp, &s, 8.0), (29.990000000000002, false));
    }

    #[test]
    fn kernel_above_domain_uses_the_0001_clamp_rule() {
        // (bp=[0.5,0.9], samples=[0,1]) shared with the Python suite.
        let bp = [0.5, 0.9];
        let s = [0.0, 1.0];
        assert_eq!(linear_frac(&bp, &s, 0.7), (0.4999999999999999, false));
        // Above it: the clamped value, extrapolated.
        assert_eq!(linear_frac(&bp, &s, 1.0), (0.999, true));
        assert_eq!(linear_frac(&bp, &s, 2.0), (0.999, true));
    }

    #[test]
    fn kernel_below_domain_clamps_to_first_control_point() {
        let bp = [0.5, 0.9];
        let s = [0.0, 1.0];
        assert_eq!(linear_frac(&bp, &s, 0.1), (0.0, true));
        let bp3 = [0.5, 0.9, 0.95];
        let s3 = [10.0, 20.0, 30.0];
        assert_eq!(linear_frac(&bp3, &s3, 0.0100001), (10.0, true));
    }

    #[test]
    fn kernel_two_step_fraction_differs_from_the_shortcut_for_large_i() {
        // The reference writes frac = i + ratio then reads f = frac - i;
        // for i > 0 that round-trip is not the identity, so pin the two-step
        // result on the 13-breakpoint draft table (i = 6 at q = 4.0).
        let qnorm = normalize_quality_factor(4.0);
        assert_eq!(qnorm, 0.4000001);
        let (value, outside) = linear_frac(BP, DESC31, qnorm);
        assert!(!outside);
        assert_eq!(value, 18.0000015);
    }

    #[test]
    fn curves_value_object_validates_shape_and_evaluates() {
        let curves = QualityCurves::new(
            QUALITY_CURVES_SCHEMA.to_string(),
            vec![0.0, 0.4, 0.8],
            std::collections::BTreeMap::from([
                ("desc3.psy_float".to_string(), vec![1.0, 1.0, 1.0]),
                ("desc29.psy_int1".to_string(), vec![-100.0, -105.0, -120.0]),
            ]),
            std::collections::BTreeMap::from([
                ("desc3.psy_float".to_string(), "no-op".to_string()),
                (
                    "desc29.psy_int1".to_string(),
                    "short.ath_offset".to_string(),
                ),
            ]),
        )
        .expect("valid curves");

        // q = 4.0 -> qnorm = 0.4000001 -> segment (1, 2) of the table.
        let (values, extrap) = curves
            .evaluate_result(normalize_quality_factor(4.0))
            .expect("evaluate");
        assert!(!extrap);
        assert_eq!(values["desc3.psy_float"], 1.0);
        // (1 - f) * -105 + f * -120 with f ~= 1.00000025e-6
        assert_eq!(values["desc29.psy_int1"], -105.00000375);

        // Non-finite quality is rejected (honesty contract).
        assert!(curves.evaluate_result(f64::NAN).is_err());
        assert!(curves.evaluate_result(f64::INFINITY).is_err());
    }

    #[test]
    fn curves_validation_rejects_malformed_input() {
        fn semantics_ok() -> BTreeMap<String, String> {
            BTreeMap::from([("a".to_string(), "no-op".to_string())])
        }
        assert!(matches!(
            QualityCurves::new(
                "wrong".to_string(),
                vec![0.0, 1.0],
                BTreeMap::from([("a".to_string(), vec![0.0, 0.0])]),
                semantics_ok()
            ),
            Err(ProfileError::QualityCurvesSchemaUnexpected { .. })
        ));
        assert!(matches!(
            QualityCurves::new(
                QUALITY_CURVES_SCHEMA.to_string(),
                vec![1.0],
                BTreeMap::from([("a".to_string(), vec![0.0])]),
                semantics_ok()
            ),
            Err(ProfileError::QualityCurvesTooFewBreakpoints)
        ));
        assert!(matches!(
            QualityCurves::new(
                QUALITY_CURVES_SCHEMA.to_string(),
                vec![1.0, 1.0],
                BTreeMap::from([("a".to_string(), vec![0.0, 0.0])]),
                semantics_ok()
            ),
            Err(ProfileError::QualityCurvesBreakpointsNotIncreasing)
        ));
        assert!(matches!(
            QualityCurves::new(
                QUALITY_CURVES_SCHEMA.to_string(),
                vec![0.0, 1.0],
                BTreeMap::new(),
                semantics_ok()
            ),
            Err(ProfileError::QualityCurvesEmptyCurves)
        ));
        assert!(matches!(
            QualityCurves::new(
                QUALITY_CURVES_SCHEMA.to_string(),
                vec![0.0, 1.0],
                BTreeMap::from([("a".to_string(), vec![0.0])]),
                semantics_ok()
            ),
            Err(ProfileError::QualityCurvesCurveLengthMismatch { .. })
        ));
        assert!(matches!(
            QualityCurves::new(
                QUALITY_CURVES_SCHEMA.to_string(),
                vec![0.0, 1.0],
                BTreeMap::from([("a".to_string(), vec![0.0, f64::NAN])]),
                semantics_ok()
            ),
            Err(ProfileError::QualityCurvesValueNonFinite { .. })
        ));
        // v2 semantics: must cover the curve names exactly.
        assert!(matches!(
            QualityCurves::new(
                QUALITY_CURVES_SCHEMA.to_string(),
                vec![0.0, 1.0],
                BTreeMap::from([("a".to_string(), vec![0.0, 0.0])]),
                BTreeMap::new()
            ),
            Err(ProfileError::QualityCurvesSemanticsIncomplete)
        ));
        assert!(matches!(
            QualityCurves::new(
                QUALITY_CURVES_SCHEMA.to_string(),
                vec![0.0, 1.0],
                BTreeMap::from([("a".to_string(), vec![0.0, 0.0])]),
                BTreeMap::from([("b".to_string(), "no-op".to_string())])
            ),
            Err(ProfileError::QualityCurvesSemanticsIncomplete)
        ));
    }
}
