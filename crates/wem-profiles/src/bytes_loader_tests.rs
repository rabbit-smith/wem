//! In-crate in-memory loader suite: index -> manifest -> verified resources
//! from bytes, with the quality-curves assembly wiring on top.
//!
//! The bytes entry point (`load_profile_bundle_from_bytes`) is crate-private,
//! so these cases cannot be integration tests any more — an integration test
//! is a separate crate and cannot see `pub(crate)` items. Every assertion
//! moved here unchanged; the filesystem twin of this suite lives in
//! `src/loader_tests.rs`.
//!
//! Nothing here is part of the public surface: the module is compiled only
//! for `cfg(test)` and never reaches `cargo doc`.

#![allow(clippy::excessive_precision)]

use std::path::Path;

use sha2::{Digest, Sha256};

use crate::assembly::assemble_encoder_profile_resources;
use crate::bundle::load_profile_bundle_from_bytes;
use crate::bundle::ProfileBundle;
use crate::error::ProfileError;
use crate::selection::{WwiseProfile, WwiseVersion};

/// The installed 6ch/44100 profile's index key.
///
/// This suite addresses profiles by key on purpose — key addressing *is* its
/// subject — so the installed key is spelled exactly once, here, instead of
/// being re-typed at every loader call.
const SIX_CHANNEL_INDEX_KEY: &str = "wwise2013-6ch-44100";

/// The index key of the 6ch/44100 manifest document inside the profile tree.
fn six_channel_manifest_key() -> String {
    format!("{SIX_CHANNEL_INDEX_KEY}/manifest.json")
}

/// The fixture profile selection: the installed Wwise 2013 6ch/44100
/// configuration.
fn fixture_selection() -> WwiseProfile {
    WwiseProfile::new(WwiseVersion::Wwise2013, 6, 44_100).expect("6ch/44100 selection")
}

/// The installed Wwise 2013 2ch/48000 configuration.
fn two_channel_selection() -> WwiseProfile {
    WwiseProfile::new(WwiseVersion::Wwise2013, 2, 48_000).expect("2ch/48000 selection")
}

/// The profiles tree the Rust kernel resolves from the repo layout.
fn profiles_dir() -> std::path::PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../..")
        .join("src/wwise_wem/data/profiles")
        .canonicalize()
        .expect("profiles directory resolves")
}

fn sha256_hex(bytes: &[u8]) -> String {
    let digest = Sha256::digest(bytes);
    let mut out = String::with_capacity(64);
    for byte in digest {
        out.push_str(&format!("{:02x}", byte));
    }
    out
}

/// Read every file of the profile tree as (profiles-dir-relative POSIX
/// path, bytes) pairs plus the raw index document — the exact input the
/// bytes loader expects (native fs read, bytes loader only).
fn read_profile_bytes_bundle() -> (Vec<u8>, Vec<(String, Vec<u8>)>) {
    let dir = profiles_dir();
    let index = std::fs::read(dir.join("index.json")).expect("index.json reads");

    let mut files: Vec<(String, Vec<u8>)> = Vec::new();
    fn walk(dir: &Path, cur: &Path, out: &mut Vec<(String, Vec<u8>)>) {
        for entry in std::fs::read_dir(cur).expect("directory reads") {
            let entry = entry.expect("directory entry");
            let path = entry.path();
            if path.is_dir() {
                walk(dir, &path, out);
            } else if path.file_name() != Some("index.json".as_ref()) {
                let rel = path.strip_prefix(dir).expect("path under profiles dir");
                // Canonical POSIX keys: the bytes contract is platform-
                // independent (no OS separator may reach the kernel).
                out.push((
                    rel.to_string_lossy().replace('\\', "/"),
                    std::fs::read(&path).expect("resource file reads"),
                ));
            }
        }
    }
    walk(&dir, &dir, &mut files);
    assert!(!files.is_empty(), "profile tree is empty");
    (index, files)
}

// ---------------------------------------------------------------------------
// 1. The bytes bundle resolves the same identity as the installed selection
// ---------------------------------------------------------------------------

#[test]
fn bytes_bundle_matches_the_installed_selection() {
    let (index, files) = read_profile_bytes_bundle();
    let from_bytes = load_profile_bundle_from_bytes(&index, files, None, true)
        .expect("bytes bundle verifies")
        .to_encoder_profile()
        .expect("bytes bundle profile");
    let installed =
        crate::resolve_wem_profile_selection(fixture_selection()).expect("6ch resolves");

    // One identity, one setup packet: the bytes assembly and the installed
    // selection agree on what the 6ch/44100 configuration is. The name is the
    // one the selection resolves to, and the setup identity is the one the
    // crate declares (`key::WWISE2013_6CH_44100_SETUP_IDENTITY`), never a
    // digest re-typed here.
    assert_eq!(from_bytes.name(), installed.name());
    assert_eq!(
        from_bytes.key().quality_setup_identity(),
        crate::WWISE2013_6CH_44100_SETUP_IDENTITY
    );
    assert_eq!(from_bytes.key(), installed.key());
    assert_eq!(from_bytes.setup_sha256(), installed.setup_sha256());

    // The filesystem loader resolves the same identity from the same tree.
    let data = crate::data::DataDir::from_profiles_dir(profiles_dir());
    let from_fs = crate::bundle::load_profile_bundle(&data, Some(SIX_CHANNEL_INDEX_KEY), true)
        .expect("fs bundle verifies")
        .to_encoder_profile()
        .expect("fs bundle profile");
    assert_eq!(from_fs.key(), from_bytes.key());
    assert_eq!(from_fs.name(), from_bytes.name());
    // Byte equality across the two backends is what a shared digest would
    // only restate.
    assert_eq!(
        from_fs.setup_packet().expect("fs setup packet"),
        from_bytes.setup_packet().expect("bytes setup packet"),
        "the filesystem loader and the bytes loader must hand back the same setup packet"
    );
}

#[test]
fn bytes_bundle_carries_the_two_channel_profile_identity() {
    let (index, files) = read_profile_bytes_bundle();
    let installed = crate::resolve_wem_profile_selection(two_channel_selection())
        .expect("2ch selection resolves");
    // The index key is the name the selection resolves to: addressed by key,
    // never by a name typed into the test.
    let from_bytes = load_profile_bundle_from_bytes(&index, files, Some(installed.name()), true)
        .expect("named 2ch bytes bundle verifies")
        .to_encoder_profile()
        .expect("2ch bytes bundle profile");

    assert_eq!(from_bytes.name(), installed.name());
    assert_eq!(from_bytes.key(), installed.key());
    assert_eq!(from_bytes.setup_sha256(), installed.setup_sha256());
    // Byte equality against the compiled-in bundle's setup packet is the
    // claim the setup digest only restated.
    assert_eq!(
        from_bytes.setup_packet().expect("bytes setup packet"),
        crate::bundle_for_selection(two_channel_selection())
            .expect("embedded 2ch bundle resolves")
            .setup_packet()
            .expect("embedded setup packet"),
        "the bytes loader must hand back the compiled-in 2ch setup packet"
    );
}

// ---------------------------------------------------------------------------
// 2. Rejection parity: the bytes loader rejects what the fs loader rejects
//    (same validator; zero drift).
// ---------------------------------------------------------------------------

#[test]
fn tampered_resource_bytes_are_rejected_with_sha_mismatch() {
    let (index, files) = read_profile_bytes_bundle();
    // The untouched bytes bundle verifies cleanly through the shared core.
    let _ok = load_profile_bundle_from_bytes(&index, files.clone(), None, true)
        .expect("untouched bytes bundle verifies");

    // Flip one byte of the vorbis.setup payload: the shared validator must
    // reject it exactly like the filesystem loader would.
    let mut tampered = files.clone();
    for (path, bytes) in &mut tampered {
        if path.ends_with("vorbis/setup.bin") {
            bytes[10] ^= 0x01;
        }
    }
    let err = load_profile_bundle_from_bytes(&index, tampered, None, true)
        .expect_err("tampered bytes must fail");
    assert!(
        matches!(err, ProfileError::ShaMismatch { .. }),
        "expected ShaMismatch, got {err:?}"
    );
}

#[test]
fn missing_resource_bytes_are_rejected() {
    let (index, files) = read_profile_bytes_bundle();
    // Drop the t97 codebook table from the bytes bundle.
    let incomplete: Vec<(String, Vec<u8>)> = files
        .into_iter()
        .filter(|(path, _)| !path.ends_with("vorbis/codebooks/t97.json"))
        .collect();
    let err = load_profile_bundle_from_bytes(&index, incomplete, None, true).unwrap_err();
    assert!(
        matches!(err, ProfileError::MissingResource { .. }),
        "expected MissingResource, got {err:?}"
    );
}

#[test]
fn unsafe_resource_paths_are_rejected_from_bytes() {
    // A manifest entry escaping the profiles directory (../) must be
    // rejected identically to the filesystem path's resource-path validator.
    //
    // The JSON tampering is done with literal text substitution on the
    // known-good documents: the manifest resource path changes and the index
    // is re-pinned to the tampered manifest's digest, so the manifest sha
    // check passes and the resource-path rejection (the drift-critical
    // condition) is exercised.
    let dir = profiles_dir();
    let manifest_text =
        std::fs::read_to_string(dir.join(six_channel_manifest_key())).expect("manifest reads");
    let index_text = std::fs::read_to_string(dir.join("index.json")).expect("index reads");

    let old_path_field = "\"path\": \"vorbis/setup.bin\"";
    let new_path_field = "\"path\": \"../escape/attempts/setup.bin\"";
    assert!(
        manifest_text.contains(old_path_field),
        "fixture manifest layout changed"
    );
    let tampered_manifest_bytes = manifest_text
        .replace(old_path_field, new_path_field)
        .into_bytes();

    let tampered_sha = crate::resources::hex(Sha256::digest(&tampered_manifest_bytes));
    // The index pins the manifest digest: locate the current sha256 value in
    // the index text (64 hex digits after the sole manifest sha field) and
    // re-pin it to the tampered document.
    let sha_field_prefix = "\"sha256\": \"";
    let sha_start = index_text
        .find(sha_field_prefix)
        .expect("index sha field present")
        + sha_field_prefix.len();
    let current_index_sha = &index_text[sha_start..sha_start + 64];
    assert!(
        current_index_sha.bytes().all(|b| b.is_ascii_hexdigit()),
        "fixture index layout changed"
    );
    let tampered_index_bytes = index_text
        .replace(current_index_sha, &tampered_sha)
        .into_bytes();

    let files: Vec<(String, Vec<u8>)> = vec![(six_channel_manifest_key(), tampered_manifest_bytes)];
    let err =
        load_profile_bundle_from_bytes(&tampered_index_bytes, files, None, false).unwrap_err();
    assert!(
        matches!(
            err,
            ProfileError::UnsafePath { .. } | ProfileError::ResourcePathMissing { .. }
        ),
        "expected a path rejection, got {err:?}"
    );
}

#[test]
fn bytes_loader_refuses_unknown_profiles() {
    let (index, files) = read_profile_bytes_bundle();
    // Unknown profile name in the index selection.
    let err = load_profile_bundle_from_bytes(&index, files, Some("nope"), false).unwrap_err();
    assert!(
        matches!(err, ProfileError::ProfileNotInIndex { ref profile } if profile == "nope"),
        "expected ProfileNotInIndex, got {err:?}"
    );
}

// ---------------------------------------------------------------------------
// 3. Quality-curves assembly wiring on a repackaged in-memory bundle
//    (the installed profile bytes are never modified on disk)
// ---------------------------------------------------------------------------

/// Repackage the installed 6ch bytes with a synthetic quality-curves
/// resource registered in the manifest and index (in-memory only).
fn bundle_with_curves(curves_json: &str) -> (ProfileBundle, String) {
    let (mut index_bytes, mut files) = read_profile_bytes_bundle();

    let curves_bytes = curves_json.as_bytes().to_vec();
    let curves_sha = sha256_hex(&curves_bytes);

    // Patch the manifest.
    let manifest_entry = files
        .iter_mut()
        .find(|(key, _)| *key == six_channel_manifest_key())
        .expect("6ch manifest present");
    let mut manifest: serde_json::Value =
        serde_json::from_slice(manifest_entry.1.as_slice()).expect("manifest json");
    manifest
        .as_object_mut()
        .expect("manifest object")
        .get_mut("resources")
        .and_then(serde_json::Value::as_object_mut)
        .expect("manifest resources")
        .insert(
            "analysis.quality-curves".to_string(),
            serde_json::json!({
                "path": "analysis/quality-curves.json",
                "sha256": curves_sha
            }),
        );
    let new_manifest = serde_json::to_string_pretty(&manifest)
        .expect("manifest serializes")
        .into_bytes();
    manifest_entry.1 = new_manifest.clone();

    // Register the curves file.
    files.push((
        format!("{SIX_CHANNEL_INDEX_KEY}/analysis/quality-curves.json"),
        curves_bytes,
    ));

    // Re-hash the manifest into the index.
    let new_manifest_sha = sha256_hex(&new_manifest);
    let mut index: serde_json::Value = serde_json::from_slice(&index_bytes).expect("index json");
    let index_map = index.as_object_mut().expect("index object");
    let profiles_map = index_map
        .get_mut("profiles")
        .expect("profiles object")
        .as_object_mut()
        .expect("profiles map");
    let six_ch_entry = profiles_map
        .get_mut(SIX_CHANNEL_INDEX_KEY)
        .expect("6ch entry")
        .as_object_mut()
        .expect("6ch entry object");
    six_ch_entry.insert(
        "sha256".to_string(),
        serde_json::Value::String(new_manifest_sha),
    );
    index_bytes = serde_json::to_string_pretty(&index)
        .expect("index serializes")
        .into_bytes();

    let bundle = load_profile_bundle_from_bytes(&index_bytes, files, None, true)
        .expect("bytes bundle loads");
    (bundle, curves_sha)
}

fn quality_curves_json() -> String {
    // v2 shape: descriptor curve names as recorded in the paired build, with
    // the per-curve semantics map routing each onto its mechanism.
    r#"{
  "schema": "wem.quality-curves.v2",
  "interpolation": "linear-frac",
  "breakpoints": [0.0, 4.0, 8.0],
  "curves": {
    "desc29.psy_int1": [1.0, 2.0, 4.0],
    "desc30.psy_int2": [-10.0, -20.0, -40.0],
    "desc3.psy_float": [0.0, 0.0, 0.0],
    "desc31.psy_double": [0.0, 0.0, 0.0]
  },
  "semantics": {
    "desc29.psy_int1": "short.ath_offset",
    "desc30.psy_int2": "short.ath_floor",
    "desc3.psy_float": "no-op",
    "desc31.psy_double": "transient.record-index-axis"
  }
}"#
    .to_string()
}

#[test]
fn quality_none_keeps_the_historical_surface() {
    let (index_bytes, files) = read_profile_bytes_bundle();
    let bundle = load_profile_bundle_from_bytes(&index_bytes, files, None, true)
        .expect("6ch bytes bundle loads");
    let setup = bundle.setup_packet().expect("setup packet");
    let resources =
        assemble_encoder_profile_resources(&bundle, Some(&setup), None).expect("assembly succeeds");
    // Historical surface (no quality resource at all, no quality value).
    assert_eq!(
        resources.analysis.short_surface.ath_offset,
        -100.00000762939453_f32
    );
    assert_eq!(resources.analysis.quality_value, None);
    assert!(!resources.analysis.quality_extrapolated);
}

#[test]
fn quality_value_applies_the_interpolated_overrides() {
    let (bundle, _curves_sha) = bundle_with_curves(&quality_curves_json());
    let setup = bundle.setup_packet().expect("setup packet");

    // Historical: no quality.
    let base =
        assemble_encoder_profile_resources(&bundle, Some(&setup), None).expect("base assembly");
    // q = 2.0 -> qnorm = 0.20000010000000001 -> f = 0.050000025:
    //   ath_offset = (1-f)*1 + f*2 = 1.0500000250000001 -> f32 boundary
    //   ath_floor  = (1-f)*(-10) + f*(-20) = -10.500000250000001 -> f32 boundary
    let quality = assemble_encoder_profile_resources(&bundle, Some(&setup), Some(2.0))
        .expect("quality assembly");
    assert_eq!(
        quality.analysis.short_surface.ath_offset,
        1.0500000715255737_f32
    );
    assert_eq!(quality.analysis.short_surface.ath_floor, -10.5_f32);
    assert_eq!(quality.analysis.quality_value, Some(2.0));
    assert!(!quality.analysis.quality_extrapolated);
    // The base surface is untouched by the quality copy.
    assert_eq!(
        base.analysis.short_surface.ath_offset,
        -100.00000762939453_f32
    );
    assert_eq!(base.analysis.quality_value, None);
}

#[test]
fn quality_request_without_curves_is_a_configuration_error() {
    let (index_bytes, files) = read_profile_bytes_bundle();
    let bundle = load_profile_bundle_from_bytes(&index_bytes, files, None, true)
        .expect("6ch bytes bundle loads");
    let setup = bundle.setup_packet().expect("setup packet");
    let error = assemble_encoder_profile_resources(&bundle, Some(&setup), Some(4.0))
        .expect_err("quality without curves must fail");
    assert!(
        matches!(error, ProfileError::QualityCurvesResourceMissing { .. }),
        "expected QualityCurvesResourceMissing, got {error:?}"
    );
}
