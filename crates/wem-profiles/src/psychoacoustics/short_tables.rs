//! Short-block psychoacoustic profiles loader
//! (Python: `profiles/psychoacoustics/short_tables.py`).

use wem_analysis::config::ShortPsyProfile;

use crate::error::ProfileError;
use crate::resources::ResourceRef;

/// Load the two short-block psychoacoustic profiles.
///
/// NOTE: Python stores this table's floats as f64 (no f32 rounding), so the
/// Rust model mirrors that exactly.
pub fn load_short_psy_profiles(ref_: &ResourceRef) -> Result<Vec<ShortPsyProfile>, ProfileError> {
    let data = ref_.read_json()?;
    let data = match data {
        serde_json::Value::Object(map) => map,
        _ => return Err(ProfileError::ShortProfilesSchemaChanged),
    };
    if data.get("schema").and_then(serde_json::Value::as_str) != Some("wem.short-psy-profiles.v1")
        || super::json_int(data.get("n").unwrap_or(&serde_json::Value::Null)) != Some(128)
    {
        return Err(ProfileError::ShortProfilesSchemaChanged);
    }
    let rows = data
        .get("profiles")
        .and_then(serde_json::Value::as_array)
        .filter(|rows| rows.len() == 2)
        .ok_or(ProfileError::ShortProfilesRowCount)?;

    let mut result = Vec::with_capacity(rows.len());
    for (profile_index, row) in rows.iter().enumerate() {
        let row_map = row.as_object().ok_or(ProfileError::ShortProfilesRowCount)?;
        let field_error = |field: &'static str| ProfileError::ShortProfileFieldMalformed {
            profile: profile_index,
            field,
        };

        let curves = row_map
            .get("mask_curves")
            .and_then(serde_json::Value::as_array)
            .filter(|curves| {
                curves.len() == 3
                    && curves
                        .iter()
                        .all(|curve| curve.as_array().map(|c| c.len() == 128) == Some(true))
            })
            .ok_or({
                ProfileError::ShortProfileMaskCurvesMalformed {
                    profile: profile_index,
                }
            })?;

        let band_limits_values = row_map
            .get("band_limits")
            .and_then(serde_json::Value::as_array)
            .ok_or_else(|| field_error("band_limits"))?;
        if band_limits_values.len() != 3 {
            return Err(ProfileError::ShortProfileBandLimitsMalformed {
                profile: profile_index,
            });
        }
        let band_limits: [i64; 3] = band_limits_values
            .iter()
            .map(|value| super::json_int(value).ok_or_else(|| field_error("band_limits")))
            .collect::<Result<Vec<i64>, _>>()?
            .try_into()
            .expect("three values");

        let float_field = |field: &'static str| -> Result<f64, ProfileError> {
            super::json_f64(row_map.get(field).ok_or_else(|| field_error(field))?, field).map_err(
                |_| ProfileError::ShortProfileFieldMalformed {
                    profile: profile_index,
                    field,
                },
            )
        };
        let int_field = |field: &'static str| -> Result<i64, ProfileError> {
            super::json_int(row_map.get(field).ok_or_else(|| field_error(field))?)
                .ok_or_else(|| field_error(field))
        };
        let bias_values = row_map
            .get("candidate_bias_by_mode")
            .and_then(serde_json::Value::as_array)
            .ok_or_else(|| field_error("candidate_bias_by_mode"))?;
        let candidate_bias_by_mode = bias_values
            .iter()
            .map(|value| {
                super::json_f64(value, "candidate_bias_by_mode").map_err(|_| {
                    ProfileError::ShortProfileFieldMalformed {
                        profile: profile_index,
                        field: "candidate_bias_by_mode",
                    }
                })
            })
            .collect::<Result<_, _>>()?;
        let mask_curves = curves
            .iter()
            .map(|curve| {
                curve
                    .as_array()
                    .ok_or_else(|| field_error("mask_curves"))?
                    .iter()
                    .map(|value| {
                        super::json_f64(value, "mask_curves").map_err(|_| {
                            ProfileError::ShortProfileFieldMalformed {
                                profile: profile_index,
                                field: "mask_curves",
                            }
                        })
                    })
                    .collect::<Result<_, _>>()
            })
            .collect::<Result<_, _>>()?;

        result.push(ShortPsyProfile {
            key: row_map
                .get("key")
                .and_then(serde_json::Value::as_str)
                .map(str::to_string)
                .or_else(|| {
                    row_map
                        .get("key")
                        .and_then(serde_json::Value::as_i64)
                        .map(|v| v.to_string())
                })
                .ok_or_else(|| field_error("key"))?,
            candidate_bias_by_mode,
            curve_cap: float_field("curve_cap")?,
            group_enabled: int_field("group_enabled")?,
            candidate_bound: int_field("candidate_bound")?,
            group_span: int_field("group_span")?,
            band_limits,
            side_gain: float_field("side_gain")?,
            peak_cutoff: int_field("peak_cutoff")?,
            blend_weight: float_field("blend_weight")?,
            mask_curves,
        });
    }
    Ok(result)
}
