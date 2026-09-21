//! The 2ch/48000 profile: its setup packet, codebooks, and quality curves are
//! registered from the paired build together with its psychoacoustic
//! calibration. The setup and encoder resources are complete.
//!
//! The bundle comes from [`wem_profiles::bundle_for_selection`] — a structured
//! selection resolved against the compiled-in profile bundle, never a profile
//! name or a profile tree.

use wem_profiles::selection::{WwiseProfile, WwiseVersion};
use wem_profiles::{
    bundle_for_selection, embedded_registry, load_quality_curves, normalize_quality_factor,
};

const TWO_CHANNEL_NAME: &str = "wwise2013-2ch-48000";
/// Setup packet SHA-256 (215-byte paired 2ch/48k setup).
const TWO_CHANNEL_SETUP_SHA: &str =
    "894a545ca48993bb0e5b768b1a367fd4475f806658b51bbcc88c8a6243849afc";

fn fixture_selection() -> WwiseProfile {
    WwiseProfile::new(WwiseVersion::Wwise2013, 6, 44_100).expect("6ch/44100 selection")
}

fn two_channel_selection() -> WwiseProfile {
    WwiseProfile::new(WwiseVersion::Wwise2013, 2, 48_000).expect("2ch/48000 selection")
}

#[test]
fn two_channel_profile_exposes_its_quality_curves() {
    // The 2ch/48000 profile ships its quality curves and setup packet.
    let bundle = bundle_for_selection(two_channel_selection()).expect("2ch bundle resolves");
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
    // The runtime registry: the 2ch/48000 profile is fully registered (setup
    // and psychoacoustics available), and a setup identity stays a
    // consequence of a selection, never an alternative selector.
    let registry = embedded_registry().expect("registry loads");
    assert_eq!(registry.len(), 2);
    let profile = registry
        .resolve_selection(two_channel_selection())
        .expect("2ch selection resolves");
    assert_eq!(profile.name(), TWO_CHANNEL_NAME);
    assert!(profile.setup_available());
    assert_eq!(profile.setup_sha256(), TWO_CHANNEL_SETUP_SHA);
    assert!(profile.pending_reason().is_none());

    // The two installed selections resolve to distinct setup digests, so a
    // setup identity is a consequence of the selection, never a selector.
    let six_channel = registry
        .resolve_selection(fixture_selection())
        .expect("6ch selection resolves");
    assert_ne!(six_channel.setup_sha256(), profile.setup_sha256());
}
