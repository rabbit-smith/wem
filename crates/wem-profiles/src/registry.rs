//! Installed Wwise Vorbis encoder profiles and exact profile registry
//! (Python: `wwise_wem_reference.profiles.artifact`).
//!
//! The caller-facing selector is one [`WwiseProfile`], resolved against the
//! profile tables compiled into this library. There is no second registry to
//! load: [`embedded_registry`] is the compiled carrier, viewed as identities.

use crate::carrier::{compiled_profiles, CompiledProfile};
use crate::error::ProfileError;
use crate::key::ProfileKey;
use crate::model::EncoderProfile;
use crate::selection::WwiseProfile;

/// Read-only exact lookup by full profile key.
///
/// Profile names stay out of every caller-facing surface: the selector is one
/// [`WwiseProfile`], resolved with [`resolve_selection`](Self::resolve_selection).
#[derive(Debug, Clone, Default)]
pub struct ProfileRegistry {
    by_key: Vec<(ProfileKey, EncoderProfile)>,
}

impl ProfileRegistry {
    /// Build a registry, rejecting duplicate keys.
    pub fn new(profiles: Vec<EncoderProfile>) -> Result<Self, ProfileError> {
        let mut by_key: Vec<(ProfileKey, EncoderProfile)> = Vec::new();
        for profile in profiles {
            let key = profile.key().clone();
            if by_key.iter().any(|(k, _)| k == &key) {
                return Err(ProfileError::RegistryDuplicateKey {
                    key: key.describe(),
                });
            }
            by_key.push((key, profile));
        }
        Ok(Self { by_key })
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
                .map(|profile| profile.key().describe())
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

    /// All profiles in stable (key) order (Python `list`).
    pub fn list(&self) -> Vec<EncoderProfile> {
        let mut profiles: Vec<EncoderProfile> =
            self.by_key.iter().map(|(_, p)| p.clone()).collect();
        profiles.sort_by(|a, b| a.key().cmp(b.key()));
        profiles
    }

    pub fn len(&self) -> usize {
        self.by_key.len()
    }

    pub fn is_empty(&self) -> bool {
        self.by_key.is_empty()
    }
}

/// Build the registry from the profiles compiled into this library.
pub fn embedded_registry() -> Result<ProfileRegistry, ProfileError> {
    let profiles = compiled_profiles()?
        .iter()
        .map(CompiledProfile::encoder_profile)
        .collect::<Result<Vec<_>, _>>()?;
    ProfileRegistry::new(profiles)
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
