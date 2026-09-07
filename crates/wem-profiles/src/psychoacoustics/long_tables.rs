//! Strict loader for the fixed 1024-bin Wwise psychoacoustic tables
//! (Python: `profiles/psychoacoustics/long_tables.py`).

use wem_analysis::config::{WwisePsyLongTables, TONE_BAND_COUNT, TONE_LEVEL_COUNT};

use crate::error::ProfileError;
use crate::resources::ResourceRef;

const SCHEMA: &str = "wem.psy-long-static.v1";
const N: i64 = 1024;
const PROFILE_WORDS: usize = 256;
const LOOK_WORDS: usize = 192;
const TONE_RECORD_FLOATS: usize = 58;

/// Load one validated static long table.
pub fn load_long_psy_tables(ref_: &ResourceRef) -> Result<WwisePsyLongTables, ProfileError> {
    let payload = ref_.read_json()?;
    let payload = match payload {
        serde_json::Value::Object(map) => map,
        _ => {
            return Err(ProfileError::LongTablesSchemaUnexpected {
                schema: String::new(),
            })
        }
    };
    if payload.get("schema").and_then(serde_json::Value::as_str) != Some(SCHEMA) {
        return Err(ProfileError::LongTablesSchemaUnexpected {
            schema: payload
                .get("schema")
                .and_then(serde_json::Value::as_str)
                .unwrap_or("")
                .to_string(),
        });
    }
    if payload.get("n").and_then(serde_json::Value::as_i64) != Some(N)
        || payload
            .get("sample_rate")
            .and_then(serde_json::Value::as_i64)
            != Some(44100)
    {
        return Err(ProfileError::LongTablesGeometryUnexpected);
    }
    let profile_key = payload
        .get("profile_key")
        .and_then(serde_json::Value::as_str)
        .filter(|key| !key.is_empty())
        .ok_or(ProfileError::LongTablesProfileKeyMissing)?;

    let analysis = payload
        .get("analysis")
        .and_then(serde_json::Value::as_object)
        .ok_or(ProfileError::LongTablesSectionMissing {
            section: "analysis",
        })?;
    let seed = payload
        .get("seed")
        .and_then(serde_json::Value::as_object)
        .ok_or(ProfileError::LongTablesSectionMissing { section: "seed" })?;

    let curves_raw = analysis
        .get("curves")
        .and_then(serde_json::Value::as_array)
        .filter(|rows| rows.len() == 3)
        .ok_or(ProfileError::LongTablesCurvesMalformed)?;
    let analysis_curves = curves_raw
        .iter()
        .map(|row| floats(Some(row), N as usize, "analysis.curves"))
        .collect::<Result<Vec<_>, _>>()?;

    let banks_raw = seed
        .get("tone_banks")
        .and_then(serde_json::Value::as_array)
        .filter(|banks| banks.len() == TONE_BAND_COUNT as usize)
        .ok_or(ProfileError::LongTablesSectionMissing {
            section: "seed.tone_banks",
        })?;
    let mut banks = Vec::with_capacity(banks_raw.len());
    for (band, levels) in banks_raw.iter().enumerate() {
        let levels = levels
            .as_array()
            .filter(|levels| levels.len() == TONE_LEVEL_COUNT as usize)
            .ok_or(ProfileError::LongTablesToneBandsMalformed { band })?;
        let bank = levels
            .iter()
            .map(|record| floats(Some(record), TONE_RECORD_FLOATS, "seed.tone_banks record"))
            .collect::<Result<Vec<_>, _>>()?;
        banks.push(bank);
    }

    Ok(WwisePsyLongTables {
        sample_rate: 44100,
        n: N,
        profile_key: profile_key.to_string(),
        analysis_profile_u32: u32s(
            analysis.get("profile_u32"),
            PROFILE_WORDS,
            "analysis.profile_u32",
        )?,
        analysis_interval_u32: u32s(
            analysis.get("interval_u32"),
            N as usize,
            "analysis.interval_u32",
        )?,
        analysis_curves,
        analysis_field_19_curve: floats(
            analysis.get("field_19_curve"),
            N as usize,
            "analysis.field_19_curve",
        )?,
        seed_outer_u32: u32s(seed.get("outer_u32"), LOOK_WORDS, "seed.outer_u32")?,
        seed_profile_u32: u32s(seed.get("profile_u32"), PROFILE_WORDS, "seed.profile_u32")?,
        seed_base_curve: floats(seed.get("base_curve"), N as usize, "seed.base_curve")?,
        seed_group_labels_u32: u32s(
            seed.get("group_labels_u32"),
            N as usize,
            "seed.group_labels_u32",
        )?,
        seed_tone_banks: banks,
    })
}

/// `analysis_field_19_curve` helper alias used above.
fn floats(
    value: Option<&serde_json::Value>,
    count: usize,
    label: &'static str,
) -> Result<Vec<f32>, ProfileError> {
    let items = value
        .and_then(serde_json::Value::as_array)
        .filter(|items| items.len() == count)
        .ok_or(ProfileError::LongTablesFieldMalformed { field: label })?;
    items
        .iter()
        .map(|item| {
            let number = item
                .as_f64()
                .and_then(|v| v.is_finite().then_some(v))
                .ok_or(ProfileError::LongTablesFieldMalformed { field: label })?;
            Ok(number as f32)
        })
        .collect()
}

fn u32s(
    value: Option<&serde_json::Value>,
    count: usize,
    label: &'static str,
) -> Result<Vec<u32>, ProfileError> {
    let items = value
        .and_then(serde_json::Value::as_array)
        .filter(|items| items.len() == count)
        .ok_or(ProfileError::LongTablesFieldMalformed { field: label })?;
    items
        .iter()
        .map(|item| {
            // Python `_u32`: strict JSON int within [0, 0xFFFFFFFF].
            match item {
                serde_json::Value::Number(n) if n.is_i64() || n.is_u64() => {
                    let v = n
                        .as_i64()
                        .or_else(|| n.as_u64().and_then(|v| i64::try_from(v).ok()))
                        .ok_or(ProfileError::LongTablesFieldMalformed { field: label })?;
                    if !(0..=0xFFFF_FFFF).contains(&v) {
                        return Err(ProfileError::LongTablesFieldMalformed { field: label });
                    }
                    Ok(v as u32)
                }
                _ => Err(ProfileError::LongTablesFieldMalformed { field: label }),
            }
        })
        .collect()
}
