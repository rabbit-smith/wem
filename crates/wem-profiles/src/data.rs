//! Profile data-directory identity for the Rust kernel.
//!
//! The kernel's runtime encoding path never resolves profile data from the
//! filesystem or the process environment: every shipped encoder carries its
//! profile bundle at compile time ([`crate::embedded`]), and the one public
//! way to obtain that bundle is [`crate::bundle_for_selection`]. [`DataDir`]
//! is the crate-internal development seam the loader suite exercises against a
//! known tree — it is constructed from a path the test passes in, and there is
//! no ambient default.
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
pub(crate) struct DataDir {
    profiles_dir: PathBuf,
}

impl DataDir {
    /// Build a handle from an explicit profiles directory (tests, tools).
    ///
    /// There is deliberately no environment-variable or repository-layout
    /// constructor: a runtime path that depends on ambient state is exactly
    /// what the embedded bundle replaced.
    ///
    /// The crate-internal loader suite is the only caller, so the
    /// development-tree seam stays compiled in every build without being
    /// reachable from outside this crate; `allow(dead_code)` keeps the
    /// reachability lint from silencing it instead.
    #[allow(dead_code)]
    pub(crate) fn from_profiles_dir(profiles_dir: impl Into<PathBuf>) -> Self {
        Self {
            profiles_dir: profiles_dir.into(),
        }
    }

    /// The `data/profiles` directory of this tree.
    pub(crate) fn profiles_dir(&self) -> &Path {
        &self.profiles_dir
    }

    /// The package index location.
    pub(crate) fn index_path(&self) -> PathBuf {
        self.profiles_dir.join("index.json")
    }
}
