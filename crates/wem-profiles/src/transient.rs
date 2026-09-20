//! Immutable window and band configuration for transient detection
//! (Python: `profiles/transient.py`).
//!
//! Two resource schemas are served from the same logical manifest entry
//! (`analysis.transient`):
//!
//! * `wem.transient-detector-table.v1` - a pre-materialized static detector
//!   table (the historical shape, still used by profiles that register one);
//! * `wem.transient-record-family.v1` - the static bias/threshold record
//!   family from the paired encoder build. The assembly layer materializes
//!   one detector table from it per quality value: the record-index curve on
//!   the shared quality axis picks a (possibly fractional) record index, the
//!   floor record supplies every field verbatim, and only upper[0..3] /
//!   lower[0..3] are linearly interpolated between the adjacent records at
//!   the fractional part.

use wem_analysis::config::{
    TransientBandConfig, TransientDetectorTables, CALIBRATION_SAMPLE_RATES,
};

use crate::error::ProfileError;
use crate::quality::{linear_frac, normalize_quality_factor};
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
    if !matches!(
        data.get("sample_rate").and_then(serde_json::Value::as_i64),
        Some(rate) if CALIBRATION_SAMPLE_RATES.contains(&rate)
    ) || data.get("n").and_then(serde_json::Value::as_i64) != Some(128)
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

    let bands = load_bands(&data)?;

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

fn load_bands(
    data: &serde_json::Map<String, serde_json::Value>,
) -> Result<Vec<TransientBandConfig>, ProfileError> {
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

    Ok(bands)
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

// ---------------------------------------------------------------------------
// wem.transient-record-family.v1: the static record library from the paired
// encoder build, materialized per quality.
//
// Selection/interpolation mechanism (instruction-pinned by the the extraction
// extraction, the internal record, the corresponding locations): the whole record
// at v5 = floor(index) is copied into the runtime block first; only
// upper[0..3] and lower[0..3] are then overwritten with a linear
// interpolation between records v5 and v5+1 at the fractional part. So all
// non-interpolated fields (bias, carry, marker, tail, upper[4..11],
// lower[4..11]) come verbatim from the FLOOR record.
// ---------------------------------------------------------------------------

/// Record-family schema (Python `TRANSIENT_RECORD_FAMILY_SCHEMA`).
pub const TRANSIENT_RECORD_FAMILY_SCHEMA: &str = "wem.transient-record-family.v1";
/// Record count in the paired build.
pub const TRANSIENT_RECORD_COUNT: usize = 6;
/// Words per record (marker + upper 12 + lower 12 + carry).
pub const TRANSIENT_RECORD_WORDS: usize = 26;
/// Points on the shared quality axis (breakpoints / index curve).
pub const TRANSIENT_RECORD_INDEX_POINTS: usize = 13;
/// Detector table window length (n = 128).
pub const TRANSIENT_WINDOW_WORDS: u64 = 128;
/// Band count in the descriptor table.
pub const TRANSIENT_BAND_COUNT: usize = 12;

/// One stored bias/threshold record (u32 bit patterns, byte-exact)
/// (Python `TransientRecord`).
#[derive(Debug, Clone, PartialEq)]
pub struct TransientRecord {
    pub file_off: String,
    pub marker_u32: u32,
    pub upper_u32: [u32; 12],
    pub lower_u32: [u32; 12],
    pub carry_u32: u32,
    pub bias_u32: u32,
    pub m_u32: u32,
    pub tail_u32: u32,
    /// All 26 words in runtime order: marker, upper[0..12), lower[0..12),
    /// carry.
    pub config_u32: [u32; TRANSIENT_RECORD_WORDS],
}

/// The static record library plus its immutable detector surfaces
/// (Python `TransientRecordFamily`).
#[derive(Debug, Clone, PartialEq)]
pub struct TransientRecordFamily {
    pub schema: &'static str,
    pub n: u64,
    pub sample_rate: i64,
    /// Record index used when no quality value is given (adjudicated default:
    /// 3.0, the High band record of the paired build).
    pub default_record_index: f64,
    pub records: Vec<TransientRecord>,
    /// f64 record-index curve on the shared quality axis (13 points).
    pub index_curve: Vec<f64>,
    /// Shared quality-axis breakpoints (13 points, strictly increasing).
    pub breakpoints: Vec<f64>,
    /// Static detector window words from the paired build.
    pub window_u32: Vec<u32>,
    /// Static detector band descriptors from the paired build.
    pub bands: Vec<TransientBandConfig>,
}

/// Load and validate the record-family resource
/// (Python `load_transient_record_family`).
pub fn load_transient_record_family(
    ref_: &ResourceRef,
) -> Result<TransientRecordFamily, ProfileError> {
    let data = ref_.read_json()?;
    let data = match data {
        serde_json::Value::Object(map) => map,
        _ => return Err(ProfileError::TransientRecordFamilyGeometryChanged),
    };
    if data.get("schema").and_then(serde_json::Value::as_str)
        != Some(TRANSIENT_RECORD_FAMILY_SCHEMA)
    {
        return Err(ProfileError::TransientSchemaChanged);
    }
    let sample_rate = data
        .get("sample_rate")
        .and_then(serde_json::Value::as_i64)
        .ok_or(ProfileError::TransientRecordFamilyGeometryChanged)?;
    if !CALIBRATION_SAMPLE_RATES.contains(&sample_rate) {
        return Err(ProfileError::TransientRecordFamilyGeometryChanged);
    }
    let n = data
        .get("n")
        .and_then(serde_json::Value::as_u64)
        .ok_or(ProfileError::TransientRecordFamilyGeometryChanged)?;
    if n != 128 {
        return Err(ProfileError::TransientRecordFamilyGeometryChanged);
    }

    let f64_list = |key: &'static str, length: usize| -> Result<Vec<f64>, ProfileError> {
        let err = || ProfileError::TransientRecordFamilyCurve { field: key };
        let block = data
            .get(key)
            .and_then(serde_json::Value::as_object)
            .ok_or_else(err)?;
        let values = block
            .get("values")
            .and_then(serde_json::Value::as_array)
            .filter(|values| values.len() == length)
            .ok_or_else(err)?;
        let mut out = Vec::with_capacity(length);
        for value in values {
            let number = value.as_f64().filter(|v| v.is_finite()).ok_or_else(err)?;
            out.push(number);
        }
        Ok(out)
    };

    let family = data
        .get("record_family")
        .and_then(serde_json::Value::as_object)
        .ok_or(ProfileError::TransientRecordFamilyRecordCount)?;
    if family.get("count").and_then(serde_json::Value::as_u64)
        != Some(TRANSIENT_RECORD_COUNT as u64)
    {
        return Err(ProfileError::TransientRecordFamilyRecordCount);
    }
    let raw_records = family
        .get("records")
        .and_then(serde_json::Value::as_array)
        .ok_or(ProfileError::TransientRecordFamilyRecordCount)?;
    if raw_records.len() != TRANSIENT_RECORD_COUNT {
        return Err(ProfileError::TransientRecordFamilyRecordCount);
    }
    let mut records = Vec::with_capacity(TRANSIENT_RECORD_COUNT);
    for (index, raw) in raw_records.iter().enumerate() {
        let record_err = || ProfileError::TransientRecordFamilyRecord { index };
        let raw_map = raw.as_object().ok_or_else(record_err)?;
        let u32s = |key: &str, length: usize| -> Result<Vec<u32>, ProfileError> {
            raw_map
                .get(key)
                .and_then(serde_json::Value::as_array)
                .filter(|values| values.len() == length)
                .ok_or_else(record_err)?
                .iter()
                .map(|value| u32_masked(value).ok_or_else(record_err))
                .collect()
        };
        let int = |key: &str| -> Result<u32, ProfileError> {
            u32_masked(raw_map.get(key).ok_or_else(record_err)?).ok_or_else(record_err)
        };
        let file_off = raw_map
            .get("file_off")
            .and_then(serde_json::Value::as_str)
            .filter(|s| !s.is_empty())
            .ok_or_else(record_err)?
            .to_string();
        let upper = u32s("upper_u32", 12)?;
        let lower = u32s("lower_u32", 12)?;
        let config = u32s("config_u32", TRANSIENT_RECORD_WORDS)?;
        let record = TransientRecord {
            file_off,
            marker_u32: int("marker_u32")?,
            upper_u32: upper.try_into().expect("checked length"),
            lower_u32: lower.try_into().expect("checked length"),
            carry_u32: int("carry_u32")?,
            bias_u32: int("bias_u32")?,
            m_u32: int("m_u32")?,
            tail_u32: int("tail_u32")?,
            config_u32: config.try_into().expect("checked length"),
        };
        // Field consistency (marker/carry/upper/lower vs config words).
        if record.marker_u32 != 8
            || record.config_u32[0] != record.marker_u32
            || record.config_u32[TRANSIENT_RECORD_WORDS - 1] != record.carry_u32
            || record.config_u32[1..13]
                .iter()
                .zip(&record.upper_u32)
                .any(|(a, b)| a != b)
            || record.config_u32[13..25]
                .iter()
                .zip(&record.lower_u32)
                .any(|(a, b)| a != b)
        {
            return Err(record_err());
        }
        records.push(record);
    }

    let index_curve = f64_list("record_index_curve", TRANSIENT_RECORD_INDEX_POINTS)?;
    let breakpoints = f64_list("quality_axis_breakpoints", TRANSIENT_RECORD_INDEX_POINTS)?;
    if breakpoints.windows(2).any(|pair| pair[0] >= pair[1]) {
        return Err(ProfileError::TransientRecordFamilyCurve {
            field: "quality_axis_breakpoints",
        });
    }
    if index_curve
        .iter()
        .any(|v| !v.is_finite() || *v < 0.0 || *v > (TRANSIENT_RECORD_COUNT - 1) as f64)
    {
        return Err(ProfileError::TransientRecordFamilyCurve {
            field: "record_index_curve",
        });
    }

    let window_u32 = u32_field(
        &data,
        "window_u32",
        TRANSIENT_WINDOW_WORDS as usize,
        ProfileError::TransientWindowMalformed,
    )?;
    let bands = load_bands(&data)?;

    let default_record_index = data
        .get("default_record_index")
        .and_then(serde_json::Value::as_f64)
        .filter(|v| v.is_finite())
        .ok_or(ProfileError::TransientRecordFamilyDefaultIndex)?;
    if default_record_index < 0.0 || default_record_index > (TRANSIENT_RECORD_COUNT - 1) as f64 {
        return Err(ProfileError::TransientRecordFamilyDefaultIndex);
    }

    Ok(TransientRecordFamily {
        schema: TRANSIENT_RECORD_FAMILY_SCHEMA,
        n,
        sample_rate,
        default_record_index,
        records,
        index_curve,
        breakpoints,
        window_u32,
        bands,
    })
}

/// Materialize one detector table from the record family for one quality
/// (Python `materialize_transient_tables`).
///
/// `quality` is the raw Wwise quality factor (0-10 convention) or `None`
/// for the profile's historical default gear: `None` materializes the
/// family's `default_record_index` record whole (the adjudicated default is
/// record 3). A quality value is normalized onto the shared quality axis,
/// read on the family's own record-index curve with the shared two-step
/// linear kernel, and mapped onto the family by the pinned selection rule:
///
/// * `v5 = floor(index)` (clamped to the last record at the top);
/// * every field except upper[0..3]/lower[0..3] comes verbatim from record
///   `v5` (bias, carry, marker, tail, upper[4..11], lower[4..11]);
/// * upper[0..3]/lower[0..3] are linearly interpolated between records
///   `v5` and `v5+1` at the fractional part (f64 lerp, f32 rounding).
///
/// The window and band surfaces are quality-independent static f32 words from
/// the paired build. Materialization only selects/interpolates record fields.
pub fn materialize_transient_tables(
    family: &TransientRecordFamily,
    quality: Option<f64>,
) -> Result<TransientDetectorTables, ProfileError> {
    let index = match quality {
        None => family.default_record_index,
        Some(quality) if !quality.is_finite() => return Err(ProfileError::QualityValueNonFinite),
        Some(quality) => {
            let (value, _outside) = linear_frac(
                &family.breakpoints,
                &family.index_curve,
                normalize_quality_factor(quality),
            );
            value
        }
    };
    let max_index = (family.records.len() - 1) as f64;
    let index = index.clamp(0.0, max_index);
    let v5 = index.floor() as usize;
    let v5 = v5.min(family.records.len() - 1);
    let frac = if v5 >= family.records.len() - 1 {
        0.0
    } else {
        index - v5 as f64
    };

    let base = &family.records[v5];
    let mut config_words: Vec<u32> = base.config_u32.to_vec();
    if frac > 0.0 {
        let nxt = &family.records[v5 + 1].config_u32;
        for j in 0..4u32 {
            for position in [1 + j, 13 + j] {
                let a = f32::from_bits(config_words[position as usize]) as f64;
                let b = f32::from_bits(nxt[position as usize]) as f64;
                let lerped = (1.0 - frac) * a + frac * b;
                config_words[position as usize] = (lerped as f32).to_bits(); // f32 boundary
            }
        }
    }

    Ok(TransientDetectorTables {
        n: family.n as i64,
        bias: f32::from_bits(base.bias_u32),
        window: family
            .window_u32
            .iter()
            .map(|bits| f32::from_bits(*bits))
            .collect(),
        config: config_words
            .iter()
            .map(|bits| f32::from_bits(*bits))
            .collect(),
        bands: family.bands.clone(),
    })
}

/// Dispatch on the resource schema and load/materialize the tables
/// (Python `load_transient_resource`).
///
/// `wem.transient-detector-table.v1` keeps the historical path exactly
/// (profiles that register a pre-materialized table; quality ignored);
/// `wem.transient-record-family.v1` materializes per quality (the paired
/// build's static mechanism).
pub fn load_transient(
    ref_: &ResourceRef,
    quality: Option<f64>,
) -> Result<TransientDetectorTables, ProfileError> {
    let payload = ref_.read_json()?;
    let schema = payload
        .get("schema")
        .and_then(serde_json::Value::as_str)
        .unwrap_or("");
    if schema == "wem.transient-detector-table.v1" {
        return load_transient_tables(ref_);
    }
    if schema == TRANSIENT_RECORD_FAMILY_SCHEMA {
        return materialize_transient_tables(&load_transient_record_family(ref_)?, quality);
    }
    Err(ProfileError::TransientSchemaChanged)
}
