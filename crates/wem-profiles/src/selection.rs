//! Typed Wwise generation and structured profile selection
//! (`docs/reference/domain-model.md`: *Wwise version*, *Profile selection*).
//!
//! One [`WwiseProfile`] names the encoder configuration a caller wants
//! without exposing how the kernel stores it: a Wwise generation plus the PCM
//! geometry. Profile names, profile directories and profile index/manifest
//! bytes are implementation details of this crate and never reach a
//! caller-facing signature; the runtime resolves against the bundle compiled
//! into the library, so there is no ambient path or environment variable to
//! point it at a tree.
//!
//! Both types are part of the cross-language contract: the C ABI spells them
//! `WemVersion` / `WemProfile` in `include/wem.h`, the Python binding
//! `WwiseVersion` / `WwiseProfile`. Codes and variants are stable and
//! append-only — a new Wwise generation appends a variant and a code, it never
//! renumbers or reuses one.

use crate::error::ProfileError;
use crate::key::{ProfileKey, WWISE_GENERATION};

/// One Wwise generation whose encoder configurations are selectable.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum WwiseVersion {
    /// Wwise 2013.2.
    Wwise2013,
}

impl WwiseVersion {
    /// Every selectable generation, in stable order.
    pub const ALL: [WwiseVersion; 1] = [WwiseVersion::Wwise2013];

    /// The generation the auto-selection path uses when the caller names
    /// none: the generation this revision installs.
    pub const DEFAULT: WwiseVersion = WwiseVersion::Wwise2013;

    /// Stable cross-language code (`WemVersion` in `include/wem.h`).
    pub const fn code(self) -> u32 {
        match self {
            WwiseVersion::Wwise2013 => 0,
        }
    }

    /// Decode a stable cross-language code.
    ///
    /// An unrecognized code is a caller error against this revision's
    /// contract, never a silent fallback to a default generation.
    pub fn from_code(code: u32) -> Result<Self, ProfileError> {
        Self::ALL
            .into_iter()
            .find(|version| version.code() == code)
            .ok_or(ProfileError::UnknownWwiseVersion { code })
    }

    /// The profile-key `generation` string this version resolves against.
    pub const fn generation(self) -> &'static str {
        match self {
            WwiseVersion::Wwise2013 => WWISE_GENERATION,
        }
    }

    /// The short label this version is spelled with on a command line.
    pub const fn label(self) -> &'static str {
        match self {
            WwiseVersion::Wwise2013 => "2013",
        }
    }

    /// Decode a profile-key `generation` string.
    pub fn from_generation(generation: &str) -> Result<Self, ProfileError> {
        Self::ALL
            .into_iter()
            .find(|version| version.generation() == generation)
            .ok_or_else(|| ProfileError::UnsupportedWwiseGeneration {
                generation: generation.to_string(),
            })
    }

    /// Decode a user-facing spelling: the short label or the full generation.
    pub fn parse(text: &str) -> Result<Self, ProfileError> {
        match Self::from_generation(text) {
            Ok(version) => Ok(version),
            Err(error) => Self::ALL
                .into_iter()
                .find(|version| version.label() == text)
                .ok_or(error),
        }
    }
}

/// Structured profile selection: one Wwise generation plus the PCM geometry
/// to encode.
///
/// This is the only profile selector on a caller-facing surface. It denotes
/// exactly one installed encoder profile; a selection that no installed
/// profile satisfies is rejected on resolution
/// ([`ProfileError::NoProfileForSelection`]) rather than silently substituted,
/// and a selection that more than one installed profile satisfies is rejected
/// as ambiguous instead of picking one by load order.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct WwiseProfile {
    version: WwiseVersion,
    channels: i64,
    sample_rate: i64,
}

impl WwiseProfile {
    /// Construct a selection; channels and sample rate must be positive.
    pub fn new(
        version: WwiseVersion,
        channels: i64,
        sample_rate: i64,
    ) -> Result<Self, ProfileError> {
        if channels <= 0 || sample_rate <= 0 {
            return Err(ProfileError::SelectionGeometryNonPositive);
        }
        Ok(Self {
            version,
            channels,
            sample_rate,
        })
    }

    /// The selected Wwise generation.
    pub const fn version(self) -> WwiseVersion {
        self.version
    }

    /// The selected PCM channel count.
    pub const fn channels(self) -> i64 {
        self.channels
    }

    /// The selected PCM sample rate.
    pub const fn sample_rate(self) -> i64 {
        self.sample_rate
    }

    /// Whether one installed profile identity satisfies this selection.
    ///
    /// All three fields participate: generation, channels and sample rate.
    /// Geometry alone is not an identity once more than one generation is
    /// installed with the same geometry.
    pub fn matches_key(self, key: &ProfileKey) -> bool {
        key.generation() == self.version.generation()
            && (key.channels(), key.sample_rate()) == (self.channels, self.sample_rate)
    }

    /// Human description used in diagnostics and error messages.
    pub fn describe(self) -> String {
        format!(
            "{}ch/{}Hz/{}",
            self.channels,
            self.sample_rate,
            self.version.label()
        )
    }
}
