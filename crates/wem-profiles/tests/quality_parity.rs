//! Quality mechanism: shared parity pins and end-to-end wiring.
//!
//! The expected doubles below are the shared test vectors: the Python
//! reference (reference/wwise_wem_reference/profiles/quality.py,
//! tests/unit/profiles/test_quality_curves.py) and this module must return
//! identical values bit for bit (same f64 arithmetic, same two-step
//! fractional-index form, same normalization entry).
//!
//! The end-to-end tests repackage the installed 6ch profile bytes with a
//! synthetic quality-curves resource (in-memory; the installed profile
//! bytes are never modified) and prove the assembly wiring: quality=None
//! keeps the historical surface exactly, a quality value applies the
//! interpolated overrides (through the f32 boundary), and a quality
//! request from a profile without curves is a configuration error.

#![allow(clippy::excessive_precision)]

use wem_profiles::{
    assemble_encoder_profile_resources, linear_frac, load_profile_bundle_from_bytes,
    normalize_quality_factor, ProfileError,
};

// ---------------------------------------------------------------------------
// Shared parity pins (identical vectors on the Python side)
// ---------------------------------------------------------------------------

const DRAFT_BP: [f64; 13] = [
    -0.2, -0.1, 0.0, 0.1, 0.2, 0.3, 0.4, 0.5, 0.6, 0.7, 0.8, 0.9, 1.0,
];
const DRAFT_DESC3: [f64; 13] = [
    1.0, 1.0, 1.0, 1.0, 1.0, 1.0, 1.0, 1.0, 1.0, 1.0, 1.0, 1.0, 1.0,
];
const DRAFT_DESC29: [f64; 13] = [
    -100.0, -100.0, -100.0, -100.0, -100.0, -100.0, -100.0, -105.0, -105.0, -105.0, -105.0, -110.0,
    -120.0,
];
const DRAFT_DESC30: [f64; 13] = [
    -130.0, -130.0, -130.0, -130.0, -130.0, -135.0, -140.0, -140.0, -140.0, -140.0, -140.0, -140.0,
    -150.0,
];
const DRAFT_DESC31: [f64; 13] = [
    12.9, 13.8, 14.7, 15.6, 16.5, 17.1, 18.0, 19.5, 48.0, 999.0, 999.0, 999.0, 999.0,
];

#[test]
fn normalization_pins_match_the_python_reference() {
    assert_eq!(normalize_quality_factor(0.0), 1e-7);
    assert_eq!(normalize_quality_factor(4.0), 0.4000001);
    assert_eq!(normalize_quality_factor(9.0), 0.9000001);
    assert_eq!(normalize_quality_factor(10.0), 0.9998999834060669);
    assert_eq!(normalize_quality_factor(100.0), 0.9998999834060669);
}

#[test]
fn kernel_pins_match_the_python_reference() {
    // (bp=[0.5,0.9], samples=[0,1]).
    let bp = [0.5, 0.9];
    let s = [0.0, 1.0];
    assert_eq!(linear_frac(&bp, &s, 0.1), (0.0, true));
    assert_eq!(linear_frac(&bp, &s, 0.7), (0.4999999999999999, false));
    // At the last breakpoint the N - 0.001 clamp formula applies (not the
    // stored value).
    assert_eq!(linear_frac(&bp, &s, 0.9), (0.999, false));
    assert_eq!(linear_frac(&bp, &s, 1.0), (0.999, true));
    assert_eq!(linear_frac(&bp, &s, 2.0), (0.999, true));

    // (bp=[0,4,8], samples=[10,20,30]).
    let bp = [0.0, 4.0, 8.0];
    let s = [10.0, 20.0, 30.0];
    assert_eq!(linear_frac(&bp, &s, 2.0), (15.0, false));
    assert_eq!(linear_frac(&bp, &s, 6.0), (25.0, false));
    assert_eq!(linear_frac(&bp, &s, 9.0), (29.990000000000002, true));
    assert_eq!(linear_frac(&bp, &s, 8.0), (29.990000000000002, false));
    assert_eq!(linear_frac(&bp, &s, 4.0), (20.0, false));
    assert_eq!(linear_frac(&bp, &s, 0.0), (10.0, false));

    // Below-domain clamp.
    let bp = [0.5, 0.9, 0.95];
    let s = [10.0, 20.0, 30.0];
    assert_eq!(linear_frac(&bp, &s, 0.0100001), (10.0, true));
}

#[test]
fn draft_13bp_curves_pins_match_the_python_reference() {
    // q = 4.0 -> qnorm = 0.4000001 -> the two-step fraction at i = 4.
    {
        let qnorm = normalize_quality_factor(4.0);
        assert_eq!(qnorm, 0.4000001);
        let (d3, e3) = linear_frac(&DRAFT_BP, &DRAFT_DESC3, qnorm);
        let (d29, e29) = linear_frac(&DRAFT_BP, &DRAFT_DESC29, qnorm);
        let (d30, e30) = linear_frac(&DRAFT_BP, &DRAFT_DESC30, qnorm);
        let (d31, e31) = linear_frac(&DRAFT_BP, &DRAFT_DESC31, qnorm);
        assert!(!e3 && !e29 && !e30 && !e31);
        assert_eq!(d3, 1.0);
        assert_eq!(d29, -100.000005);
        assert_eq!(d30, -140.0);
        assert_eq!(d31, 18.0000015);
    }
    // q = 10.0 -> clamped qnorm, in-domain segment (11, 12).
    {
        let qnorm = normalize_quality_factor(10.0);
        assert_eq!(qnorm, 0.9998999834060669);
        let (d29, e29) = linear_frac(&DRAFT_BP, &DRAFT_DESC29, qnorm);
        let (d30, e30) = linear_frac(&DRAFT_BP, &DRAFT_DESC30, qnorm);
        let (d31, e31) = linear_frac(&DRAFT_BP, &DRAFT_DESC31, qnorm);
        assert!(!e29 && !e30 && !e31);
        assert_eq!(d29, -119.98999834060669);
        assert_eq!(d30, -149.9899983406067);
        assert_eq!(d31, 999.0);
    }
    // q = -3.0 -> below-domain clamp, extrapolated.
    {
        let qnorm = normalize_quality_factor(-3.0);
        let (d31, e31) = linear_frac(&DRAFT_BP, &DRAFT_DESC31, qnorm);
        assert!(e31);
        assert_eq!(d31, 12.9);
    }
}

// ---------------------------------------------------------------------------
// End-to-end wiring (in-memory 6ch repackage + synthetic curves)
// ---------------------------------------------------------------------------

fn profiles_dir() -> std::path::PathBuf {
    std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../..")
        .join("src/wwise_wem/data/profiles")
        .canonicalize()
        .expect("profiles directory resolves")
}

fn sha256_hex(bytes: &[u8]) -> String {
    use sha2::{Digest, Sha256};
    let digest = Sha256::digest(bytes);
    let mut out = String::with_capacity(64);
    for byte in digest {
        out.push_str(&format!("{:02x}", byte));
    }
    out
}

/// The installed 6ch profile tree as (index bytes, files) pairs.
fn installed_6ch_bytes() -> (Vec<u8>, Vec<(String, Vec<u8>)>) {
    let dir = profiles_dir();
    let index = std::fs::read(dir.join("index.json")).expect("index reads");
    let mut files: Vec<(String, Vec<u8>)> = Vec::new();
    fn walk(base: &std::path::Path, dir: &std::path::Path, out: &mut Vec<(String, Vec<u8>)>) {
        for entry in std::fs::read_dir(dir).expect("directory reads") {
            let entry = entry.expect("directory entry");
            let path = entry.path();
            if path.is_dir() {
                walk(base, &path, out);
            } else if path.file_name() != Some("index.json".as_ref()) {
                let rel = path.strip_prefix(base).expect("under profiles dir");
                out.push((
                    rel.to_string_lossy().replace('\\', "/"),
                    std::fs::read(&path).expect("resource reads"),
                ));
            }
        }
    }
    walk(&dir, &dir, &mut files);
    (index, files)
}

/// Repackage the installed 6ch bytes with a synthetic quality-curves
/// resource registered in the manifest and index (in-memory only).
fn bundle_with_curves(curves_json: &str) -> (wem_profiles::ProfileBundle, String) {
    let (mut index_bytes, mut files) = installed_6ch_bytes();

    let curves_bytes = curves_json.as_bytes().to_vec();
    let curves_sha = sha256_hex(&curves_bytes);

    // Patch the manifest.
    let manifest_key = "wwise2013-6ch-44100/manifest.json";
    let manifest_entry = files
        .iter_mut()
        .find(|(key, _)| key == manifest_key)
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
        "wwise2013-6ch-44100/analysis/quality-curves.json".to_string(),
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
        .get_mut("wwise2013-6ch-44100")
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
    let (index_bytes, files) = installed_6ch_bytes();
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
    let (index_bytes, files) = installed_6ch_bytes();
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
