//! Immutable window and band configuration for transient detection
//! (Python: `profiles/transient.py`).

use wem_analysis::config::{TransientBandConfig, TransientDetectorTables};

use crate::error::ProfileError;
use crate::resources::ResourceRef;

/// Load the checked n=128 static detector table.
pub fn load_transient_tables(ref_: &ResourceRef) -> Result<TransientDetectorTables, ProfileError> {
    let data = ref_.read_json()?;
    let data = match data {
        serde_json::Value::Object(map) => map,
        _ => return Err(ProfileError::TransientSchemaChanged),
    };
    if data.get("schema").and_then(serde_json::Value::as_str)
        != Some("wem.transient-detector-table.v1")
    {
        return Err(ProfileError::TransientSchemaChanged);
    }
    if data.get("sample_rate").and_then(serde_json::Value::as_i64) != Some(44100)
        || data.get("n").and_then(serde_json::Value::as_i64) != Some(128)
    {
        return Err(ProfileError::TransientSchemaChanged);
    }

    let window_u32 = u32_field(
        &data,
        "window_u32",
        128,
        ProfileError::TransientWindowMalformed,
    )?;
    let config_u32 = u32_field(
        &data,
        "config_u32",
        26,
        ProfileError::TransientConfigMalformed,
    )?;

    let rows = data
        .get("bands")
        .and_then(serde_json::Value::as_array)
        .ok_or(ProfileError::TransientFieldMissing { field: "bands" })?;
    if rows.len() != 12 {
        return Err(ProfileError::TransientBandsMalformed);
    }
    let mut bands = Vec::with_capacity(rows.len());
    for (band_index, row) in rows.iter().enumerate() {
        let row_map = row
            .as_object()
            .ok_or(ProfileError::TransientRowMalformed { band: band_index })?;
        let row_err = || ProfileError::TransientRowMalformed { band: band_index };

        let strict_int = |field: &str| -> Result<i64, ProfileError> {
            let value = row_map.get(field).ok_or_else(row_err)?;
            strict_json_int(value).ok_or_else(row_err)
        };

        let count = strict_int("count")?;
        let offset = strict_int("offset")?;
        let scale_bits =
            u32_masked(row_map.get("scale_u32").ok_or_else(row_err)?).ok_or_else(row_err)?;
        let weights = row_map
            .get("weights_u32")
            .and_then(serde_json::Value::as_array)
            .ok_or_else(row_err)?;
        if count < 0 || weights.len() != count as usize {
            return Err(row_err());
        }
        let weights = weights
            .iter()
            .map(|value| Ok(f32::from_bits(u32_masked(value).ok_or_else(row_err)?)))
            .collect::<Result<_, _>>()?;
        bands.push(TransientBandConfig {
            offset,
            weights,
            scale: f32::from_bits(scale_bits),
        });
    }

    let bias_bits = u32_masked(
        data.get("bias_u32")
            .ok_or(ProfileError::TransientFieldMissing { field: "bias_u32" })?,
    )
    .ok_or(ProfileError::TransientFieldMissing { field: "bias_u32" })?;

    Ok(TransientDetectorTables {
        n: 128,
        bias: f32::from_bits(bias_bits),
        window: window_u32
            .iter()
            .map(|bits| f32::from_bits(*bits))
            .collect(),
        config: config_u32
            .iter()
            .map(|bits| f32::from_bits(*bits))
            .collect(),
        bands,
    })
}

/// A JSON field that must be a list of exactly `expected_len` stored u32s.
fn u32_field(
    data: &serde_json::Map<String, serde_json::Value>,
    field: &str,
    expected_len: usize,
    wrong_length: ProfileError,
) -> Result<Vec<u32>, ProfileError> {
    let values = data
        .get(field)
        .and_then(serde_json::Value::as_array)
        .filter(|values| values.len() == expected_len)
        .ok_or(wrong_length.clone())?;
    values
        .iter()
        .map(|value| u32_masked(value).ok_or(wrong_length.clone()))
        .collect()
}

/// Strict JSON integer check (Python `isinstance(value, int)` on JSON
/// numbers; booleans are a separate JSON type and are rejected).
fn strict_json_int(value: &serde_json::Value) -> Option<i64> {
    match value {
        serde_json::Value::Number(n) if n.is_i64() || n.is_u64() => n
            .as_i64()
            .or_else(|| n.as_u64().and_then(|v| i64::try_from(v).ok())),
        _ => None,
    }
}

/// Python `int(value) & 0xFFFFFFFF` for stored u32 words.
fn u32_masked(value: &serde_json::Value) -> Option<u32> {
    let v = match value {
        serde_json::Value::Number(n) if n.is_i64() || n.is_u64() => n
            .as_i64()
            .or_else(|| n.as_u64().and_then(|v| i64::try_from(v).ok()))?,
        serde_json::Value::Number(n) if n.is_f64() => {
            n.as_f64().and_then(|v| v.is_finite().then_some(v))?.trunc() as i64
        }
        _ => return None,
    };
    Some(v as u32)
}
