//! Shared plumbing for the `wem-capi` integration suite.
//!
//! Only what the C ABI end-to-end tests need: the repository fixture paths
//! and the two installed selections in their `WemProfile` form. Deliberately
//! no digest constants — the suite compares the bytes the callbacks received
//! against the committed reference container.
//!
//! A test binary uses a subset of these helpers, so the module opts out of
//! dead-code analysis.

#![allow(dead_code)]

use std::path::{Path, PathBuf};

use wem_capi::{WemProfile, WemVersion};
use wem_core::usecases::wav::read_pcm16;

/// The repository root (`CARGO_MANIFEST_DIR` = `<root>/crates/wem-capi`).
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

/// The committed reference container every byte-exact case compares against.
pub fn reference_wem() -> Vec<u8> {
    std::fs::read(fixtures_dir().join("reference.wem")).expect("reference.wem reads")
}

/// The fixture WAV as (interleaved little-endian i16 bytes, frames, channels).
pub fn read_fixture_pcm() -> (Vec<u8>, usize, usize) {
    let wav = read_pcm16(&fixtures_dir().join("input.wav")).expect("input.wav reads");
    // The helper hands owned bytes to its callers, so the WAV's borrow is
    // copied here.
    (
        wav.interleaved_le_bytes().to_vec(),
        wav.frames(),
        wav.channels(),
    )
}

/// The fixture's encoder configuration: Wwise 2013.2, 6ch @ 44.1kHz.
pub fn fixture_profile() -> WemProfile {
    WemProfile {
        version: WemVersion::Wwise2013,
        channels: 6,
        sample_rate: 44_100,
    }
}

/// The other installed configuration: Wwise 2013.2, 2ch @ 48kHz.
pub fn stereo_profile() -> WemProfile {
    WemProfile {
        version: WemVersion::Wwise2013,
        channels: 2,
        sample_rate: 48_000,
    }
}
