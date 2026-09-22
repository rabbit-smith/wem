//! The 2ch/48000 profile: its setup packet, codebooks, and quality curves are
//! registered from the paired build together with its psychoacoustic
//! calibration. The setup and encoder resources are complete.
//!
//! The profile comes from `wem_profiles::compiled_profile_for_selection` — a
//! structured selection resolved against the compiled profile carrier, never a
//! profile name or a profile tree. The setup packet is pinned by comparing its
//! bytes against the seq-0 reply packet of the committed two-channel reference
//! container, never by re-typing its digest.

use wem_profiles::{compiled_profile_for_selection, embedded_registry, normalize_quality_factor};

mod common;

use common::{fixture_selection, two_channel_dir, two_channel_selection};

/// The 2ch/48000 setup packet as the committed two-channel reference container
/// carries it: the seq-0 reply packet of `tests/data/2ch-reference/tone_high.wem`
/// (`include/wem.h`: seq 0 is the setup packet, framed by its u16 LE length).
fn committed_two_channel_setup_packet() -> Vec<u8> {
    let wem = std::fs::read(two_channel_dir().join("tone_high.wem"))
        .expect("committed 2ch reference container reads");
    let parts = wem_container::load_wem_parts_bytes(&wem).expect("wem parts load");
    parts
        .setup_packet
        .expect("the committed container carries a setup packet")
        .to_vec()
}

#[test]
fn two_channel_profile_exposes_its_quality_curves() {
    // The 2ch/48000 profile ships its quality curves and setup packet.
    let compiled =
        compiled_profile_for_selection(two_channel_selection()).expect("2ch profile resolves");
    let model = compiled.encoder_profile().expect("encoder profile");
    assert!(model.setup_available());
    assert!(model.pending_reason().is_none());
    // The setup bytes are the committed container's setup bytes: identity is
    // proven against the artifact, not against a hand-written digest.
    assert_eq!(
        compiled.setup_packet().expect("setup packet"),
        committed_two_channel_setup_packet(),
        "the profile's setup packet must be the committed container's seq-0 packet"
    );

    let curves = compiled
        .quality_curves()
        .expect("curves read")
        .expect("profile registers the curves table");
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
    let registry = embedded_registry().expect("registry loads");
    assert_eq!(registry.len(), 2);
    let profile = registry
        .resolve_selection(two_channel_selection())
        .expect("2ch selection resolves");
    // The label is derived from the identity the registry resolves for the
    // selection, never a literal.
    assert_eq!(
        profile.label(),
        compiled_profile_for_selection(two_channel_selection())
            .expect("2ch profile resolves")
            .label()
    );
    assert!(profile.setup_available());
    // The setup bytes the registry's profile hands back are the committed
    // container's, so its declared digest is never re-typed as a literal.
    assert_eq!(
        profile.setup_packet().expect("setup packet"),
        committed_two_channel_setup_packet(),
        "the registry's 2ch profile must hand back the committed container's setup packet"
    );
    assert!(profile.pending_reason().is_none());

    // The two installed selections resolve to distinct setup packets and
    // distinct setup identities, so a setup identity is a consequence of the
    // selection, never a selector.
    let six_channel = registry
        .resolve_selection(fixture_selection())
        .expect("6ch selection resolves");
    assert_ne!(six_channel.setup_bytes(), profile.setup_bytes());
    assert_ne!(
        six_channel.key().quality_setup_identity(),
        profile.key().quality_setup_identity()
    );
}
