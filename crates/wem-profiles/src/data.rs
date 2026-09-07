//! Data directory resolution for the Rust kernel.
//!
//! The Python loaders resolve resources through `importlib.resources` from
//! the `wwise_wem` package. The Rust kernel has no package namespace, so it
//! resolves the same layout from the filesystem:
//!
//! * `WEM_DATA_DIR` (when set) names the `data/profiles` directory of the
//!   active data tree, and
//! * otherwise the tree is discovered by walking up from this crate's
//!   manifest directory to a repository root containing
//!   `src/wwise_wem/data/profiles/index.json`.
//!
//! All resource paths carried by [`crate::resources::ResourceRef`] are
//! relative to the profiles directory.

use std::path::{Path, PathBuf};

use crate::error::ProfileError;

/// Handle to one installed profile data tree.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DataDir {
    profiles_dir: PathBuf,
}

impl DataDir {
    /// Resolve the active data directory from the environment or the
    /// repository layout (see module docs).
    pub fn from_env() -> Result<Self, ProfileError> {
        if let Ok(dir) = std::env::var("WEM_DATA_DIR") {
            if dir.is_empty() {
                return Err(ProfileError::DataDirNotFound);
            }
            return Ok(Self {
                profiles_dir: PathBuf::from(dir),
            });
        }
        let mut current = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
        loop {
            let candidate = current.join("src/wwise_wem/data/profiles");
            if candidate.join("index.json").is_file() {
                return Ok(Self {
                    profiles_dir: candidate,
                });
            }
            match current.parent() {
                Some(parent) => current = parent.to_path_buf(),
                None => return Err(ProfileError::DataDirNotFound),
            }
        }
    }

    /// Build a handle from an explicit profiles directory (tests, tools).
    pub fn from_profiles_dir(profiles_dir: impl Into<PathBuf>) -> Self {
        Self {
            profiles_dir: profiles_dir.into(),
        }
    }

    /// The `data/profiles` directory of this tree.
    pub fn profiles_dir(&self) -> &Path {
        &self.profiles_dir
    }

    /// The package index location.
    pub fn index_path(&self) -> PathBuf {
        self.profiles_dir.join("index.json")
    }
}
