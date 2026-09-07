//! Load the checksum-addressed frozen transcendental tables of one exact
//! profile (Python: `profiles/frozen.py`).
//!
//! All floating-point content is restored from stored IEEE bit patterns
//! (little-endian byte-hex); no float string parsing anywhere.

use std::collections::HashMap;

use wem_analysis::config::FrozenMathTables;

use crate::error::ProfileError;
use crate::resources::ResourceRef;

/// Validate the frozen-math payload shape and return typed bit-mapped tables.
pub fn load_frozen_tables(ref_: &ResourceRef) -> Result<FrozenMathTables, ProfileError> {
    let payload = ref_.read_json()?;
    let payload = match payload {
        serde_json::Value::Object(map) => map,
        _ => {
            return Err(ProfileError::FrozenSchemaUnexpected {
                schema: String::new(),
            })
        }
    };
    if payload.get("schema").and_then(serde_json::Value::as_str) != Some("wem.frozen-math.v1") {
        return Err(ProfileError::FrozenSchemaUnexpected {
            schema: payload
                .get("schema")
                .and_then(serde_json::Value::as_str)
                .unwrap_or("")
                .to_string(),
        });
    }

    let mut coordinate_ln = HashMap::new();
    for entry in payload
        .get("coordinate_ln")
        .and_then(serde_json::Value::as_array)
        .into_iter()
        .flatten()
    {
        let pair = entry.as_array().filter(|pair| pair.len() == 2).ok_or(
            ProfileError::FrozenEntryMalformed {
                section: "coordinate_ln",
            },
        )?;
        let in_bits = hex_le_u64(
            pair[0].as_str().ok_or(ProfileError::FrozenEntryMalformed {
                section: "coordinate_ln",
            })?,
            "coordinate_ln",
        )?;
        let out_bits = hex_le_u64(
            pair[1].as_str().ok_or(ProfileError::FrozenEntryMalformed {
                section: "coordinate_ln",
            })?,
            "coordinate_ln",
        )?;
        coordinate_ln.insert(in_bits, f64::from_bits(out_bits));
    }

    let mut fft_twiddles = HashMap::new();
    for entry in payload
        .get("fft_twiddles")
        .and_then(serde_json::Value::as_array)
        .into_iter()
        .flatten()
    {
        let triple = entry.as_array().filter(|triple| triple.len() == 3).ok_or(
            ProfileError::FrozenEntryMalformed {
                section: "fft_twiddles",
            },
        )?;
        let length = triple[0]
            .as_str()
            .and_then(|value| value.parse::<i64>().ok())
            .ok_or_else(|| ProfileError::FrozenTwiddleLengthMalformed {
                value: triple[0].to_string(),
            })?;
        let cos_bits = hex_le_u64(
            triple[1]
                .as_str()
                .ok_or(ProfileError::FrozenEntryMalformed {
                    section: "fft_twiddles",
                })?,
            "fft_twiddles",
        )?;
        let sin_bits = hex_le_u64(
            triple[2]
                .as_str()
                .ok_or(ProfileError::FrozenEntryMalformed {
                    section: "fft_twiddles",
                })?,
            "fft_twiddles",
        )?;
        fft_twiddles.insert(length, (f64::from_bits(cos_bits), f64::from_bits(sin_bits)));
    }

    let mut window_halves: HashMap<i64, Vec<f32>> = HashMap::new();
    if let Some(words) = payload.get("window_halves") {
        let words = words
            .as_object()
            .ok_or(ProfileError::FrozenEntryMalformed {
                section: "window_halves",
            })?;
        for (key, values) in words {
            let size = key
                .parse::<i64>()
                .map_err(|_| ProfileError::FrozenWindowHalvesMismatch { size: -1 })?;
            let values = values
                .as_array()
                .ok_or(ProfileError::FrozenEntryMalformed {
                    section: "window_halves",
                })?;
            if size < 2 || size % 2 != 0 || values.len() != size as usize / 2 {
                return Err(ProfileError::FrozenWindowHalvesMismatch { size });
            }
            let mut half = Vec::with_capacity(values.len());
            for value in values {
                let bits = hex_le_u32(
                    value.as_str().ok_or(ProfileError::FrozenEntryMalformed {
                        section: "window_halves",
                    })?,
                    "window_halves",
                )?;
                half.push(f32::from_bits(bits));
            }
            window_halves.insert(size, half);
        }
    }

    if coordinate_ln.is_empty() || fft_twiddles.is_empty() || window_halves.is_empty() {
        return Err(ProfileError::FrozenSectionMissing);
    }
    Ok(FrozenMathTables {
        coordinate_ln,
        fft_twiddles,
        window_halves,
    })
}

/// Little-endian byte-hex to u64 bit pattern (Python `_hex_to_u64`).
fn hex_le_u64(hex: &str, section: &'static str) -> Result<u64, ProfileError> {
    let bytes = hex_bytes(hex, section)?;
    if bytes.len() != 8 {
        return Err(ProfileError::FrozenEntryMalformed { section });
    }
    Ok(u64::from_le_bytes(bytes.try_into().expect("8 bytes")))
}

/// Little-endian byte-hex to u32 bit pattern (Python `_hex_to_u32`); longer
/// inputs keep only the low 32 bits, mirroring `int & 0xFFFFFFFF`.
fn hex_le_u32(hex: &str, section: &'static str) -> Result<u32, ProfileError> {
    let bytes = hex_bytes(hex, section)?;
    let mut low = [0u8; 4];
    let n = bytes.len().min(4);
    low[..n].copy_from_slice(&bytes[..n]);
    Ok(u32::from_le_bytes(low))
}

fn hex_bytes(hex: &str, section: &'static str) -> Result<Vec<u8>, ProfileError> {
    let hex = hex.trim();
    if !hex.len().is_multiple_of(2) {
        return Err(ProfileError::FrozenEntryMalformed { section });
    }
    hex.as_bytes()
        .chunks_exact(2)
        .map(|pair| u8::from_str_radix(std::str::from_utf8(pair).expect("validated ascii"), 16))
        .collect::<Result<_, _>>()
        .map_err(|_| ProfileError::FrozenEntryMalformed { section })
}
