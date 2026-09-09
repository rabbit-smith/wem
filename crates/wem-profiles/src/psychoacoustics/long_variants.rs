//! Pure loader and runtime adapter for long analysis mode 2/mode 3
//! (Python: `profiles/psychoacoustics/long_variants.py`).

use wem_analysis::config::{CALIBRATION_SAMPLE_RATES, WwisePsyLongTables};

use crate::error::ProfileError;
use crate::resources::ResourceRef;

const SCHEMA: &str = "wem.psy-long-analysis-variants.v1";

/// Return a complete long table with the selected analysis surface.
///
/// The floor-seed surface is shared by both modes and inherited from the
/// checked base profile rather than duplicated in the variant resource.
pub fn load_long_variant(
    mode: i64,
    ref_: &ResourceRef,
    base: &WwisePsyLongTables,
) -> Result<WwisePsyLongTables, ProfileError> {
    if mode != 2 && mode != 3 {
        return Err(ProfileError::LongVariantModeInvalid { mode });
    }
    let payload = ref_.read_json()?;
    let payload = match payload {
        serde_json::Value::Object(map) => map,
        _ => {
            return Err(ProfileError::LongVariantSchemaUnexpected {
                schema: String::new(),
            })
        }
    };
    if payload.get("schema").and_then(serde_json::Value::as_str) != Some(SCHEMA)
        || payload.get("n").and_then(serde_json::Value::as_i64) != Some(1024)
        || !matches!(
            payload.get("sample_rate").and_then(serde_json::Value::as_i64),
            Some(rate) if CALIBRATION_SAMPLE_RATES.contains(&rate)
        )
    {
        return Err(ProfileError::LongVariantSchemaUnexpected {
            schema: payload
                .get("schema")
                .and_then(serde_json::Value::as_str)
                .unwrap_or("")
                .to_string(),
        });
    }

    let variants = payload
        .get("variants")
        .and_then(serde_json::Value::as_object)
        .ok_or(ProfileError::LongVariantMissingMode { mode })?;
    let surface = variants
        .get(mode.to_string().as_str())
        .and_then(serde_json::Value::as_object)
        .ok_or(ProfileError::LongVariantMissingMode { mode })?;

    let analysis_profile_u32 = surface
        .get("profile_u32")
        .and_then(serde_json::Value::as_array)
        .filter(|items| items.len() == 256)
        .ok_or(ProfileError::LongVariantProfileMalformed)?
        .iter()
        .map(|item| {
            super::json_int(item)
                .map(|v| v as u32)
                .ok_or(ProfileError::LongVariantProfileMalformed)
        })
        .collect::<Result<Vec<u32>, _>>()?;
    let analysis_interval_u32 = surface
        .get("interval_u32")
        .and_then(serde_json::Value::as_array)
        .filter(|items| items.len() == 1024)
        .ok_or(ProfileError::LongVariantIntervalsMalformed)?
        .iter()
        .map(|item| {
            super::json_int(item)
                .map(|v| v as u32)
                .ok_or(ProfileError::LongVariantIntervalsMalformed)
        })
        .collect::<Result<Vec<u32>, _>>()?;

    let curves = surface
        .get("curves")
        .and_then(serde_json::Value::as_array)
        .filter(|rows| {
            rows.len() == 3
                && rows
                    .iter()
                    .all(|row| row.as_array().map(|c| c.len() == 1024) == Some(true))
        })
        .ok_or(ProfileError::LongVariantCurvesMalformed)?;
    let analysis_curves = curves
        .iter()
        .map(|row| {
            row.as_array()
                .ok_or(ProfileError::LongVariantCurvesMalformed)?
                .iter()
                .map(|value| {
                    let number = value
                        .as_f64()
                        .and_then(|v| v.is_finite().then_some(v))
                        .ok_or(ProfileError::LongVariantCurvesMalformed)?;
                    Ok(number as f32)
                })
                .collect::<Result<Vec<_>, _>>()
        })
        .collect::<Result<Vec<_>, _>>()?;

    let field19 = surface
        .get("field_19_curve")
        .and_then(serde_json::Value::as_array)
        .filter(|row| row.len() == 1024)
        .ok_or(ProfileError::LongVariantField19Malformed)?;
    let analysis_field_19_curve = field19
        .iter()
        .map(|value| {
            let number = value
                .as_f64()
                .and_then(|v| v.is_finite().then_some(v))
                .ok_or(ProfileError::LongVariantField19Malformed)?;
            Ok(number as f32)
        })
        .collect::<Result<Vec<_>, _>>()?;

    Ok(WwisePsyLongTables {
        sample_rate: base.sample_rate,
        n: base.n,
        profile_key: format!("1024:{mode}"),
        analysis_profile_u32,
        analysis_interval_u32,
        analysis_curves,
        analysis_field_19_curve,
        seed_outer_u32: base.seed_outer_u32.clone(),
        seed_profile_u32: base.seed_profile_u32.clone(),
        seed_base_curve: base.seed_base_curve.clone(),
        seed_group_labels_u32: base.seed_group_labels_u32.clone(),
        seed_tone_banks: base.seed_tone_banks.clone(),
    })
}
