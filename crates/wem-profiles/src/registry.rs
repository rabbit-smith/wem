//! Installed Wwise Vorbis encoder profiles and exact profile registry
//! (Python: `profiles/registry.py`).
//!
//! Rust callers build the registry lazily from a caller-provided [`DataDir`].

use crate::bundle::load_profile_bundle;
use crate::data::DataDir;
use crate::embedded::{embedded_profile_names, load_embedded_profile_bundle};
use crate::error::ProfileError;
use crate::key::ProfileKey;
use crate::model::EncoderProfile;
use crate::selection::WwiseProfile;

/// Read-only exact lookup by full profile key.
///
/// Profile names stay an internal addressing detail (manifests, index
/// entries, diagnostics): the caller-facing selector is one
/// [`WwiseProfile`], resolved with [`resolve_selection`](Self::resolve_selection).
#[derive(Debug, Clone, Default)]
pub struct ProfileRegistry {
    by_key: Vec<(ProfileKey, EncoderProfile)>,
    by_name: Vec<(String, EncoderProfile)>,
}

impl ProfileRegistry {
    /// Build a registry, rejecting duplicate keys or names.
    pub fn new(profiles: Vec<EncoderProfile>) -> Result<Self, ProfileError> {
        let mut by_key: Vec<(ProfileKey, EncoderProfile)> = Vec::new();
        let mut by_name: Vec<(String, EncoderProfile)> = Vec::new();
        for profile in profiles {
            let key = profile.key().clone();
            let name = profile.name().to_string();
            if by_key.iter().any(|(k, _)| k == &key) {
                return Err(ProfileError::RegistryDuplicateKey {
                    key: key.describe(),
                });
            }
            if by_name.iter().any(|(n, _)| n == &name) {
                return Err(ProfileError::RegistryDuplicateName { name: name.clone() });
            }
            by_key.push((key, profile.clone()));
            by_name.push((name, profile));
        }
        Ok(Self { by_key, by_name })
    }

    /// Lookup by exact key (Python `__getitem__`).
    pub fn get_by_key(&self, key: &ProfileKey) -> Option<&EncoderProfile> {
        self.by_key.iter().find(|(k, _)| k == key).map(|(_, p)| p)
    }

    /// Resolve a complete profile identity (Python `resolve` with a key).
    pub fn resolve_key(&self, key: &ProfileKey) -> Result<&EncoderProfile, ProfileError> {
        self.get_by_key(key)
            .ok_or_else(|| ProfileError::UnknownProfileKey {
                key: key.describe(),
            })
    }

    /// Resolve from a structured selection: generation + channels + sample
    /// rate, all three participating (the caller-facing selector).
    ///
    /// A selection never resolves by geometry alone, so it stays
    /// unambiguous once two installed profiles share a geometry across
    /// Wwise generations.
    pub fn resolve_selection(
        &self,
        selection: WwiseProfile,
    ) -> Result<&EncoderProfile, ProfileError> {
        let matches: Vec<&EncoderProfile> = self
            .by_key
            .iter()
            .filter(|(key, _)| selection.matches_key(key))
            .map(|(_, profile)| profile)
            .collect();
        if matches.is_empty() {
            return Err(ProfileError::NoProfileForSelection {
                version: selection.version().label().to_string(),
                channels: selection.channels(),
                sample_rate: selection.sample_rate(),
                installed: self.describe_installed(),
            });
        }
        if matches.len() != 1 {
            let names = matches
                .iter()
                .map(|profile| profile.name().to_string())
                .collect::<Vec<_>>()
                .join(", ");
            return Err(ProfileError::AmbiguousProfileSelection {
                version: selection.version().label().to_string(),
                channels: selection.channels(),
                sample_rate: selection.sample_rate(),
                names,
            });
        }
        Ok(matches[0])
    }

    /// Every installed profile as `{channels}ch/{rate}Hz/{generation}`, for
    /// resolution diagnostics.
    fn describe_installed(&self) -> String {
        let mut described: Vec<String> = self
            .by_key
            .iter()
            .map(|(key, _)| {
                format!(
                    "{}ch/{}Hz/{}",
                    key.channels(),
                    key.sample_rate(),
                    key.generation()
                )
            })
            .collect();
        described.sort();
        described.join(", ")
    }

    /// All profiles sorted by name (Python `list`).
    pub fn list(&self) -> Vec<EncoderProfile> {
        let mut profiles: Vec<EncoderProfile> =
            self.by_name.iter().map(|(_, p)| p.clone()).collect();
        profiles.sort_by(|a, b| a.name().cmp(b.name()));
        profiles
    }

    pub fn len(&self) -> usize {
        self.by_key.len()
    }

    pub fn is_empty(&self) -> bool {
        self.by_key.is_empty()
    }
}

/// Build the registry from every profile registered in the package index.
///
/// Complete profiles and draft profiles (setup pending corpus export) are
/// both listed; draft entries carry `setup_available == false` plus the
/// manifest-declared `pending_reason`, and never match a setup digest.
pub fn installed_registry(data: &DataDir) -> Result<ProfileRegistry, ProfileError> {
    let names = index_profile_names(data)?;
    let mut profiles = Vec::with_capacity(names.len());
    for name in names {
        let bundle = load_profile_bundle(data, Some(&name), false)?;
        profiles.push(bundle.to_encoder_profile()?);
    }
    ProfileRegistry::new(profiles)
}

/// Build the registry from profiles compiled into this library.
pub fn embedded_registry() -> Result<ProfileRegistry, ProfileError> {
    let names = embedded_profile_names()?;
    let mut profiles = Vec::with_capacity(names.len());
    for name in names {
        let bundle = load_embedded_profile_bundle(Some(&name))?;
        profiles.push(bundle.to_encoder_profile()?);
    }
    ProfileRegistry::new(profiles)
}

/// The profile names registered in the package index, in deterministic
/// (sorted) order.
fn index_profile_names(data: &DataDir) -> Result<Vec<String>, ProfileError> {
    let raw = std::fs::read(data.index_path()).map_err(|_| ProfileError::MissingResource {
        path: "index.json".to_string(),
    })?;
    let index: serde_json::Value =
        serde_json::from_slice(&raw).map_err(|_| ProfileError::InvalidJson {
            path: "index.json".to_string(),
        })?;
    let profiles = index
        .get("profiles")
        .and_then(serde_json::Value::as_object)
        .ok_or(ProfileError::IndexProfilesNotObject)?;
    Ok(profiles.keys().cloned().collect())
}

/// Resolve an installed exact profile from a structured selection — the
/// caller-facing selector (generation + channels + sample rate).
pub fn resolve_wem_profile_selection(
    selection: WwiseProfile,
) -> Result<EncoderProfile, ProfileError> {
    resolve_wem_profile_selection_quality(selection, None)
}

/// Resolve an installed exact profile from a structured selection,
/// optionally bound to a quality factor.
pub fn resolve_wem_profile_selection_quality(
    selection: WwiseProfile,
    quality: Option<f64>,
) -> Result<EncoderProfile, ProfileError> {
    let registry = embedded_registry()?;
    let profile = registry.resolve_selection(selection)?;
    match quality {
        None => Ok(profile.clone()),
        Some(quality) => profile.clone().with_quality(quality),
    }
}
