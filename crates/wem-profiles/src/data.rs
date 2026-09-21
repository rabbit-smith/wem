//! Profile data-directory identity for the Rust kernel.
//!
//! The kernel's runtime encoding path never resolves profile data from the
//! filesystem or the process environment: every shipped encoder carries its
//! profile bundle at compile time ([`crate::embedded`]). [`DataDir`] is the
//! explicit development seam for the tests and tools that exercise the
//! filesystem loader against a known tree — it is constructed from a path a
//! caller passes in, and there is no ambient default.
//!
//! The Python loaders resolve resources through `importlib.resources` from
//! the `wwise_wem` package. The Rust kernel has no package namespace, so the
//! loader reads the same layout from a filesystem path given to it.
//!
//! All resource paths carried by [`crate::resources::ResourceRef`] are
//! relative to the profiles directory.

use std::path::{Path, PathBuf};

/// Handle to one profile data tree (explicit, development-time only).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DataDir {
    profiles_dir: PathBuf,
}

impl DataDir {
    /// Build a handle from an explicit profiles directory (tests, tools).
    ///
    /// There is deliberately no environment-variable or repository-layout
    /// constructor: a runtime path that depends on ambient state is exactly
    /// what the embedded bundle replaced.
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
