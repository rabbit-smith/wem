//! Shared plumbing for the `wem-profiles` integration suites.
//!
//! Only what more than one suite needs lives here: the repository fixture
//! paths and the two installed selections. Deliberately no digest constants —
//! a suite that cares about a profile's bytes reads the committed artifact and
//! compares against those bytes.
//!
//! Each file under `tests/` is its own test binary, and a binary uses a
//! subset of these helpers, so the module opts out of dead-code analysis.

#![allow(dead_code)]

use std::path::{Path, PathBuf};

use wem_profiles::{WwiseProfile, WwiseVersion};

/// The repository root (`CARGO_MANIFEST_DIR` = `<root>/crates/wem-profiles`).
pub fn repo_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../..")
        .canonicalize()
        .expect("repo root resolves")
}

/// The committed two-channel reference containers
/// (`repo_root/tests/data/2ch-reference`).
pub fn two_channel_reference_dir() -> PathBuf {
    repo_root().join("tests/data/2ch-reference")
}

/// The installed Wwise 2013 6ch/44100 selection.
pub fn six_selection() -> WwiseProfile {
    WwiseProfile::new(WwiseVersion::Wwise2013, 6, 44_100).expect("6ch/44100 selection")
}

/// The installed Wwise 2013 2ch/48000 selection.
pub fn two_channel_selection() -> WwiseProfile {
    WwiseProfile::new(WwiseVersion::Wwise2013, 2, 48_000).expect("2ch/48000 selection")
}
