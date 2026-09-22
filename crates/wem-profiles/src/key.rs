//! Exact encoder profile identity (Python: `wwise_wem.profiles.key`).

use crate::error::ProfileError;

/// Wwise generation of the installed profiles.
pub const WWISE_GENERATION: &str = "2013.2";

/// The human label for one profile identity: its geometry plus its generation.
///
/// The label is derived from the identity and never stored — with one source
/// of profile data there is no name-keyed index, no directory and no manifest
/// field left to carry it.
pub fn profile_label(channels: i64, sample_rate: i64, generation: &str) -> String {
    format!("{channels}ch/{sample_rate}Hz/{generation}")
}

/// The complete description of one profile identity, for resolution and
/// duplicate diagnostics: the label plus the channel layout the label does not
/// carry.
///
/// Two installed profiles that share generation and geometry are told apart
/// here and nowhere else — the label alone would print them identically, which
/// is exactly the ambiguity a selection cannot resolve.
pub fn profile_description(
    channels: i64,
    sample_rate: i64,
    generation: &str,
    channel_layout: &str,
) -> String {
    format!(
        "{}/{channel_layout}",
        profile_label(channels, sample_rate, generation)
    )
}

/// Complete encoder profile identity (Python `ProfileKey`).
///
/// The identity is the key itself — generation, geometry and channel layout.
/// Nothing derived from the profile's setup packet is part of it: the packet
/// travels as bytes, so a digest stored beside it would only be checked
/// against the bytes it came from.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct ProfileKey {
    channels: i64,
    sample_rate: i64,
    generation: String,
    channel_layout: String,
}

impl ProfileKey {
    /// Construct a complete profile identity.
    pub fn new(
        channels: i64,
        sample_rate: i64,
        generation: String,
        channel_layout: String,
    ) -> Result<Self, ProfileError> {
        if channels <= 0 || sample_rate <= 0 {
            return Err(ProfileError::identity(
                "profile channels and sample rate must be positive",
            ));
        }
        for (field, value) in [
            ("generation", &generation),
            ("channel layout", &channel_layout),
        ] {
            if value.is_empty() {
                return Err(ProfileError::identity(format!(
                    "profile {field} must not be empty"
                )));
            }
        }

        Ok(Self {
            channels,
            sample_rate,
            generation,
            channel_layout,
        })
    }

    pub fn channels(&self) -> i64 {
        self.channels
    }

    pub fn sample_rate(&self) -> i64 {
        self.sample_rate
    }

    pub fn generation(&self) -> &str {
        &self.generation
    }

    pub fn channel_layout(&self) -> &str {
        &self.channel_layout
    }

    /// Complete description used in registry diagnostics.
    pub fn describe(&self) -> String {
        profile_description(
            self.channels,
            self.sample_rate,
            &self.generation,
            &self.channel_layout,
        )
    }

    /// The human label for this profile, derived from the identity.
    pub fn label(&self) -> String {
        profile_label(self.channels, self.sample_rate, &self.generation)
    }
}
