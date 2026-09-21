//! The 2ch/48000 profile: its setup packet, codebooks, and quality curves are
//! registered from the paired build together with its psychoacoustic
//! calibration. The setup and encoder resources are complete.
//!
//! The bundle comes from [`wem_profiles::bundle_for_selection`] — a structured
//! selection resolved against the compiled-in profile bundle, never a profile
//! name or a profile tree. The setup packet is pinned by comparing its bytes
//! against the committed two-channel reference container, never by re-typing
//! its digest.

use wem_profiles::{
    bundle_for_selection, embedded_registry, load_quality_curves, normalize_quality_factor,
};

mod common;

use common::{six_selection, two_channel_reference_dir, two_channel_selection};

/// The profile's setup packet must be carried verbatim, as the seq-0 reply
/// packet, by a committed two-channel reference container: `include/wem.h`
/// frames every reply packet as a u16 LE length followed by the packet bytes.
///
/// The committed container is the fixture; the framing rule is applied here
/// rather than through a container reader because the WEM container is not a
/// dependency of this crate (and must not become one just for a test).
fn assert_the_committed_container_carries(setup_packet: &[u8]) {
    let wem = std::fs::read(two_channel_reference_dir().join("tone_high.wem"))
        .expect("committed 2ch reference container reads");
    let length = u16::try_from(setup_packet.len()).expect("a reply packet length is a u16");
    let framed: Vec<u8> = length
        .to_le_bytes()
        .into_iter()
        .chain(setup_packet.iter().copied())
        .collect();
    assert!(
        wem.windows(framed.len()).any(|window| window == framed),
        "the committed 2ch reference container must frame the profile's setup packet \
         as its seq-0 reply packet"
    );
}

#[test]
fn two_channel_profile_exposes_its_quality_curves() {
    // The 2ch/48000 profile ships its quality curves and setup packet.
    let bundle = bundle_for_selection(two_channel_selection()).expect("2ch bundle resolves");
    assert!(bundle.setup_available());
    assert!(bundle.pending_reason().is_none());
    // Byte identity against the committed container is the claim; the digest
    // is a consequence of it, never a second hand-written statement.
    assert_the_committed_container_carries(&bundle.setup_packet().expect("setup packet"));

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
    // The name and the setup identity are properties read off the tree the
    // selection resolves to, never literals in the test.
    let bundle = bundle_for_selection(two_channel_selection()).expect("2ch bundle resolves");
    assert_eq!(profile.name(), bundle.name());
    assert!(profile.setup_available());
    // The setup bytes the registry's profile hands back are the committed
    // container's, so its declared digest is never re-typed as a literal.
    assert_the_committed_container_carries(&profile.setup_packet().expect("setup packet"));
    assert!(profile.pending_reason().is_none());

    // The two installed selections resolve to distinct setup digests, so a
    // setup identity is a consequence of the selection, never a selector.
    let six_channel = registry
        .resolve_selection(six_selection())
        .expect("6ch selection resolves");
    assert_ne!(six_channel.setup_sha256(), profile.setup_sha256());
}
