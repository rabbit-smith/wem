//! Quality pass-through (wem-core level): a quality factor bound to the
//! profile is forwarded to the analysis assembly; with quality=None the
//! historical golden bytes are unchanged.

use sha2::{Digest, Sha256};
use wem_core::encoder::Encoder;
use wem_core::usecases::wav::read_pcm16;
use wem_profiles::{resolve_wem_profile, resolve_wem_profile_quality};

const GOLDEN_WEM_SHA256: &str = "17851d26c6210b85e498ae0452d2562d7b9e2c3e9e795c459656b9c9d8d35247";

fn fixtures_dir() -> std::path::PathBuf {
    std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../..")
        .join("tests/fixtures")
        .canonicalize()
        .expect("fixtures directory resolves")
}

#[test]
fn resolve_wem_profile_quality_returns_additive_copies() {
    let base = resolve_wem_profile(6, 44100).expect("6ch profile resolves");
    assert!(base.setup_available());
    assert_eq!(base.quality(), None);

    let bound =
        resolve_wem_profile_quality(6, 44100, Some(4.0)).expect("quality-bound copy");
    assert_eq!(bound.quality(), Some(4.0));
    assert!(bound.setup_available());

    // Non-finite quality is rejected.
    assert!(resolve_wem_profile_quality(6, 44100, Some(f64::NAN)).is_err());
}

#[test]
fn quality_none_encode_bytes_match_the_golden_sha() {
    let profile =
        resolve_wem_profile_quality(6, 44100, None).expect("6ch profile resolves");
    let encoder = Encoder::from_profile_model(&profile, None).expect("encoder builds");
    let wav = read_pcm16(&fixtures_dir().join("input.wav")).expect("input.wav reads");
    let pcm = wav.to_pcm16().expect("wav converts to Pcm16");
    let encoded = encoder.encode_pcm(&pcm).expect("encode runs");
    let sha = wem_profiles::resources::hex(Sha256::digest(&encoded.data));
    assert_eq!(
        sha, GOLDEN_WEM_SHA256,
        "quality=None must reproduce the historical golden bytes exactly"
    );
}
