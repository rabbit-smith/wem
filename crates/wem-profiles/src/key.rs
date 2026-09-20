//! Exact encoder profile identity (Python: `profiles/bundle.py::ProfileKey`).

use crate::error::ProfileError;

/// Wwise generation of the installed profiles.
pub const WWISE_GENERATION: &str = "2013.2";
/// Setup identity of the installed 2013.2 6ch/44100 profile.
pub const WWISE2013_6CH_44100_SETUP_IDENTITY: &str =
    "sha256:3ef56cbd6e6a66a5474005db05912624487faa555fb2cdfed130f606b322e4e3";

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

    /// Short human description used in registry diagnostics.
    pub fn describe(&self) -> String {
        format!(
            "{}ch/{}Hz/{}({})",
            self.channels, self.sample_rate, self.generation, self.quality_setup_identity
        )
    }
}
