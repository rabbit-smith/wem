//! Structured profile selection (`WwiseVersion` / `WwiseProfile`).
//!
//! The selector is the only profile identity a caller-facing surface exposes,
//! so its behaviour is pinned here: the version code table, the spelling
//! round-trip, and the resolution rules — including the failure mode the
//! selector exists to make impossible (a silent pick when more than one
//! installed profile satisfies the selection).

use wem_profiles::error::ProfileError;
use wem_profiles::key::ProfileKey;
use wem_profiles::model::EncoderProfile;
use wem_profiles::selection::{WwiseProfile, WwiseVersion};
use wem_profiles::{compiled_profile_for_selection, ProfileRegistry};

mod common;

use common::{six_selection, two_channel_selection};

// ---------------------------------------------------------------------------
// Version table
// ---------------------------------------------------------------------------

#[test]
fn version_codes_are_stable_and_append_only() {
    // The code is the cross-language `WemVersion` table in include/wem.h:
    // stable and append-only, exactly like the WemError values.
    assert_eq!(WwiseVersion::Wwise2013.code(), 0);
    for version in WwiseVersion::ALL {
        assert_eq!(
            WwiseVersion::from_code(version.code()).expect("code round-trips"),
            version
        );
    }
    // Every variant is in ALL (a new variant that forgets the table would
    // otherwise be unreachable from the C ABI and from Python).
    assert_eq!(WwiseVersion::ALL.len(), 1);
    assert_eq!(WwiseVersion::DEFAULT, WwiseVersion::Wwise2013);
}

#[test]
fn an_unknown_version_code_is_rejected_not_defaulted() {
    assert_eq!(
        WwiseVersion::from_code(1).unwrap_err(),
        ProfileError::UnknownWwiseVersion { code: 1 }
    );
    assert_eq!(
        WwiseVersion::from_code(u32::MAX).unwrap_err(),
        ProfileError::UnknownWwiseVersion { code: u32::MAX }
    );
}

#[test]
fn generation_and_label_spellings_round_trip() {
    // The profile-key generation and the command-line label are two
    // spellings of the same selector.
    assert_eq!(WwiseVersion::Wwise2013.generation(), "2013.2");
    assert_eq!(WwiseVersion::Wwise2013.label(), "2013");
    assert_eq!(
        WwiseVersion::from_generation("2013.2").expect("generation"),
        WwiseVersion::Wwise2013
    );
    assert_eq!(
        WwiseVersion::parse("2013").expect("short label"),
        WwiseVersion::Wwise2013
    );
    assert_eq!(
        WwiseVersion::parse("2013.2").expect("full generation"),
        WwiseVersion::Wwise2013
    );
    assert!(matches!(
        WwiseVersion::from_generation("2012.1").unwrap_err(),
        ProfileError::UnsupportedWwiseGeneration { .. }
    ));
    // A label that is neither spelling is rejected, never silently defaulted.
    assert!(WwiseVersion::parse("2014").is_err());
    assert!(WwiseVersion::parse("").is_err());
    // A profile label is not a version label: this literal is the input under
    // rejection, not a way to mean "the 6ch profile".
    assert!(WwiseVersion::parse("6ch/44100Hz/2013.2").is_err());
}

// ---------------------------------------------------------------------------
// Selection construction
// ---------------------------------------------------------------------------

#[test]
fn selection_requires_positive_geometry() {
    for (channels, sample_rate) in [(0, 44_100), (6, 0), (-6, 44_100), (6, -44_100)] {
        assert_eq!(
            WwiseProfile::new(WwiseVersion::Wwise2013, channels, sample_rate).unwrap_err(),
            ProfileError::SelectionGeometryNonPositive
        );
    }
}

#[test]
fn selection_matches_on_generation_and_geometry_together() {
    let installed =
        ProfileKey::new(6, 44_100, "2013.2".into(), "5.1".into(), "sha256:0".into()).expect("key");
    assert!(six_selection().matches_key(&installed));
    assert!(!two_channel_selection().matches_key(&installed));

    // The generation participates: the same geometry from another Wwise
    // generation is a different selection. This is the ambiguity that
    // geometry-only resolution could not express.
    let other_generation =
        ProfileKey::new(6, 44_100, "2012.1".into(), "5.1".into(), "sha256:0".into()).expect("key");
    assert!(!six_selection().matches_key(&other_generation));
}

#[test]
fn selection_describes_itself_for_diagnostics() {
    assert_eq!(six_selection().describe(), "6ch/44100Hz/2013");
    assert_eq!(two_channel_selection().describe(), "2ch/48000Hz/2013");
    assert_eq!(six_selection().version(), WwiseVersion::Wwise2013);
    assert_eq!(six_selection().channels(), 6);
    assert_eq!(six_selection().sample_rate(), 44_100);
}

// ---------------------------------------------------------------------------
// Resolution against the installed profiles
// ---------------------------------------------------------------------------

#[test]
fn both_installed_selections_resolve_by_generation_and_geometry() {
    let registry = wem_profiles::embedded_registry().expect("embedded registry");
    assert_eq!(registry.len(), 2);

    // Every label compared below is derived from the identity the selection
    // resolves to — with two entry points that must agree on it, never a
    // literal.
    let six_channel = registry
        .resolve_selection(six_selection())
        .expect("6ch resolves");
    assert!(six_channel.setup_available());
    let two_channel = registry
        .resolve_selection(two_channel_selection())
        .expect("2ch resolves");
    assert!(two_channel.setup_available());

    assert_eq!(
        six_channel.label(),
        wem_profiles::resolve_wem_profile_selection(six_selection())
            .expect("6ch resolves")
            .label()
    );
    assert_eq!(
        two_channel.label(),
        wem_profiles::resolve_wem_profile_selection(two_channel_selection())
            .expect("2ch resolves")
            .label()
    );
    assert_ne!(six_channel.label(), two_channel.label());
}

#[test]
fn the_free_resolvers_agree_with_the_registry() {
    for selection in [six_selection(), two_channel_selection()] {
        let resolved = wem_profiles::resolve_wem_profile_selection(selection).expect("resolves");
        let compiled = compiled_profile_for_selection(selection).expect("compiled profile");
        // The label is derived from the identity the selection resolves to;
        // the two resolution paths must not disagree on it.
        assert_eq!(resolved.label(), compiled.label());
        assert_eq!(resolved.quality(), None);

        // The quality-bound form is an additive copy, never a mutation.
        let bound = wem_profiles::resolve_wem_profile_selection_quality(selection, Some(4.0))
            .expect("quality copy");
        assert_eq!(bound.label(), resolved.label());
        assert_eq!(bound.quality(), Some(4.0));
        assert_eq!(
            wem_profiles::resolve_wem_profile_selection(selection)
                .expect("re-resolves")
                .quality(),
            None
        );
        assert!(
            wem_profiles::resolve_wem_profile_selection_quality(selection, Some(f64::NAN)).is_err()
        );
    }
}

#[test]
fn the_carrier_intake_resolves_what_the_registry_resolves() {
    // The one public way to a compiled profile takes a structured selection
    // and no name, path or bytes; it must agree with the registry's own
    // exactly-one resolution for every installed configuration.
    let registry = wem_profiles::embedded_registry().expect("embedded registry");
    for selection in [six_selection(), two_channel_selection()] {
        let resolved = registry
            .resolve_selection(selection)
            .expect("selection resolves");
        let compiled = compiled_profile_for_selection(selection).expect("compiled profile");
        assert_eq!(compiled.key(), resolved.key());
        assert_eq!(compiled.label(), resolved.label());
        // The setup packet the carrier hands back is the value model's own,
        // compared as bytes.
        assert_eq!(
            compiled.setup_packet().expect("setup packet"),
            resolved.setup_bytes().expect("setup packet")
        );
        assert_eq!(compiled.container_metadata(), resolved.container_metadata());
        // The setup packet the carrier hands back is the recorded one: 201
        // bytes for 6ch/44100, 215 for 2ch/48000.
        let expected_setup_len = if selection.channels() == 6 { 201 } else { 215 };
        assert_eq!(
            compiled.setup_packet().expect("setup packet").len(),
            expected_setup_len
        );
    }
}

#[test]
fn an_unsatisfiable_selection_has_no_compiled_profile_either() {
    // The carrier intake is the same exactly-one rule: an unsatisfiable
    // selection is an error, never a first-match pick.
    let selection =
        WwiseProfile::new(WwiseVersion::Wwise2013, 3, 44_100).expect("positive geometry");
    match compiled_profile_for_selection(selection).unwrap_err() {
        ProfileError::NoProfileForSelection { channels, .. } => assert_eq!(channels, 3),
        other => panic!("expected NoProfileForSelection, got {other:?}"),
    }
}

#[test]
fn an_unsatisfiable_selection_lists_what_is_installed() {
    // Installed rates on the wrong channel count, and an uninstalled rate on
    // an installed channel count: neither may fall back to a neighbour.
    for (channels, sample_rate) in [(6, 48_000), (2, 44_100), (3, 44_100), (1, 48_000)] {
        let selection =
            WwiseProfile::new(WwiseVersion::Wwise2013, channels, sample_rate).expect("selection");
        match wem_profiles::resolve_wem_profile_selection(selection).unwrap_err() {
            ProfileError::NoProfileForSelection {
                channels: reported_channels,
                sample_rate: reported_rate,
                installed,
                ..
            } => {
                assert_eq!(reported_channels, channels);
                assert_eq!(reported_rate, sample_rate);
                // Both installed configurations are named, with generation.
                assert!(installed.contains("6ch/44100Hz/2013.2"), "{installed}");
                assert!(installed.contains("2ch/48000Hz/2013.2"), "{installed}");
            }
            other => panic!("expected NoProfileForSelection, got {other:?}"),
        }
    }
}

#[test]
fn an_ambiguous_selection_is_rejected_rather_than_picked() {
    // Two installed profiles that share generation + geometry but differ in
    // channel layout are two distinct profile keys, so the registry accepts
    // both. A selection cannot tell them apart, and must say so instead of
    // returning whichever came first.
    let base = wem_profiles::embedded_registry()
        .expect("embedded registry")
        .resolve_selection(six_selection())
        .expect("6ch installs")
        .clone();
    // The twin is a synthetic profile derived from the installed one: only
    // its channel layout differs, so the two share generation and geometry —
    // which is exactly the collision a selection cannot resolve.
    let installed_key = base.key().clone();
    let installed_description = installed_key.describe();
    let twin_key = ProfileKey::new(
        base.channels(),
        base.sample_rate(),
        base.key().generation().to_string(),
        "5.1-twin".into(),
        base.key().quality_setup_identity().to_string(),
    )
    .expect("twin key");
    let twin = EncoderProfile::new(
        twin_key,
        base.setup_bytes().map(<[u8]>::to_vec),
        None,
        base.block_sizes(),
        *base.container_metadata(),
        true,
        None,
    )
    .expect("twin profile");

    let registry =
        ProfileRegistry::new(vec![base, twin]).expect("distinct keys are not duplicates");
    assert_eq!(registry.len(), 2);
    match registry.resolve_selection(six_selection()).unwrap_err() {
        ProfileError::AmbiguousProfileSelection { names, .. } => {
            // The derived label alone would print both candidates
            // identically; the ambiguity diagnostic carries the full
            // identity so the collision is readable.
            assert!(names.contains(&installed_description), "{names}");
            assert!(names.contains("5.1-twin"), "{names}");
        }
        other => panic!("expected AmbiguousProfileSelection, got {other:?}"),
    }
}
