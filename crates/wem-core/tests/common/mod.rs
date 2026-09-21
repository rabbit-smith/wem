//! Shared plumbing for the `wem-core` integration suites.
//!
//! Only what more than one suite needs lives here: the repository paths and
//! the two installed selections. Deliberately no digest constants — a suite
//! that cares about bytes reads the committed artifact and compares against
//! those bytes, so a derived value is never re-typed as a literal.
//!
//! Each file under `tests/` is its own test binary, and a binary uses a
//! subset of these helpers, so the module opts out of dead-code analysis.

#![allow(dead_code)]

use std::path::{Path, PathBuf};

use wem_core::{WwiseProfile, WwiseVersion};

/// The repository root (`CARGO_MANIFEST_DIR` = `<root>/crates/wem-core`).
pub fn repo_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../..")
        .canonicalize()
        .expect("repo root resolves")
}

/// The repository fixtures directory (`repo_root/tests/fixtures`).
pub fn fixtures_dir() -> PathBuf {
    repo_root().join("tests/fixtures")
}

/// One committed fixture file, e.g. `read_fixture("reference.wem")`.
pub fn read_fixture(name: &str) -> Vec<u8> {
    std::fs::read(fixtures_dir().join(name)).expect("fixture file reads")
}

/// The committed two-channel reference containers
/// (`repo_root/tests/data/2ch-reference`).
pub fn two_channel_dir() -> PathBuf {
    repo_root().join("tests/data/2ch-reference")
}

/// The fixture profile selection: the installed Wwise 2013 6ch/44100
/// configuration.
pub fn fixture_selection() -> WwiseProfile {
    WwiseProfile::new(WwiseVersion::Wwise2013, 6, 44_100).expect("fixture selection")
}

/// The installed Wwise 2013 2ch/48000 configuration.
pub fn two_channel_selection() -> WwiseProfile {
    WwiseProfile::new(WwiseVersion::Wwise2013, 2, 48_000).expect("2ch/48000 selection")
}
