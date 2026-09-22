//! Exact encoder profile identity (Python: `wwise_wem.profiles.key`).

use crate::error::ProfileError;

/// Wwise generation of the installed profiles.
pub const WWISE_GENERATION: &str = "2013.2";
/// Setup identity of the installed 2013.2 6ch/44100 profile.
pub const WWISE2013_6CH_44100_SETUP_IDENTITY: &str =
    "sha256:3ef56cbd6e6a66a5474005db05912624487faa555fb2cdfed130f606b322e4e3";

/// The human label for one profile identity: its geometry plus its generation.
///
/// The label is derived from the identity and never stored — with one source
/// of profile data there is no name-keyed index, no directory and no manifest
/// field left to carry it.
pub fn profile_label(channels: i64, sample_rate: i64, generation: &str) -> String {
    format!("{channels}ch/{sample_rate}Hz/{generation}")
}

/// The complete description of one profile identity, for resolution and
/// duplicate diagnostics: the label plus the two identity fields the label
/// does not carry (channel layout and setup identity).
///
/// Two installed profiles that share generation and geometry are told apart
/// here and nowhere else — the label alone would print them identically, which
/// is exactly the ambiguity a selection cannot resolve.
pub fn profile_description(
    channels: i64,
    sample_rate: i64,
    generation: &str,
    channel_layout: &str,
    quality_setup_identity: &str,
) -> String {
    format!(
        "{}/{channel_layout}({quality_setup_identity})",
        profile_label(channels, sample_rate, generation)
    )
}

/// Complete encoder profile identity (Python `ProfileKey`).
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct ProfileKey {
    channels: i64,
    sample_rate: i64,
    generation: String,
    channel_layout: String,
    quality_setup_identity: String,
}

impl ProfileKey {
    /// Construct a complete profile identity.
    pub fn new(
        channels: i64,
        sample_rate: i64,
        generation: String,
        channel_layout: String,
        quality_setup_identity: String,
    ) -> Result<Self, ProfileError> {
        if channels <= 0 || sample_rate <= 0 {
            return Err(ProfileError::ProfileKeyNonPositive);
        }
        for (field, value) in [
            ("generation", &generation),
            ("channel layout", &channel_layout),
            ("quality/setup identity", &quality_setup_identity),
        ] {
            if value.is_empty() {
                return Err(ProfileError::ProfileKeyFieldEmpty { field });
            }
        }

        Ok(Self {
            channels,
            sample_rate,
            generation,
            channel_layout,
            quality_setup_identity,
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

    pub fn quality_setup_identity(&self) -> &str {
        &self.quality_setup_identity
    }

    /// Complete description used in registry diagnostics.
    pub fn describe(&self) -> String {
        profile_description(
            self.channels,
            self.sample_rate,
            &self.generation,
            &self.channel_layout,
            &self.quality_setup_identity,
        )
    }

    /// The human label for this profile, derived from the identity.
    pub fn label(&self) -> String {
        profile_label(self.channels, self.sample_rate, &self.generation)
    }
}
