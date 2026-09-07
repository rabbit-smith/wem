//! Exact encoder profile identity (Python: `profiles/bundle.py::ProfileKey`).

use crate::error::ProfileError;

/// Wwise generation of the installed profiles.
pub const WWISE_GENERATION: &str = "2013.2";
/// Setup identity of the installed 2013.2 6ch/44100 profile.
pub const WWISE2013_6CH_44100_SETUP_IDENTITY: &str =
    "sha256:3ef56cbd6e6a66a5474005db05912624487faa555fb2cdfed130f606b322e4e3";

/// Exact encoder identity with a legacy two-argument geometry adapter
/// (Python `ProfileKey`).
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct ProfileKey {
    channels: i64,
    sample_rate: i64,
    generation: Option<String>,
    channel_layout: Option<String>,
    quality_setup_identity: Option<String>,
}

impl ProfileKey {
    /// Two-argument geometry constructor; (6, 44100) fills in the installed
    /// 2013.2 identity, any other geometry without a full identity is
    /// rejected (mirrors Python `__post_init__`).
    pub fn new(channels: i64, sample_rate: i64) -> Result<Self, ProfileError> {
        Self::with_identity(channels, sample_rate, None, None, None)
    }

    /// Full-identity constructor.
    #[allow(clippy::too_many_arguments)]
    pub fn with_identity(
        channels: i64,
        sample_rate: i64,
        generation: Option<String>,
        channel_layout: Option<String>,
        quality_setup_identity: Option<String>,
    ) -> Result<Self, ProfileError> {
        if channels <= 0 || sample_rate <= 0 {
            return Err(ProfileError::ProfileKeyNonPositive);
        }
        let (generation, channel_layout, quality_setup_identity) = if matches!(
            (&generation, &channel_layout, &quality_setup_identity),
            (None, None, None)
        ) && (channels, sample_rate)
            == (6, 44100)
        {
            (
                Some(WWISE_GENERATION.to_string()),
                Some("5.1".to_string()),
                Some(WWISE2013_6CH_44100_SETUP_IDENTITY.to_string()),
            )
        } else if generation.is_none()
            || channel_layout.is_none()
            || quality_setup_identity.is_none()
        {
            return Err(ProfileError::ProfileKeyIdentityIncomplete);
        } else {
            (generation, channel_layout, quality_setup_identity)
        };

        for (field, value) in [
            ("generation", &generation),
            ("channel layout", &channel_layout),
            ("quality/setup identity", &quality_setup_identity),
        ] {
            match value {
                Some(text) if !text.is_empty() => {}
                _ => return Err(ProfileError::ProfileKeyFieldEmpty { field }),
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

    pub fn generation(&self) -> Option<&str> {
        self.generation.as_deref()
    }

    pub fn channel_layout(&self) -> Option<&str> {
        self.channel_layout.as_deref()
    }

    pub fn quality_setup_identity(&self) -> Option<&str> {
        self.quality_setup_identity.as_deref()
    }

    /// Short human description used in registry diagnostics.
    pub fn describe(&self) -> String {
        format!(
            "{}ch/{}Hz/{}({})",
            self.channels,
            self.sample_rate,
            self.generation.as_deref().unwrap_or("?"),
            self.quality_setup_identity.as_deref().unwrap_or("?")
        )
    }
}
