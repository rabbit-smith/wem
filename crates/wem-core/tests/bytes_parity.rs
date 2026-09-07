//! Bytes-entry parity test (the wasm32-unknown-unknown path).
//!
//! The in-memory entry points (`Encoder::from_profile_bytes`,
//! `StreamSession::for_profile_ref_bytes`,
//! `wem_profiles::load_profile_bundle_from_bytes`) must produce
//! byte-identical output to the filesystem entries for the same profile,
//! and must reject exactly what the shared validator rejects (zero drift).
//!
//! These tests run natively: they read the fixture profile tree from disk
//! only to build the bytes input — the bytes entry itself never touches
//! the filesystem.

use sha2::{Digest, Sha256};
use wem_core::encoder::{Encoder, Pcm16};
use wem_core::stream::{ProfileRef, StreamSession};
use wem_core::usecases::wav::read_pcm16;

const PROFILE_NAME: &str = "wwise2013-6ch-44100";
const GOLDEN_WEM_SHA256: &str = "17851d26c6210b85e498ae0452d2562d7b9e2c3e9e795c459656b9c9d8d35247";
const SETUP_SHA256: &str = "3ef56cbd6e6a66a5474005db05912624487faa555fb2cdfed130f606b322e4e3";

fn fixtures_dir() -> std::path::PathBuf {
    // CARGO_MANIFEST_DIR = <root>/crates/wem-core
    std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../..")
        .join("tests/fixtures")
        .canonicalize()
        .expect("fixtures directory resolves")
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
/// bytes entry expects (native fs read, bytes entry only).
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
                out.push((
                    rel.to_string_lossy().into_owned(),
                    std::fs::read(&path).expect("resource file reads"),
                ));
            }
        }
    }
    walk(&dir, &dir, &mut files);
    assert!(!files.is_empty(), "profile tree is empty");
    (index, files)
}

fn read_fixture(name: &str) -> Vec<u8> {
    std::fs::read(fixtures_dir().join(name)).expect("fixture file reads")
}

fn encode_once(encoder: &Encoder) -> wem_core::EncodeResult {
    let wav = read_pcm16(&fixtures_dir().join("input.wav")).expect("input.wav reads");
    let pcm = wav.to_pcm16().expect("wav converts to Pcm16");
    encoder.encode_pcm(&pcm).expect("encode runs")
}

// ---------------------------------------------------------------------------
// 1. One-shot encode: bytes entry == filesystem entry (Vec<u8> equality)
// ---------------------------------------------------------------------------

#[test]
fn bytes_encoder_matches_fs_encoder_byte_for_byte() {
    let (index, files) = read_profile_bytes_bundle();
    let encoder_bytes =
        Encoder::from_profile_bytes(&index, files.clone()).expect("bytes encoder builds");
    let encoder_fs = Encoder::from_profile(PROFILE_NAME).expect("fs encoder builds");

    let result_bytes = encode_once(&encoder_bytes);
    let result_fs = encode_once(&encoder_fs);

    // The two construction paths must produce identical WEM bytes.
    assert_eq!(
        result_bytes.data, result_fs.data,
        "bytes entry WEM differs from encode_pcm WEM"
    );
    // ...and that is the reference golden.
    let sha = wem_profiles::resources::hex(Sha256::digest(&result_bytes.data));
    println!(
        "bytes-entry WEM sha256: {} (len {})",
        sha,
        result_bytes.data.len()
    );
    assert_eq!(
        sha, GOLDEN_WEM_SHA256,
        "bytes entry WEM differs from golden"
    );
    assert_eq!(result_bytes.data, read_fixture("reference.wem"));
    assert_eq!(result_bytes.stats, result_fs.stats);
}

// ---------------------------------------------------------------------------
// 2. StreamSession: bytes init == encode_pcm (chunked pushes)
// ---------------------------------------------------------------------------

#[test]
fn bytes_stream_session_matches_encode_pcm() {
    let (index, files) = read_profile_bytes_bundle();
    let wav = read_pcm16(&fixtures_dir().join("input.wav")).expect("input.wav reads");
    let le_bytes: Vec<u8> = wav.interleaved_le_bytes();
    let bytes_per_frame = wav.channels() * 2usize;

    let ref_ = ProfileRef::with_name(SETUP_SHA256, PROFILE_NAME);
    let mut session =
        StreamSession::for_profile_ref_bytes(&ref_, &index, files).expect("bytes init");
    // Two uneven chunks.
    let cut = (le_bytes.len() / bytes_per_frame / 2) * bytes_per_frame;
    assert_eq!(cut % bytes_per_frame, 0);
    session.push_pcm_chunk(&le_bytes[..cut]).expect("chunk one");
    session.push_pcm_chunk(&le_bytes[cut..]).expect("chunk two");
    let streamed = session.finish().expect("finish");

    let encoder_fs = Encoder::from_profile(PROFILE_NAME).expect("fs encoder builds");
    let oneshot = encode_once(&encoder_fs);

    assert_eq!(
        streamed.data, oneshot.data,
        "bytes StreamSession WEM differs from encode_pcm WEM"
    );
    let sha = wem_profiles::resources::hex(Sha256::digest(&streamed.data));
    println!(
        "bytes stream-session WEM sha256: {} (len {})",
        sha,
        streamed.data.len()
    );
    assert_eq!(
        sha, GOLDEN_WEM_SHA256,
        "bytes stream WEM differs from golden"
    );
}

// ---------------------------------------------------------------------------
// 3. Rejection parity: the bytes entry rejects exactly what the fs entry
//    rejects (same validator; zero drift).
// ---------------------------------------------------------------------------

#[test]
fn tampered_resource_bytes_are_rejected_with_sha_mismatch() {
    let (index, files) = read_profile_bytes_bundle();
    // The untouched bytes bundle verifies cleanly through the shared core.
    let _ok = wem_profiles::load_profile_bundle_from_bytes(&index, files.clone(), None, true)
        .expect("untouched bytes bundle verifies");

    // Flip one byte of the vorbis.setup payload: the shared validator must
    // reject it exactly like the filesystem entry would.
    let mut tampered = files.clone();
    for (path, bytes) in &mut tampered {
        if path.ends_with("vorbis/setup.bin") {
            bytes[10] ^= 0x01;
        }
    }
    let err = Encoder::from_profile_bytes(&index, tampered)
        .err()
        .expect("tampered bytes must fail");
    match err {
        wem_core::error::EncoderError::Internal(inner) => {
            assert!(
                matches!(
                    inner,
                    wem_core::error::InternalError::Profile(
                        wem_profiles::ProfileError::ShaMismatch { .. }
                    )
                ),
                "expected ShaMismatch, got {inner:?}"
            );
        }
        other => panic!("expected Profile(ShaMismatch), got {other:?}"),
    }
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
fn bytes_entry_refuses_unknown_profiles_and_bogus_digests() {
    let (index, files) = read_profile_bytes_bundle();
    // Unknown profile name in the index selection.
    let err =
        wem_profiles::load_profile_bundle_from_bytes(&index, files.clone(), Some("nope"), false)
            .unwrap_err();
    assert!(
        matches!(
            err,
            wem_profiles::ProfileError::ProfileNotInIndex { ref profile } if profile == "nope"
        ),
        "expected ProfileNotInIndex, got {err:?}"
    );

    // StreamSession: setup sha that matches no bytes-bundle profile.
    let bogus = ProfileRef {
        setup_sha256: "0".repeat(64),
        name: None,
    };
    let err = StreamSession::for_profile_ref_bytes(&bogus, &index, files.clone())
        .err()
        .expect("bogus digest must fail");
    assert!(
        matches!(err, wem_core::error::EncoderError::ProfileNotFound { .. }),
        "expected ProfileNotFound, got {err:?}"
    );

    // StreamSession: right sha, wrong soft name.
    let wrong_name = ProfileRef::with_name(SETUP_SHA256, "another-profile");
    let err = StreamSession::for_profile_ref_bytes(&wrong_name, &index, files)
        .err()
        .expect("name mismatch must fail");
    assert!(
        matches!(err, wem_core::error::EncoderError::StateError { .. }),
        "expected StateError, got {err:?}"
    );
}

// ---------------------------------------------------------------------------
// 4. Input geometry guards still apply on the bytes entry.
// ---------------------------------------------------------------------------

#[test]
fn bytes_encoder_geometry_and_minimum_frame_guards_apply() {
    let (index, files) = read_profile_bytes_bundle();
    let encoder = Encoder::from_profile_bytes(&index, files).expect("bytes encoder builds");
    let bad_geometry =
        Pcm16::new(48000, vec![vec![0i16; 5000], vec![0i16; 5000]]).expect("pcm builds");
    assert!(
        matches!(
            encoder.encode_pcm(&bad_geometry),
            Err(wem_core::error::EncoderError::GeometryMismatch { .. })
        ),
        "wrong geometry must be rejected"
    );

    let short = Pcm16::new(44100, vec![vec![0i16; 4095]; 6]).expect("pcm builds");
    assert!(
        matches!(
            encoder.encode_pcm(&short),
            Err(wem_core::error::EncoderError::InputTooShort {
                want: 4096,
                got: 4095
            })
        ),
        "4095 frames must be rejected"
    );
}
