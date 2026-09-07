//! Psychoacoustic resource loaders (Python: `profiles/psychoacoustics/`).
//!
//! * `config` — short seed-surface (`wem.seed-surface.v6`)
//! * `short_tables` — short-block psychoacoustic profiles
//! * `long_tables` — fixed 1024-bin long tables
//! * `long_variants` — long analysis mode 2/3 surface

pub mod config;
pub mod long_tables;
pub mod long_variants;
pub mod short_tables;

/// Python `_f32(float(x))`: number -> f32-rounded float.
pub(crate) fn json_f32(
    value: &serde_json::Value,
    field: &'static str,
) -> Result<f32, crate::error::ProfileError> {
    let number = value
        .as_f64()
        .and_then(|v| v.is_finite().then_some(v))
        .ok_or(crate::error::ProfileError::ShortSeedFieldMalformed { field })?;
    Ok(number as f32)
}

/// Python `float()` on numbers for f64 fields (short profiles, etc).
pub(crate) fn json_f64(
    value: &serde_json::Value,
    field: &'static str,
) -> Result<f64, crate::error::ProfileError> {
    value
        .as_f64()
        .and_then(|v| v.is_finite().then_some(v))
        .ok_or(crate::error::ProfileError::ShortProfileFieldMalformed { profile: 0, field })
}

/// Python `int()` on numbers (truncates finite floats toward zero).
pub(crate) fn json_int(value: &serde_json::Value) -> Option<i64> {
    match value {
        serde_json::Value::Number(n) if n.is_i64() => n.as_i64(),
        serde_json::Value::Number(n) if n.is_u64() => {
            n.as_u64().and_then(|v| i64::try_from(v).ok())
        }
        serde_json::Value::Number(n) if n.is_f64() => {
            n.as_f64().and_then(|v| v.is_finite().then_some(v as i64))
        }
        _ => None,
    }
}
