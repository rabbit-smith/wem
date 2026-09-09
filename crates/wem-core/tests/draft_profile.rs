//! The 2ch/48000 profile: its setup packet, codebooks, and quality curves are
//! all registered from the reverse-engineered paired-build sample, and its
//! psychoacoustic calibration is registered as well. The profile is fully
//! registered: setup available and encoding available, with no
//! manifest-declared pending reason.

use wem_profiles::{
    load_profile_bundle, load_quality_curves, normalize_quality_factor, DataDir,
};

const DRAFT_NAME: &str = "wwise2013-2ch-48000";
/// Setup packet SHA-256 (215-byte 2ch/48k setup extracted from the corpus WEM).
const DRAFT_SETUP_SHA: &str =
    "894a545ca48993bb0e5b768b1a367fd4475f806658b51bbcc88c8a6243849afc";

fn data_dir() -> DataDir {
    DataDir::from_profiles_dir(
        std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../..")
            .join("src/wwise_wem/data/profiles")
            .canonicalize()
            .expect("profiles directory resolves"),
    )
}

#[test]
fn twenty_two_ch_profile_exposes_its_quality_curves() {
    // The 2ch/48000 profile ships its quality-curves control points from
    // static data, and now carries its setup packet as well.
    let bundle =
        load_profile_bundle(&data_dir(), Some(DRAFT_NAME), false).expect("2ch bundle loads");
    assert!(bundle.setup_available());
    assert!(bundle.pending_reason().is_none());
    assert_eq!(bundle.setup().expect("setup ref").sha256(), DRAFT_SETUP_SHA);
    assert_eq!(bundle.setup_packet().unwrap().len(), 215);

    let curves_ref = bundle
        .runtime_manifest()
        .resources()
        .iter()
        .find(|(name, _)| name == "analysis.quality-curves")
        .expect("profile registers the curves resource")
        .1
        .clone();
    let curves = load_quality_curves(&curves_ref).expect("curves load");
    assert_eq!(curves.breakpoints().len(), 13);
    assert_eq!(curves.breakpoints()[0], -0.2);
    assert_eq!(curves.breakpoints()[12], 1.0);
    assert_eq!(curves.curve_names().count(), 4);

    // The curves evaluate on the normalized axis (shared pin: q = 4.0).
    let (values, extrapolated) = curves
        .evaluate_result(normalize_quality_factor(4.0))
        .expect("evaluate");
    assert!(!extrapolated);
    assert_eq!(values["desc31.psy_double"], 18.0000015);
}

#[test]
fn twenty_two_ch_profile_lists_in_the_registry_as_setup_available() {
    let registry =
        wem_profiles::installed_registry(&data_dir()).expect("registry loads");
    assert_eq!(registry.len(), 2);
    let profile = registry
        .resolve_geometry(2, 48000)
        .expect("2ch geometry resolves");
    assert_eq!(profile.name(), DRAFT_NAME);
    assert!(profile.setup_available());
    assert_eq!(profile.setup_sha256(), DRAFT_SETUP_SHA);
    assert!(profile.pending_reason().is_none());
    // The profile now carries a setup digest: it resolves by that digest.
    assert_eq!(
        registry
            .resolve_setup(2, 48000, DRAFT_SETUP_SHA)
            .expect("2ch resolves by setup digest")
            .name(),
        DRAFT_NAME
    );
    // An empty digest no longer matches the registered profile.
    assert!(registry.resolve_setup(2, 48000, "").is_err());
}
