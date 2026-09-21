//! Structured profile selection contract (`WwiseVersion` / `WwiseProfile`).
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
use wem_profiles::ProfileRegistry;

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
    // A profile name is not a version label: this literal is the input under
    // rejection, not a way to mean "the 6ch profile".
    assert!(WwiseVersion::parse("wwise2013-6ch-44100").is_err());
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

    // Every name compared below is read off the tree the selection resolves
    // to — with two entry points that must agree on it, never a literal.
    let six_channel = registry
        .resolve_selection(six_selection())
        .expect("6ch resolves");
    assert!(six_channel.setup_available());
    let two_channel = registry
        .resolve_selection(two_channel_selection())
        .expect("2ch resolves");
    assert!(two_channel.setup_available());

    assert_eq!(
        six_channel.name(),
        wem_profiles::resolve_wem_profile_selection(six_selection())
            .expect("6ch resolves")
            .name()
    );
    assert_eq!(
        two_channel.name(),
        wem_profiles::resolve_wem_profile_selection(two_channel_selection())
            .expect("2ch resolves")
            .name()
    );
    assert_ne!(six_channel.name(), two_channel.name());
}

#[test]
fn the_free_resolvers_agree_with_the_registry() {
    for selection in [six_selection(), two_channel_selection()] {
        let resolved = wem_profiles::resolve_wem_profile_selection(selection).expect("resolves");
        let bundle = wem_profiles::bundle_for_selection(selection).expect("bundle resolves");
        // The name is the one the tree resolves for this selection; the two
        // intake paths must not disagree on it.
        assert_eq!(resolved.name(), bundle.name());
        assert_eq!(resolved.quality(), None);

        // The quality-bound form is an additive copy, never a mutation.
        let bound = wem_profiles::resolve_wem_profile_selection_quality(selection, Some(4.0))
            .expect("quality copy");
        assert_eq!(bound.name(), resolved.name());
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
fn the_bundle_intake_resolves_what_the_registry_resolves() {
    // The one public way to a bundle takes a structured selection and no
    // name, path or bytes; it must agree with the registry's own exactly-one
    // resolution for every installed configuration.
    let registry = wem_profiles::embedded_registry().expect("embedded registry");
    for selection in [six_selection(), two_channel_selection()] {
        let resolved = registry
            .resolve_selection(selection)
            .expect("selection resolves");
        let bundle = wem_profiles::bundle_for_selection(selection).expect("bundle resolves");
        assert_eq!(bundle.key(), resolved.key());
        assert_eq!(bundle.name(), resolved.name());
        assert_eq!(
            bundle.setup().expect("setup ref").sha256(),
            resolved.setup_sha256()
        );
        assert_eq!(bundle.container_metadata(), resolved.container_metadata());
    }
}

#[test]
fn an_unsatisfiable_selection_has_no_bundle_either() {
    // The bundle intake is the same exactly-one rule: an unsatisfiable
    // selection is an error, never a first-match pick.
    let selection =
        WwiseProfile::new(WwiseVersion::Wwise2013, 3, 44_100).expect("positive geometry");
    match wem_profiles::bundle_for_selection(selection).unwrap_err() {
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
    // The twin is a synthetic profile derived from the installed one: its name
    // is the installed name plus a suffix, so no profile name is typed here.
    let installed_name = base.name().to_string();
    let twin_name = format!("{installed_name}-twin");
    let twin_key = ProfileKey::new(
        base.channels(),
        base.sample_rate(),
        base.key().generation().to_string(),
        "5.1-twin".into(),
        base.key().quality_setup_identity().to_string(),
    )
    .expect("twin key");
    let twin = EncoderProfile::new(
        twin_name.clone(),
        twin_key,
        base.setup_path().cloned(),
        base.setup_sha256().to_string(),
        None,
        base.block_sizes(),
        base.container_metadata().clone(),
        true,
        None,
    )
    .expect("twin profile");

    let registry =
        ProfileRegistry::new(vec![base, twin]).expect("distinct keys are not duplicates");
    assert_eq!(registry.len(), 2);
    match registry.resolve_selection(six_selection()).unwrap_err() {
        ProfileError::AmbiguousProfileSelection { names, .. } => {
            assert!(names.contains(&installed_name), "{names}");
            assert!(names.contains(&twin_name), "{names}");
        }
        other => panic!("expected AmbiguousProfileSelection, got {other:?}"),
    }
}
