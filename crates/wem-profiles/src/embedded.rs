//! Compile-time embedded profile bundle used by every default runtime path.
//!
//! Filesystem loading remains an explicit development seam through DataDir,
//! while shipped encoders always use this bundle and verify the same manifest
//! and resource digests as externally supplied profile bytes.

use crate::bundle::{load_profile_bundle_from_static_bytes, ProfileBundle};
use crate::error::ProfileError;

include!(concat!(env!("OUT_DIR"), "/embedded_profiles.rs"));

/// Load one installed profile directly from the encoder binary.
pub fn load_embedded_profile_bundle(profile: Option<&str>) -> Result<ProfileBundle, ProfileError> {
    load_profile_bundle_from_static_bytes(EMBEDDED_INDEX, EMBEDDED_FILES, profile, true)
}

/// Return every profile declared by the embedded index in deterministic order.
pub fn embedded_profile_names() -> Result<Vec<String>, ProfileError> {
    let index: serde_json::Value =
        serde_json::from_slice(EMBEDDED_INDEX).map_err(|_| ProfileError::InvalidJson {
            path: "index.json".to_string(),
        })?;
    let profiles = index
        .get("profiles")
        .and_then(serde_json::Value::as_object)
        .ok_or(ProfileError::IndexProfilesNotObject)?;
    Ok(profiles.keys().cloned().collect())
}
