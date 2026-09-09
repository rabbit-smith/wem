//! Short seed-surface loader (Python: `profiles/psychoacoustics/config.py`).

use wem_analysis::config::{
    CALIBRATION_SAMPLE_RATES, TONE_BAND_COUNT, TONE_LEVEL_COUNT, WwisePsySeedSurface,
};

use super::{json_f32, json_int};
use crate::error::ProfileError;
use crate::resources::ResourceRef;

const TONE_SHAPE: [i64; 3] = [TONE_BAND_COUNT, TONE_LEVEL_COUNT, 58];
const TONE_TOTAL_WORDS: i64 = TONE_BAND_COUNT * TONE_LEVEL_COUNT * 58;

/// Load the checksum-addressed short profile and validate its complete shape.
pub fn load_short_seed_surface(ref_: &ResourceRef) -> Result<WwisePsySeedSurface, ProfileError> {
    let payload = ref_.read_json()?;
    let payload = match payload {
        serde_json::Value::Object(map) => map,
        _ => {
            return Err(ProfileError::ShortSeedSchemaUnexpected {
                schema: String::new(),
            })
        }
    };
    if payload.get("schema").and_then(serde_json::Value::as_str) != Some("wem.seed-surface.v6") {
        return Err(ProfileError::ShortSeedSchemaUnexpected {
            schema: payload
                .get("schema")
                .and_then(serde_json::Value::as_str)
                .unwrap_or("")
                .to_string(),
        });
    }

    let section = |name: &str| -> Option<&serde_json::Map<String, serde_json::Value>> {
        payload.get(name).and_then(serde_json::Value::as_object)
    };
    let profile =
        section("profile").ok_or(ProfileError::ShortSeedSectionMissing { section: "profile" })?;
    let geometry = section("geometry").ok_or(ProfileError::ShortSeedSectionMissing {
        section: "geometry",
    })?;
    let look = section("look").ok_or(ProfileError::ShortSeedSectionMissing { section: "look" })?;

    let sample_rate = json_int(
        geometry
            .get("sample_rate")
            .unwrap_or(&serde_json::Value::Null),
    );
    if json_int(geometry.get("n").unwrap_or(&serde_json::Value::Null)) != Some(128)
        || !matches!(
            sample_rate,
            Some(rate) if CALIBRATION_SAMPLE_RATES.contains(&rate)
        )
    {
        return Err(ProfileError::ShortSeedGeometryUnsupported);
    }

    let tone_shape = payload
        .get("tone_shape")
        .and_then(serde_json::Value::as_array);
    let tone_shape_ok = tone_shape
        .map(|shape| {
            shape
                .iter()
                .enumerate()
                .take(3)
                .all(|(i, value)| json_int(value) == Some(TONE_SHAPE[i]))
                && shape.len() == 3
        })
        .unwrap_or(false);
    if !tone_shape_ok {
        return Err(ProfileError::ShortSeedToneShapeUnexpected);
    }

    // Tone RLE expansion.
    let tone_rle = payload
        .get("tone_rle")
        .and_then(serde_json::Value::as_array)
        .ok_or(ProfileError::ShortSeedToneRleEntryMalformed)?;
    let mut raw_tone: Vec<f32> = Vec::with_capacity(TONE_TOTAL_WORDS as usize);
    for entry in tone_rle {
        let pair = entry
            .as_array()
            .filter(|pair| pair.len() == 2)
            .ok_or(ProfileError::ShortSeedToneRleEntryMalformed)?;
        let value = json_f32(&pair[0], "tone_rle value")?;
        let count = json_int(&pair[1])
            .filter(|count| *count > 0)
            .ok_or(ProfileError::ShortSeedToneRleCountInvalid)?;
        raw_tone.extend(std::iter::repeat_n(value, count as usize));
    }
    if raw_tone.len() as i64 != TONE_TOTAL_WORDS || !raw_tone.iter().all(|value| value.is_finite())
    {
        return Err(ProfileError::ShortSeedToneDataMalformed);
    }
    let tone_curves: Vec<Vec<Vec<f32>>> = (0..TONE_BAND_COUNT)
        .map(|band| {
            (0..TONE_LEVEL_COUNT)
                .map(|level| {
                    let start = (band * TONE_LEVEL_COUNT + level) * 58;
                    raw_tone[start as usize..(start + 58) as usize].to_vec()
                })
                .collect()
        })
        .collect();

    // Profile scalars.
    let scalar = |field: &'static str, map: &serde_json::Map<String, serde_json::Value>| {
        json_f32(
            map.get(field)
                .ok_or(ProfileError::ShortSeedFieldMalformed { field })?,
            field,
        )
    };

    // Float tuples with exact-length + finiteness checks.
    let float_tuple = |field: &'static str,
                       map: &serde_json::Map<String, serde_json::Value>,
                       size: usize|
     -> Result<Vec<f32>, ProfileError> {
        let values = map
            .get(field)
            .and_then(serde_json::Value::as_array)
            .ok_or(ProfileError::ShortSeedFieldMalformed { field })?;
        if values.len() != size {
            return Err(ProfileError::ShortSeedFieldMalformed { field });
        }
        let mut out = Vec::with_capacity(size);
        for value in values {
            let f32_value = json_f32(value, field)?;
            if !f32_value.is_finite() {
                return Err(ProfileError::ShortSeedFieldMalformed { field });
            }
            out.push(f32_value);
        }
        Ok(out)
    };

    // Integer tuples with exact-length + integer checks.
    let int_tuple = |field: &'static str,
                     map: &serde_json::Map<String, serde_json::Value>,
                     size: usize|
     -> Result<Vec<i64>, ProfileError> {
        let values = map
            .get(field)
            .and_then(serde_json::Value::as_array)
            .ok_or(ProfileError::ShortSeedFieldMalformed { field })?;
        if values.len() != size {
            return Err(ProfileError::ShortSeedFieldMalformed { field });
        }
        let mut out = Vec::with_capacity(size);
        for value in values {
            let integer =
                strict_int(value).ok_or(ProfileError::ShortSeedFieldMalformed { field })?;
            out.push(integer);
        }
        Ok(out)
    };

    let geometry_int = |field: &'static str| {
        json_int(
            geometry
                .get(field)
                .ok_or(ProfileError::ShortSeedFieldMalformed { field })?,
        )
        .ok_or(ProfileError::ShortSeedFieldMalformed { field })
    };

    Ok(WwisePsySeedSurface {
        n: 128,
        sample_rate: sample_rate.expect("geometry validated the sample rate"),
        ath_offset: scalar("ath_offset", profile)?,
        ath_floor: scalar("ath_floor", profile)?,
        seed_ceiling: scalar("seed_ceiling", profile)?,
        max_curve_db: scalar("max_curve_db", profile)?,
        curve_offset: scalar("curve_offset", profile)?,
        curve_slope: scalar("curve_slope", profile)?,
        curve_offset_2: scalar("curve_offset_2", profile)?,
        curve_attenuation: float_tuple("curve_attenuation", profile, TONE_BAND_COUNT as usize)?,
        ath: float_tuple("ath", &payload, 128)?,
        octave: int_tuple("octave", &payload, 128)?,
        first_octave: geometry_int("first_octave")?,
        shift_octave: geometry_int("shift_octave")?,
        eighth_octave_lines: geometry_int("eighth_octave_lines")?,
        total_octave_lines: geometry_int("total_octave_lines")?,
        tone_curves,
        row: float_tuple("row", look, 133)?,
        envelope_low: float_tuple("envelope_low", look, 17)?,
        envelope_high: float_tuple("envelope_high", look, 17)?,
        mask_curve: float_tuple("mask_curve", look, 128)?,
        interval_table: int_tuple("interval_table", look, 128)?,
        short_limit: json_int(look.get("short_limit").ok_or(
            ProfileError::ShortSeedFieldMalformed {
                field: "short_limit",
            },
        )?)
        .ok_or(ProfileError::ShortSeedFieldMalformed {
            field: "short_limit",
        })?,
        noise_fixed_window: json_int(look.get("noise_fixed_window").ok_or(
            ProfileError::ShortSeedFieldMalformed {
                field: "noise_fixed_window",
            },
        )?)
        .ok_or(ProfileError::ShortSeedFieldMalformed {
            field: "noise_fixed_window",
        })?,
        regular_curve_bias: scalar("regular_curve_bias", look)?,
        regular_curve_cap: scalar("regular_curve_cap", look)?,
        remap_curve_offsets: int_tuple("remap_curve_offsets", look, 40)?,
        remap_low_by_index: float_tuple("remap_low_by_index", look, 40)?,
        remap_high_by_index: float_tuple("remap_high_by_index", look, 40)?,
    })
}

/// Strict JSON integer (rejects booleans and floats).
fn strict_int(value: &serde_json::Value) -> Option<i64> {
    match value {
        serde_json::Value::Number(n) if n.is_i64() || n.is_u64() => n
            .as_i64()
            .or_else(|| n.as_u64().and_then(|v| i64::try_from(v).ok())),
        _ => None,
    }
}
