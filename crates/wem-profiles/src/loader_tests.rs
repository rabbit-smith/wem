//! In-crate loader suite: the filesystem development seam, the digest chain,
//! and the rejection parity that used to live in `tests/`.
//!
//! The profile intake (`DataDir`, `load_profile_bundle`, `installed_registry`,
//! `ResourceRef::new`, `normalize_resource_path`) is crate-private, so these
//! cases cannot be integration tests any more — an integration test is a
//! separate crate and cannot see `pub(crate)` items. Every assertion moved
//! here unchanged; the in-memory twin of this suite lives in
//! `src/bytes_loader_tests.rs`.
//!
//! Nothing here is part of the public surface: the module is compiled only
//! for `cfg(test)` and never reaches `cargo doc`.

use std::path::{Path, PathBuf};

use sha2::{Digest, Sha256};

use crate::bundle::load_profile_bundle;
use crate::data::DataDir;
use crate::error::ProfileError;
use crate::key::ProfileKey;
use crate::registry::installed_registry;
use crate::resources::{normalize_resource_path, ResourceRef};
use crate::selection::{WwiseProfile, WwiseVersion};

fn repo_root() -> PathBuf {
    // crates/wem-profiles -> repo root (two levels up from the manifest dir).
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .expect("crates dir")
        .parent()
        .expect("repo root")
        .to_path_buf()
}

fn data_dir() -> DataDir {
    DataDir::from_profiles_dir(repo_root().join("src/wwise_wem/data/profiles"))
}

/// The installed Wwise 2013 6ch/44100 selection.
fn six_selection() -> WwiseProfile {
    WwiseProfile::new(WwiseVersion::Wwise2013, 6, 44_100).expect("6ch/44100 selection")
}

/// The installed Wwise 2013 2ch/48000 selection.
fn two_channel_selection() -> WwiseProfile {
    WwiseProfile::new(WwiseVersion::Wwise2013, 2, 48_000).expect("2ch/48000 selection")
}

fn sha256_hex(payload: &[u8]) -> String {
    let digest = Sha256::digest(payload);
    const DIGITS: &[u8; 16] = b"0123456789abcdef";
    let mut out = String::new();
    for &b in digest.as_slice() {
        out.push(DIGITS[(b >> 4) as usize] as char);
        out.push(DIGITS[(b & 0xF) as usize] as char);
    }
    out
}

fn write(path: &Path, payload: &[u8]) {
    std::fs::write(path, payload).expect("write temp resource");
}

fn temp_tree(tag: &str) -> PathBuf {
    let dir =
        std::env::temp_dir().join(format!("wem-profiles-test-{}-{}", tag, std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(dir.join("wwise2013-6ch-44100")).expect("temp dirs");
    dir
}

/// Build a minimal valid profile tree and return the DataDir.
fn valid_tree(tag: &str) -> DataDir {
    let root = temp_tree(tag);
    let profile_dir = root.join("wwise2013-6ch-44100");

    let setup_payload = b"setup-payload";
    write(&profile_dir.join("vorbis_setup.bin"), setup_payload);
    let setup_sha = sha256_hex(setup_payload);

    let index = serde_json::json!({
        "schema": "wwise-wem.profile-index.v1",
        "default": "wwise2013-6ch-44100",
        "profiles": {
            "wwise2013-6ch-44100": {
                "manifest": "wwise2013-6ch-44100/manifest.json",
                "sha256": format!("pending-{tag}")
            }
        }
    });
    let manifest = serde_json::json!({
        "schema": "wwise-wem.profile-manifest.v1",
        "name": "wwise2013-6ch-44100",
        "key": {
            "generation": "2013.2",
            "channels": 6,
            "sample_rate": 44100,
            "channel_layout": "5.1",
            "quality_setup_identity": format!("sha256:{setup_sha}")
        },
        "block_sizes": [4, 8],
        "container_metadata": {
            "wFormatTag": 65535, "nChannels": 6, "nSamplesPerSec": 44100,
            "nAvgBytesPerSec": 0, "nBlockAlign": 0, "wBitsPerSample": 0,
            "cbSize": 48, "wReserved0": 0, "dwChannelMask": 63,
            "dwTotalPCMFrames": 0, "dwFirstAudioPacketOffset": 0,
            "dwDataPayloadSize": 0, "dwUnknown_0x24": 0, "dwSeekTableSize": 0,
            "dwVorbisDataOffset": 0, "uMaxPacketSize": 0, "uUnknown_0x32": 0,
            "dwUnknown_0x34": 0, "dwUnknown_0x38": 0, "dwUnknown_0x3C": 0,
            "uBlocksize0Pow": 2, "uBlocksize1Pow": 3
        },
        "resources": {
            "vorbis.setup": {
                "path": "vorbis_setup.bin",
                "sha256": setup_sha
            }
        }
    });
    let manifest_bytes = serde_json::to_vec_pretty(&manifest).unwrap();
    let manifest_sha = sha256_hex(&manifest_bytes);
    write(&profile_dir.join("manifest.json"), &manifest_bytes);

    let mut index = serde_json::to_value(&index).unwrap();
    index["profiles"]["wwise2013-6ch-44100"]["sha256"] = serde_json::Value::String(manifest_sha);
    write(
        &root.join("index.json"),
        &serde_json::to_vec(&index).unwrap(),
    );

    DataDir::from_profiles_dir(root)
}

// ---------------------------------------------------------------------------
// Real-asset filesystem loading: the digest chain on the shipped tree
// ---------------------------------------------------------------------------

#[test]
fn installed_tree_verifies_and_matches_the_embedded_bundle() {
    // The development seam must accept exactly what the compiled-in bundle
    // accepts: same name, same key, same setup digest, every resource
    // SHA-256-verified (payload -> manifest SHA -> index SHA).
    let from_tree = load_profile_bundle(&data_dir(), None, true).expect("tree verifies");
    let embedded = crate::bundle_for_selection(six_selection()).expect("embedded bundle");

    assert_eq!(from_tree.name(), "wwise2013-6ch-44100");
    assert_eq!(from_tree.key(), embedded.key());
    assert_eq!(from_tree.block_sizes(), embedded.block_sizes());
    assert_eq!(
        from_tree.container_metadata(),
        embedded.container_metadata()
    );
    assert_eq!(
        from_tree.setup().expect("vorbis.setup").sha256(),
        embedded.setup().expect("vorbis.setup").sha256()
    );
    assert_eq!(from_tree.runtime_manifest().resources().len(), 10);
    from_tree.verify_all().expect("verify_all over the tree");
}

// ---------------------------------------------------------------------------
// Synthetic profile trees: tamper detection and schema rejection
// ---------------------------------------------------------------------------

#[test]
fn synthetic_tree_loads_and_verifies() {
    let data = valid_tree("ok");
    let bundle = load_profile_bundle(&data, None, true).expect("synthetic tree loads");
    assert_eq!(bundle.name(), "wwise2013-6ch-44100");
    let setup = bundle.setup_packet().expect("setup payload");
    assert_eq!(setup, b"setup-payload");
}

#[test]
fn tamper_detection_sha_mismatch() {
    let data = valid_tree("sha");
    // Mutate the resource payload after the manifest digest was recorded.
    let file = data
        .profiles_dir()
        .join("wwise2013-6ch-44100/vorbis_setup.bin");
    std::fs::write(&file, b"setup-payload-MUTATED").unwrap();
    let err = load_profile_bundle(&data, None, true).unwrap_err();
    match err {
        ProfileError::ShaMismatch {
            expected, actual, ..
        } => {
            assert_ne!(expected, actual);
        }
        other => panic!("expected ShaMismatch, got {other:?}"),
    }
}

#[test]
fn tamper_detection_truncated_resource() {
    let data = valid_tree("trunc");
    let file = data
        .profiles_dir()
        .join("wwise2013-6ch-44100/vorbis_setup.bin");
    std::fs::write(&file, b"short").unwrap();
    let err = load_profile_bundle(&data, None, true).unwrap_err();
    assert!(
        matches!(err, ProfileError::ShaMismatch { .. }),
        "got {err:?}"
    );
}

#[test]
fn tamper_detection_path_traversal_in_manifest() {
    let root = temp_tree("trav");
    let profile_dir = root.join("wwise2013-6ch-44100");
    let setup_payload = b"setup";
    let setup_sha = sha256_hex(setup_payload);
    write(&profile_dir.join("setup.bin"), setup_payload);

    let manifest = serde_json::json!({
        "schema": "wwise-wem.profile-manifest.v1",
        "name": "wwise2013-6ch-44100",
        "key": {
            "generation": "2013.2", "channels": 6, "sample_rate": 44100,
            "channel_layout": "5.1",
            "quality_setup_identity": format!("sha256:{setup_sha}")
        },
        "block_sizes": [4, 8],
        "container_metadata": {
            "wFormatTag": 65535, "nChannels": 6, "nSamplesPerSec": 44100,
            "nAvgBytesPerSec": 0, "nBlockAlign": 0, "wBitsPerSample": 0,
            "cbSize": 48, "wReserved0": 0, "dwChannelMask": 63,
            "dwTotalPCMFrames": 0, "dwFirstAudioPacketOffset": 0,
            "dwDataPayloadSize": 0, "dwUnknown_0x24": 0, "dwSeekTableSize": 0,
            "dwVorbisDataOffset": 0, "uMaxPacketSize": 0, "uUnknown_0x32": 0,
            "dwUnknown_0x34": 0, "dwUnknown_0x38": 0, "dwUnknown_0x3C": 0,
            "uBlocksize0Pow": 2, "uBlocksize1Pow": 3
        },
        "resources": {
            "vorbis.setup": {
                "path": "../../escape/setup.bin",
                "sha256": setup_sha
            }
        }
    });
    let manifest_bytes = serde_json::to_vec_pretty(&manifest).unwrap();
    let manifest_sha = sha256_hex(&manifest_bytes);
    write(&profile_dir.join("manifest.json"), &manifest_bytes);

    let index = serde_json::json!({
        "schema": "wwise-wem.profile-index.v1",
        "default": "wwise2013-6ch-44100",
        "profiles": {
            "wwise2013-6ch-44100": {
                "manifest": "wwise2013-6ch-44100/manifest.json",
                "sha256": manifest_sha
            }
        }
    });
    write(
        &root.join("index.json"),
        &serde_json::to_vec(&index).unwrap(),
    );

    let err = load_profile_bundle(&DataDir::from_profiles_dir(root), None, true).unwrap_err();
    assert!(
        matches!(err, ProfileError::UnsafePath { .. }),
        "got {err:?}"
    );
}

#[test]
fn tamper_detection_index_sha_mismatch() {
    let root = temp_tree("idx");
    let profile_dir = root.join("wwise2013-6ch-44100");
    let manifest = serde_json::json!({
        "schema": "wwise-wem.profile-manifest.v1",
        "name": "wrong-name",
        "key": {
            "generation": "2013.2", "channels": 6, "sample_rate": 44100,
            "channel_layout": "5.1",
            "quality_setup_identity": "sha256:0000000000000000000000000000000000000000000000000000000000000000"
        },
        "block_sizes": [4, 8],
        "container_metadata": {
            "wFormatTag": 65535, "nChannels": 6, "nSamplesPerSec": 44100,
            "nAvgBytesPerSec": 0, "nBlockAlign": 0, "wBitsPerSample": 0,
            "cbSize": 48, "wReserved0": 0, "dwChannelMask": 63,
            "dwTotalPCMFrames": 0, "dwFirstAudioPacketOffset": 0,
            "dwDataPayloadSize": 0, "dwUnknown_0x24": 0, "dwSeekTableSize": 0,
            "dwVorbisDataOffset": 0, "uMaxPacketSize": 0, "uUnknown_0x32": 0,
            "dwUnknown_0x34": 0, "dwUnknown_0x38": 0, "dwUnknown_0x3C": 0,
            "uBlocksize0Pow": 2, "uBlocksize1Pow": 3
        },
        "resources": {}
    });
    let manifest_bytes = serde_json::to_vec(&manifest).unwrap();
    write(&profile_dir.join("manifest.json"), &manifest_bytes);
    // Index carries a deliberately wrong manifest sha256.
    let index = serde_json::json!({
        "schema": "wwise-wem.profile-index.v1",
        "default": "wwise2013-6ch-44100",
        "profiles": {
            "wwise2013-6ch-44100": {
                "manifest": "wwise2013-6ch-44100/manifest.json",
                "sha256": "0000000000000000000000000000000000000000000000000000000000000000"
            }
        }
    });
    write(
        &root.join("index.json"),
        &serde_json::to_vec(&index).unwrap(),
    );

    let err = load_profile_bundle(&DataDir::from_profiles_dir(root), None, false).unwrap_err();
    assert!(
        matches!(err, ProfileError::ShaMismatch { .. }),
        "got {err:?}"
    );
}

#[test]
fn tamper_detection_schema_strings() {
    // Bad index schema.
    let root = temp_tree("schema-index");
    let index = serde_json::json!({
        "schema": "wwise-wem.profile-index.v0",
        "default": "wwise2013-6ch-44100",
        "profiles": {}
    });
    write(
        &root.join("index.json"),
        &serde_json::to_vec(&index).unwrap(),
    );
    let err = load_profile_bundle(&DataDir::from_profiles_dir(root), None, false).unwrap_err();
    assert!(
        matches!(err, ProfileError::UnsupportedIndexSchema { .. }),
        "got {err:?}"
    );

    // Unknown profile in the index.
    let err = load_profile_bundle(&data_dir(), Some("does-not-exist"), false).unwrap_err();
    assert!(
        matches!(err, ProfileError::ProfileNotInIndex { .. }),
        "got {err:?}"
    );
}

// ---------------------------------------------------------------------------
// Resource identity and path-safety rejections
// ---------------------------------------------------------------------------

#[test]
fn resource_ref_rejections() {
    let data = data_dir();
    // Bad digest shapes.
    assert!(ResourceRef::new(
        data.clone(),
        "wwise2013-6ch-44100/vorbis/setup.bin",
        "ABCDEF"
    )
    .is_err());
    assert!(ResourceRef::new(
        data.clone(),
        "wwise2013-6ch-44100/vorbis/setup.bin",
        "g000000000000000000000000000000000000000000000000000000000000000"
    )
    .is_err());
    // Uppercase digest is normalized to lowercase.
    let ref_ = ResourceRef::new(
        data.clone(),
        "wwise2013-6ch-44100/vorbis/setup.bin",
        &crate::WWISE2013_6CH_44100_SETUP_IDENTITY
            .strip_prefix("sha256:")
            .unwrap()
            .to_uppercase(),
    )
    .expect("uppercase digest normalizes");
    assert_eq!(
        ref_.sha256(),
        crate::WWISE2013_6CH_44100_SETUP_IDENTITY
            .strip_prefix("sha256:")
            .unwrap()
    );
    // Read verifies the digest.
    let bytes = ref_.read_bytes().expect("verified read");
    assert_eq!(bytes.len(), 201);

    // Missing resource.
    let missing = ResourceRef::new(
        data,
        "wwise2013-6ch-44100/does-not-exist.bin",
        &"0".repeat(64),
    )
    .expect("ref constructs");
    assert!(matches!(
        missing.read_bytes().unwrap_err(),
        ProfileError::MissingResource { .. }
    ));
}

#[test]
fn resource_keys_are_canonical_slash_form() {
    // Key-shape invariant (holds on every platform): every logical resource
    // key of a loaded bundle is a profiles-dir-relative POSIX path. The
    // shared validator rejects backslashes, so any platform separator
    // leaking into key construction (std::path's display() on Windows)
    // fails here — asserted at the code level, no OS reproduction needed.
    let bundle = load_profile_bundle(&data_dir(), None, false).expect("profile loads");
    let manifest = bundle.runtime_manifest();
    assert_eq!(manifest.ref_path(), "wwise2013-6ch-44100/manifest.json");
    for (name, ref_) in manifest.resources() {
        assert!(
            !ref_.path().contains('\\'),
            "resource key of {name} must stay slash-separated"
        );
        assert!(
            normalize_resource_path(ref_.path()).is_ok(),
            "resource key of {name} must stay valid against the shared rules"
        );
        assert!(
            ref_.path()
                .strip_prefix("wwise2013-6ch-44100/")
                .is_some_and(|rel| !rel.is_empty()),
            "resource key of {name} is profiles-dir-relative"
        );
    }
    // The manifest view's keys are the same canonical keys, parent-relative.
    let view = bundle
        .to_encoder_profile()
        .expect("encoder profile")
        .runtime_manifest()
        .expect("manifest view");
    for key in view.files.keys() {
        assert!(!key.contains('\\'), "manifest view key {key:?}");
    }
    assert!(view.files.contains_key("vorbis/setup.bin"));
}

#[test]
fn resource_path_normalization_rejections() {
    assert!(normalize_resource_path("vorbis/setup.bin").is_ok());
    assert!(normalize_resource_path("").is_err());
    assert!(normalize_resource_path("vorbis\\setup.bin").is_err());
    assert!(normalize_resource_path("/abs/setup.bin").is_err());
    assert!(normalize_resource_path("./vorbis/setup.bin").is_err());
    assert!(normalize_resource_path("../escape.bin").is_err());
    assert!(normalize_resource_path("vorbis/../escape.bin").is_err());
    assert!(normalize_resource_path("vorbis//setup.bin").is_err());
}

// ---------------------------------------------------------------------------
// Registry behaviour of the development tree
// ---------------------------------------------------------------------------

#[test]
fn installed_registry_resolutions() {
    let registry = installed_registry(&data_dir()).expect("registry");
    // The installed index carries the 6ch profile plus the 2ch/48000
    // profile (fully registered: setup and psychoacoustics available).
    assert_eq!(registry.len(), 2);

    let six_channel = registry
        .resolve_selection(six_selection())
        .expect("6ch selection resolves");
    assert_eq!(six_channel.name(), "wwise2013-6ch-44100");
    assert!(six_channel.setup_available());

    // The 2ch/48000 profile resolves by its geometry and carries its setup
    // digest; its psychoacoustics are registered, so the profile is fully
    // ready (no manifest-declared pending reason).
    let stereo = registry
        .resolve_selection(two_channel_selection())
        .expect("2ch selection resolves");
    assert_eq!(stereo.name(), "wwise2013-2ch-48000");
    assert!(stereo.setup_available());
    assert_eq!(
        stereo.setup_sha256(),
        "894a545ca48993bb0e5b768b1a367fd4475f806658b51bbcc88c8a6243849afc"
    );
    assert!(stereo.pending_reason().is_none());
    // The two selections resolve to distinct setup digests: a setup digest
    // is a consequence of a selection, never an alternative selector.
    assert_ne!(six_channel.setup_sha256(), stereo.setup_sha256());

    // Unknown key rejected.
    let unknown = ProfileKey::new(
        2,
        48000,
        "2013.2".into(),
        "stereo".into(),
        "sha256:0".into(),
    )
    .expect("full identity");
    assert!(registry.resolve_key(&unknown).is_err());

    // The free resolvers agree with the registry.
    let resolved = crate::resolve_wem_profile_selection(six_selection()).expect("6ch selection");
    assert_eq!(resolved.name(), "wwise2013-6ch-44100");
    assert_eq!(resolved.block_sizes(), [256, 2048]);
    assert_eq!(resolved.quality(), None);

    // Quality-bound lookups are additive copies (the registry instance is
    // never mutated).
    let bound = crate::resolve_wem_profile_selection_quality(six_selection(), Some(4.0))
        .expect("quality copy");
    assert_eq!(bound.quality(), Some(4.0));
    let registry = installed_registry(&data_dir()).expect("registry");
    assert_eq!(
        registry
            .resolve_selection(six_selection())
            .expect("original")
            .quality(),
        None
    );
    assert!(crate::resolve_wem_profile_selection_quality(six_selection(), Some(f64::NAN)).is_err());
    let stereo_by_selection =
        crate::resolve_wem_profile_selection(two_channel_selection()).expect("2ch selection");
    assert!(stereo_by_selection.setup_available());
}

/// The 2ch/48000 profile: its setup packet, codebooks, and quality curves are
/// registered from the paired build together with its psychoacoustic
/// calibration.
#[test]
fn two_channel_profile_lists_in_the_registry_as_setup_available() {
    const TWO_CHANNEL_NAME: &str = "wwise2013-2ch-48000";
    const TWO_CHANNEL_SETUP_SHA: &str =
        "894a545ca48993bb0e5b768b1a367fd4475f806658b51bbcc88c8a6243849afc";

    let registry = installed_registry(&data_dir()).expect("registry loads");
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
        .resolve_selection(six_selection())
        .expect("6ch selection resolves");
    assert_ne!(six_channel.setup_sha256(), profile.setup_sha256());
}

/// The development tree and the compiled-in bundle are one profile: the
/// loader must never prefer one over the other.
#[test]
fn development_tree_and_compiled_in_bundle_agree_for_every_selection() {
    let registry = installed_registry(&data_dir()).expect("registry loads");
    for selection in [six_selection(), two_channel_selection()] {
        let from_tree = registry
            .resolve_selection(selection)
            .expect("tree profile resolves");
        let bundle = crate::bundle_for_selection(selection).expect("embedded bundle");
        assert_eq!(from_tree.key(), bundle.key());
        assert_eq!(from_tree.setup_sha256(), bundle.setup().unwrap().sha256());
    }
}
