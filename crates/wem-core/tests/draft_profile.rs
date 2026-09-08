//! Draft profile (2ch/48000) loading: the setup-pending profile ships its
//! static data (quality curves from the profile descriptor) even though its
//! setup packet awaits corpus export. The registry lists it with
//! `setup_available == false` and the manifest-declared pending reason.

use wem_profiles::{
    load_profile_bundle, load_quality_curves, normalize_quality_factor, DataDir,
};

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
fn installed_draft_profile_exposes_its_quality_curves() {
    // The draft profile (2ch/48000) ships its quality-curves control points
    // from static data even though its setup awaits corpus export.
    let bundle = load_profile_bundle(&data_dir(), Some("wwise2013-2ch-48000"), false)
        .expect("draft bundle loads");
    assert!(!bundle.setup_available());
    assert!(bundle.pending_reason().is_some());

    let curves_ref = bundle
        .runtime_manifest()
        .resources()
        .iter()
        .find(|(name, _)| name == "analysis.quality-curves")
        .expect("draft registers the curves resource")
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
fn draft_profile_lists_in_the_registry_as_setup_pending() {
    let registry =
        wem_profiles::installed_registry(&data_dir()).expect("registry loads");
    assert_eq!(registry.len(), 2);
    let draft = registry
        .resolve_geometry(2, 48000)
        .expect("draft geometry resolves");
    assert_eq!(draft.name(), "wwise2013-2ch-48000");
    assert!(!draft.setup_available());
    assert_eq!(draft.setup_sha256(), "");
    assert!(draft.pending_reason().is_some());
    // The draft carries no setup digest: no digest resolves to it.
    assert!(registry.resolve_setup(2, 48000, "").is_err());
}
