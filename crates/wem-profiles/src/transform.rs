//! Checksum-addressed MDCT table decoding for profile assembly
//! (Python: `profiles/transform.py`).

use std::collections::{BTreeMap, BTreeSet};

use wem_analysis::config::MdctLook;
use wem_analysis::dsp::transform::make_mdct_look;

use crate::error::ProfileError;
use crate::resources::ResourceRef;

/// Decode all static trig banks from an explicit manifest resource.
pub fn load_mdct_looks(ref_: &ResourceRef) -> Result<BTreeMap<i64, MdctLook>, ProfileError> {
    let payload = ref_.read_json()?;
    let payload = match payload {
        serde_json::Value::Object(map) => map,
        _ => {
            return Err(ProfileError::MdctSchemaUnsupported {
                schema: String::new(),
            })
        }
    };
    if payload.get("schema").and_then(serde_json::Value::as_str) != Some("wem.mdct-trig-static.v1")
    {
        return Err(ProfileError::MdctSchemaUnsupported {
            schema: payload
                .get("schema")
                .and_then(serde_json::Value::as_str)
                .unwrap_or("")
                .to_string(),
        });
    }
    let profiles = payload
        .get("profiles")
        .and_then(serde_json::Value::as_object)
        .ok_or(ProfileError::MdctProfilesMalformed)?;

    let mut result = BTreeMap::new();
    for (key, profile) in profiles {
        let profile_map = profile
            .as_object()
            .ok_or_else(|| ProfileError::MdctProfileMalformed { key: key.clone() })?;
        let n = key
            .parse::<i64>()
            .map_err(|_| ProfileError::MdctProfileMalformed { key: key.clone() })?;
        let expected_words = n + n / 4;
        let encoded = profile_map
            .get("little_endian_f32_base64")
            .and_then(serde_json::Value::as_str)
            .unwrap_or("");
        let raw = base64_decode(encoded).map_err(|_| ProfileError::MdctBase64Malformed { n })?;
        let word_count_ok = profile_map
            .get("word_count")
            .and_then(serde_json::Value::as_i64)
            == Some(expected_words);
        if !word_count_ok || raw.len() != 4 * expected_words as usize {
            return Err(ProfileError::MdctWordCountMismatch { n });
        }
        let mut trig = Vec::with_capacity(expected_words as usize);
        for chunk in raw.chunks_exact(4) {
            trig.push(f32::from_le_bytes(chunk.try_into().expect("4 bytes")));
        }
        let look = make_mdct_look(n, Some(&trig)).map_err(ProfileError::Analysis)?;
        result.insert(n, look);
    }

    let required: BTreeSet<i64> = [128, 256, 512, 1024, 2048].into_iter().collect();
    let got: BTreeSet<i64> = result.keys().copied().collect();
    if got != required {
        return Err(ProfileError::MdctProfileSetIncomplete);
    }
    Ok(result)
}

/// Standard-alphabet base64 decoder matching `base64.b64decode(s, validate=True)`:
/// rejects non-alphabet characters, misplaced padding, and lengths not
/// divisible by four.
pub(crate) fn base64_decode(input: &str) -> Result<Vec<u8>, ()> {
    fn decode_char(byte: u8) -> Option<u8> {
        match byte {
            b'A'..=b'Z' => Some(byte - b'A'),
            b'a'..=b'z' => Some(byte - b'a' + 26),
            b'0'..=b'9' => Some(byte - b'0' + 52),
            b'+' => Some(62),
            b'/' => Some(63),
            _ => None,
        }
    }

    let bytes = input.as_bytes();
    if !bytes.len().is_multiple_of(4) {
        return Err(());
    }
    // Padding '=' must be trailing only, at most two.
    let first_pad = bytes.iter().position(|&b| b == b'=');
    if let Some(first_pad) = first_pad {
        if bytes[first_pad + 1..].iter().any(|&b| b != b'=') {
            return Err(());
        }
        if 4 - (first_pad % 4) > 2 {
            return Err(());
        }
    }

    let mut out = Vec::with_capacity(bytes.len() * 3 / 4);
    for chunk in bytes.chunks_exact(4) {
        let mut values = [0u32; 4];
        for (slot, &byte) in chunk.iter().enumerate() {
            values[slot] = if byte == b'=' {
                0
            } else {
                decode_char(byte).ok_or(())? as u32
            };
        }
        let combined = (values[0] << 18) | (values[1] << 12) | (values[2] << 6) | values[3];
        let pad = chunk.iter().filter(|&&b| b == b'=').count();
        for i in 0..(3 - pad) {
            out.push(((combined >> (16 - 8 * i)) & 0xFF) as u8);
        }
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn base64_roundtrip_and_rejections() {
        assert_eq!(
            base64_decode("YWJjZA==").unwrap(),
            vec![b'a', b'b', b'c', b'd']
        );
        assert_eq!(base64_decode("YWJj").unwrap(), vec![b'a', b'b', b'c']);
        // "ab==" -> single byte 0x69, same as Python b64decode
        assert_eq!(base64_decode("ab==").unwrap(), vec![0x69]);
        assert!(base64_decode("YWJjZ").is_err()); // length % 4 != 0
        assert!(base64_decode("YWJ_ZA==").is_err()); // invalid character
        assert!(base64_decode("==YWJj").is_err()); // leading padding
    }
}
