//! The 2ch/48000 profile: its setup packet, codebooks, and quality curves are
//! registered from the paired build together with its psychoacoustic
//! calibration. The setup and encoder resources are complete.

use wem_profiles::{load_profile_bundle, load_quality_curves, normalize_quality_factor, DataDir};

const TWO_CHANNEL_NAME: &str = "wwise2013-2ch-48000";
/// Setup packet SHA-256 (215-byte paired 2ch/48k setup).
const TWO_CHANNEL_SETUP_SHA: &str =
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
fn two_channel_profile_exposes_its_quality_curves() {
    // The 2ch/48000 profile ships its quality curves and setup packet.
    let bundle =
        load_profile_bundle(&data_dir(), Some(TWO_CHANNEL_NAME), false).expect("2ch bundle loads");
    assert!(bundle.setup_available());
    assert!(bundle.pending_reason().is_none());
    assert_eq!(
        bundle.setup().expect("setup ref").sha256(),
        TWO_CHANNEL_SETUP_SHA
    );
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
fn two_channel_profile_lists_in_the_registry_as_setup_available() {
    let registry = wem_profiles::installed_registry(&data_dir()).expect("registry loads");
    assert_eq!(registry.len(), 2);
    let profile = registry
        .resolve_geometry(2, 48000)
        .expect("2ch geometry resolves");
    assert_eq!(profile.name(), TWO_CHANNEL_NAME);
    assert!(profile.setup_available());
    assert_eq!(profile.setup_sha256(), TWO_CHANNEL_SETUP_SHA);
    assert!(profile.pending_reason().is_none());
    // The setup digest is a stable resolution key.
    assert_eq!(
        registry
            .resolve_setup(2, 48000, TWO_CHANNEL_SETUP_SHA)
            .expect("2ch resolves by setup digest")
            .name(),
        TWO_CHANNEL_NAME
    );
    // An empty digest does not match a registered profile.
    assert!(registry.resolve_setup(2, 48000, "").is_err());
}
