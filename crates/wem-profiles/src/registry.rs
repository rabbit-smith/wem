//! Installed Wwise Vorbis encoder profiles and exact profile registry
//! (Python: `profiles/registry.py`).
//!
//! Rust callers build the registry lazily from a caller-provided [`DataDir`].

use crate::bundle::load_profile_bundle;
use crate::data::DataDir;
use crate::error::ProfileError;
use crate::key::{ProfileKey, WWISE_GENERATION};
use crate::model::EncoderProfile;

/// Read-only exact lookup by full profile key or stable profile name.
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

    /// Lookup by name (Python `get` with a string identity).
    pub fn get_by_name(&self, name: &str) -> Option<&EncoderProfile> {
        self.by_name.iter().find(|(n, _)| n == name).map(|(_, p)| p)
    }

    /// Resolve a complete profile identity (Python `resolve` with a key).
    pub fn resolve_key(&self, key: &ProfileKey) -> Result<&EncoderProfile, ProfileError> {
        self.get_by_key(key)
            .ok_or_else(|| ProfileError::UnknownProfileKey {
                key: key.describe(),
            })
    }

    /// Resolve from WAV geometry (Python `resolve` with channels/sample_rate).
    pub fn resolve_geometry(
        &self,
        channels: i64,
        sample_rate: i64,
    ) -> Result<&EncoderProfile, ProfileError> {
        let matches: Vec<&EncoderProfile> = self
            .by_key
            .iter()
            .filter(|(k, _)| (k.channels(), k.sample_rate()) == (channels, sample_rate))
            .map(|(_, p)| p)
            .collect();
        if matches.is_empty() {
            let installed = self
                .list()
                .iter()
                .map(|p| format!("{}ch/{}Hz", p.channels(), p.sample_rate()))
                .collect::<Vec<_>>()
                .join(", ");
            return Err(ProfileError::NoProfileForGeometry {
                channels,
                sample_rate,
                installed,
            });
        }
        if matches.len() != 1 {
            let names = matches
                .iter()
                .map(|p| p.name().to_string())
                .collect::<Vec<_>>()
                .join(", ");
            return Err(ProfileError::AmbiguousProfileGeometry {
                channels,
                sample_rate,
                names,
            });
        }
        Ok(matches[0])
    }

    /// Resolve a complete profile identity carried by a template setup
    /// (Python `resolve_setup`).
    pub fn resolve_setup(
        &self,
        channels: i64,
        sample_rate: i64,
        setup_sha256: &str,
    ) -> Result<&EncoderProfile, ProfileError> {
        let setup_sha256 = setup_sha256.to_lowercase();
        // Draft profiles carry an empty setup digest; an empty digest can
        // never resolve to anything (never to the draft itself).
        if setup_sha256.is_empty() {
            return Err(ProfileError::TemplateSetupNoInstalledProfile {
                setup_sha256: String::new(),
                channels,
                sample_rate,
                installed: "none".to_string(),
            });
        }
        let matches: Vec<&EncoderProfile> = self
            .by_key
            .iter()
            .filter(|(k, _)| (k.channels(), k.sample_rate()) == (channels, sample_rate))
            .map(|(_, p)| p)
            .collect();
        let selected: Vec<&EncoderProfile> = matches
            .iter()
            .filter(|p| p.setup_sha256() == setup_sha256)
            .copied()
            .collect();
        if selected.len() == 1 {
            return Ok(selected[0]);
        }
        let installed = if matches.is_empty() {
            "none".to_string()
        } else {
            matches
                .iter()
                .map(|p| format!("{}({})", p.name(), p.setup_sha256()))
                .collect::<Vec<_>>()
                .join(", ")
        };
        Err(ProfileError::TemplateSetupNoInstalledProfile {
            setup_sha256: setup_sha256.clone(),
            channels,
            sample_rate,
            installed,
        })
    }

    /// All profiles sorted by name (Python `list`).
    pub fn list(&self) -> Vec<EncoderProfile> {
        let mut profiles: Vec<EncoderProfile> =
            self.by_name.iter().map(|(_, p)| p.clone()).collect();
        profiles.sort_by(|a, b| a.name().cmp(b.name()));
        profiles
    }

    /// Profiles indexed by name (Python `profiles_by_name`).
    pub fn profiles_by_name(&self) -> Vec<(&str, &EncoderProfile)> {
        self.by_name.iter().map(|(n, p)| (n.as_str(), p)).collect()
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

/// Load one installed WEM profile by stable name
/// (Python `load_wem_profile`).
pub fn load_wem_profile(name: &str) -> Result<EncoderProfile, ProfileError> {
    load_wem_profile_quality(name, None)
}

/// Load one installed WEM profile, optionally bound to a quality factor
/// (Python `load_wem_profile(name, quality=...)`).
///
/// With `quality` omitted the registry's cached instance is returned
/// exactly as before; with a quality value a copy of the profile carrying
/// that quality is returned (the registered profile is never mutated).
pub fn load_wem_profile_quality(
    name: &str,
    quality: Option<f64>,
) -> Result<EncoderProfile, ProfileError> {
    let data = DataDir::from_env()?;
    let registry = installed_registry(&data)?;
    let available = registry
        .list()
        .iter()
        .map(|p| p.name().to_string())
        .collect::<Vec<_>>()
        .join(",");
    let profile =
        registry
            .get_by_name(name)
            .cloned()
            .ok_or_else(|| ProfileError::UnknownProfile {
                name: name.to_string(),
                available,
            })?;
    match quality {
        None => Ok(profile),
        Some(quality) => profile.with_quality(quality),
    }
}

/// Resolve an installed exact profile from WAV geometry
/// (Python `resolve_wem_profile`).
pub fn resolve_wem_profile(
    channels: i64,
    sample_rate: i64,
) -> Result<EncoderProfile, ProfileError> {
    resolve_wem_profile_quality(channels, sample_rate, None)
}

/// Resolve an installed exact profile from WAV geometry, optionally bound
/// to a quality factor (Python `resolve_wem_profile(..., quality=...)`).
pub fn resolve_wem_profile_quality(
    channels: i64,
    sample_rate: i64,
    quality: Option<f64>,
) -> Result<EncoderProfile, ProfileError> {
    let data = DataDir::from_env()?;
    let registry = installed_registry(&data)?;
    let profile = registry.resolve_geometry(channels, sample_rate)?;
    match quality {
        None => Ok(profile.clone()),
        Some(quality) => profile.clone().with_quality(quality),
    }
}

/// Generation string used in registry diagnostics.
pub const GENERATION: &str = WWISE_GENERATION;
