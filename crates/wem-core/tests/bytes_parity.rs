//! Profile-bundle bytes assembly: index -> manifest -> verified resources.
//!
//! The bytes loader (`wem_profiles::load_profile_bundle_from_bytes`) is the
//! threadless profile-assembly path (wasm32-unknown-unknown): every logical
//! resource is SHA-256 verified on load, and it must accept and reject
//! exactly what the filesystem loader does (shared validator, zero drift).
//!
//! These tests run natively: they read the fixture profile tree from disk
//! only to build the bytes input — the bytes loader itself never touches
//! the filesystem.

use sha2::{Digest, Sha256};
use wem_core::encoder::Encoder;
use wem_core::stream::StreamSession;
use wem_core::usecases::wav::read_pcm16;
use wem_core::{WwiseProfile, WwiseVersion};

const PROFILE_NAME: &str = "wwise2013-6ch-44100";
const SETUP_SHA256: &str = "3ef56cbd6e6a66a5474005db05912624487faa555fb2cdfed130f606b322e4e3";
const TWO_CHANNEL_PROFILE_NAME: &str = "wwise2013-2ch-48000";
const TWO_CHANNEL_SETUP_SHA256: &str =
    "894a545ca48993bb0e5b768b1a367fd4475f806658b51bbcc88c8a6243849afc";

/// The fixture profile selection: the installed Wwise 2013 6ch/44100
/// configuration.
fn fixture_selection() -> WwiseProfile {
    WwiseProfile::new(WwiseVersion::Wwise2013, 6, 44_100).expect("6ch/44100 selection")
}

/// The installed Wwise 2013 2ch/48000 configuration.
fn two_channel_selection() -> WwiseProfile {
    WwiseProfile::new(WwiseVersion::Wwise2013, 2, 48_000).expect("2ch/48000 selection")
}

/// The profiles tree the Rust kernel resolves from the repo layout.
fn profiles_dir() -> std::path::PathBuf {
    std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../..")
        .join("src/wwise_wem/data/profiles")
        .canonicalize()
        .expect("profiles directory resolves")
}

/// Read every file of the profile tree as (profiles-dir-relative POSIX
/// path, bytes) pairs plus the raw index document — the exact input the
/// bytes loader expects (native fs read, bytes loader only).
fn read_profile_bytes_bundle() -> (Vec<u8>, Vec<(String, Vec<u8>)>) {
    let dir = profiles_dir();
    let index = std::fs::read(dir.join("index.json")).expect("index.json reads");

    let mut files: Vec<(String, Vec<u8>)> = Vec::new();
    fn walk(dir: &std::path::Path, cur: &std::path::Path, out: &mut Vec<(String, Vec<u8>)>) {
        for entry in std::fs::read_dir(cur).expect("directory reads") {
            let entry = entry.expect("directory entry");
            let path = entry.path();
            if path.is_dir() {
                walk(dir, &path, out);
            } else if path.file_name() != Some("index.json".as_ref()) {
                let rel = path.strip_prefix(dir).expect("path under profiles dir");
                // Canonical POSIX keys: the bytes contract is platform-
                // independent (no OS separator may reach the kernel).
                out.push((
                    rel.to_string_lossy().replace('\\', "/"),
                    std::fs::read(&path).expect("resource file reads"),
                ));
            }
        }
    }
    walk(&dir, &dir, &mut files);
    assert!(!files.is_empty(), "profile tree is empty");
    (index, files)
}

fn two_channel_dir() -> std::path::PathBuf {
    std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../..")
        .join("tests/data/2ch-reference")
}

// ---------------------------------------------------------------------------
// 1. The bytes bundle resolves the same identity as the installed selection
// ---------------------------------------------------------------------------

#[test]
fn bytes_bundle_matches_the_installed_selection() {
    let (index, files) = read_profile_bytes_bundle();
    let from_bytes = wem_profiles::load_profile_bundle_from_bytes(&index, files, None, true)
        .expect("bytes bundle verifies")
        .to_encoder_profile()
        .expect("bytes bundle profile");
    let installed =
        wem_profiles::resolve_wem_profile_selection(fixture_selection()).expect("6ch resolves");

    // One identity, one setup digest: the bytes assembly and the installed
    // selection agree on what the 6ch/44100 configuration is.
    assert_eq!(from_bytes.name(), PROFILE_NAME);
    assert_eq!(from_bytes.setup_sha256(), SETUP_SHA256);
    assert_eq!(from_bytes.key(), installed.key());
    assert_eq!(from_bytes.setup_sha256(), installed.setup_sha256());

    // The filesystem loader resolves the same identity from the same tree.
    let data = wem_profiles::DataDir::from_profiles_dir(profiles_dir());
    let from_fs = wem_profiles::load_profile_bundle(&data, Some(PROFILE_NAME), true)
        .expect("fs bundle verifies")
        .to_encoder_profile()
        .expect("fs bundle profile");
    assert_eq!(from_fs.key(), from_bytes.key());
    assert_eq!(from_fs.setup_sha256(), from_bytes.setup_sha256());
}

#[test]
fn bytes_bundle_carries_the_two_channel_profile_identity() {
    let (index, files) = read_profile_bytes_bundle();
    let from_bytes = wem_profiles::load_profile_bundle_from_bytes(
        &index,
        files,
        Some(TWO_CHANNEL_PROFILE_NAME),
        true,
    )
    .expect("named 2ch bytes bundle verifies")
    .to_encoder_profile()
    .expect("2ch bytes bundle profile");
    let installed = wem_profiles::resolve_wem_profile_selection(two_channel_selection())
        .expect("2ch selection resolves");

    assert_eq!(from_bytes.name(), TWO_CHANNEL_PROFILE_NAME);
    assert_eq!(from_bytes.setup_sha256(), TWO_CHANNEL_SETUP_SHA256);
    assert_eq!(from_bytes.key(), installed.key());
}

// ---------------------------------------------------------------------------
// 2. The installed selections encode the committed reference material
// ---------------------------------------------------------------------------

#[test]
fn two_channel_selection_encodes_the_committed_two_channel_wem() {
    let encoder = Encoder::new(two_channel_selection()).expect("2ch selection resolves");
    assert_eq!(encoder.profile().name(), TWO_CHANNEL_PROFILE_NAME);
    assert_eq!(encoder.profile().setup_sha256(), TWO_CHANNEL_SETUP_SHA256);

    let wav = read_pcm16(&two_channel_dir().join("tone_high.wav")).expect("2ch WAV reads");
    let pcm = wav.to_pcm16().expect("2ch WAV converts to Pcm16");
    let result = encoder.encode_pcm(&pcm).expect("2ch encode runs");
    assert_eq!(
        result.data,
        std::fs::read(two_channel_dir().join("tone_high.wem")).unwrap()
    );
}

#[test]
fn two_channel_selection_streams_the_committed_two_channel_wem() {
    let wav = read_pcm16(&two_channel_dir().join("tone_high.wav")).expect("2ch WAV reads");
    let le_bytes = wav.interleaved_le_bytes();
    let bytes_per_frame = wav.channels() * 2;
    let mut session =
        StreamSession::for_selection(two_channel_selection()).expect("2ch session opens");

    // Two uneven chunks: chunk boundaries must not affect the bytes.
    let cut = 12_345 * bytes_per_frame;
    session.push_pcm_chunk(&le_bytes[..cut]).expect("chunk one");
    session.push_pcm_chunk(&le_bytes[cut..]).expect("chunk two");
    let result = session.finish().expect("2ch stream finishes");

    assert_eq!(
        result.data,
        std::fs::read(two_channel_dir().join("tone_high.wem")).unwrap()
    );
}

// ---------------------------------------------------------------------------
// 3. Rejection parity: the bytes loader rejects what the fs loader rejects
//    (same validator; zero drift).
// ---------------------------------------------------------------------------

#[test]
fn tampered_resource_bytes_are_rejected_with_sha_mismatch() {
    let (index, files) = read_profile_bytes_bundle();
    // The untouched bytes bundle verifies cleanly through the shared core.
    let _ok = wem_profiles::load_profile_bundle_from_bytes(&index, files.clone(), None, true)
        .expect("untouched bytes bundle verifies");

    // Flip one byte of the vorbis.setup payload: the shared validator must
    // reject it exactly like the filesystem loader would.
    let mut tampered = files.clone();
    for (path, bytes) in &mut tampered {
        if path.ends_with("vorbis/setup.bin") {
            bytes[10] ^= 0x01;
        }
    }
    let err = wem_profiles::load_profile_bundle_from_bytes(&index, tampered, None, true)
        .expect_err("tampered bytes must fail");
    assert!(
        matches!(err, wem_profiles::ProfileError::ShaMismatch { .. }),
        "expected ShaMismatch, got {err:?}"
    );
}

#[test]
fn missing_resource_bytes_are_rejected() {
    let (index, files) = read_profile_bytes_bundle();
    // Drop the t97 codebook table from the bytes bundle.
    let incomplete: Vec<(String, Vec<u8>)> = files
        .into_iter()
        .filter(|(path, _)| !path.ends_with("vorbis/codebooks/t97.json"))
        .collect();
    let err =
        wem_profiles::load_profile_bundle_from_bytes(&index, incomplete, None, true).unwrap_err();
    assert!(
        matches!(err, wem_profiles::ProfileError::MissingResource { .. }),
        "expected MissingResource, got {err:?}"
    );
}

#[test]
fn unsafe_resource_paths_are_rejected_from_bytes() {
    // A manifest entry escaping the profiles directory (../) must be
    // rejected identically to the filesystem path's normalize_resource_path.
    //
    // The JSON tampering is done with literal text substitution on the
    // known-good documents (no JSON dependency in wem-core): the manifest
    // resource path changes and the index is re-pinned to the tampered
    // manifest's digest, so the manifest sha check passes and the
    // resource-path rejection (the drift-critical condition) is exercised.
    let dir = profiles_dir();
    let manifest_text = std::fs::read_to_string(dir.join("wwise2013-6ch-44100/manifest.json"))
        .expect("manifest reads");
    let index_text = std::fs::read_to_string(dir.join("index.json")).expect("index reads");

    let old_path_field = "\"path\": \"vorbis/setup.bin\"";
    let new_path_field = "\"path\": \"../escape/attempts/setup.bin\"";
    assert!(
        manifest_text.contains(old_path_field),
        "fixture manifest layout changed"
    );
    let tampered_manifest_bytes = manifest_text
        .replace(old_path_field, new_path_field)
        .into_bytes();

    let tampered_sha = wem_profiles::resources::hex(Sha256::digest(&tampered_manifest_bytes));
    // The index pins the manifest digest: locate the current sha256 value in
    // the index text (64 hex digits after the sole manifest sha field) and
    // re-pin it to the tampered document.
    let sha_field_prefix = "\"sha256\": \"";
    let sha_start = index_text
        .find(sha_field_prefix)
        .expect("index sha field present")
        + sha_field_prefix.len();
    let current_index_sha = &index_text[sha_start..sha_start + 64];
    assert!(
        current_index_sha.bytes().all(|b| b.is_ascii_hexdigit()),
        "fixture index layout changed"
    );
    let tampered_index_bytes = index_text
        .replace(current_index_sha, &tampered_sha)
        .into_bytes();

    let files: Vec<(String, Vec<u8>)> = vec![(
        "wwise2013-6ch-44100/manifest.json".to_string(),
        tampered_manifest_bytes,
    )];
    let err =
        wem_profiles::load_profile_bundle_from_bytes(&tampered_index_bytes, files, None, false)
            .unwrap_err();
    assert!(
        matches!(
            err,
            wem_profiles::ProfileError::UnsafePath { .. }
                | wem_profiles::ProfileError::ResourcePathMissing { .. }
        ),
        "expected a path rejection, got {err:?}"
    );
}

#[test]
fn bytes_loader_refuses_unknown_profiles() {
    let (index, files) = read_profile_bytes_bundle();
    // Unknown profile name in the index selection.
    let err = wem_profiles::load_profile_bundle_from_bytes(&index, files, Some("nope"), false)
        .unwrap_err();
    assert!(
        matches!(
            err,
            wem_profiles::ProfileError::ProfileNotInIndex { ref profile } if profile == "nope"
        ),
        "expected ProfileNotInIndex, got {err:?}"
    );
}
