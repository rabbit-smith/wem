//! Mode-selection tail rule (paired-build parity).
//!
//! The paired build emits frames while the *previous* frame's center still lies
//! inside the PCM, so the final frame runs one hop past the source length -- its
//! own hop, not a fixed prefix.  A constant `center < source_len + prefix` bound
//! overshoots by a fixed amount instead and emits trailing frames the build does
//! not (the 2ch/48k reference stream is seven short frames shorter than that
//! bound produces).

use wem_analysis::session::AnalysisSession;

fn repo_root() -> std::path::PathBuf {
    std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .and_then(|p| p.parent())
        .expect("repo root")
        .to_path_buf()
}

fn synthetic(frames: usize, channels: usize) -> Vec<Vec<f64>> {
    (0..channels)
        .map(|channel| {
            (0..frames)
                .map(|frame| {
                    (((frame * 7 + channel * 11 + 3) % 64536) as i64 - 32768) as f64 / 32768.0
                })
                .collect()
        })
        .collect()
}

#[test]
fn tail_stops_one_frame_past_the_source_length() {
    let data_dir =
        wem_profiles::DataDir::from_profiles_dir(repo_root().join("src/wwise_wem/data/profiles"));
    let bundle = wem_profiles::load_profile_bundle(&data_dir, Some("wwise2013-2ch-48000"), false)
        .expect("profile loads");
    let resources =
        wem_profiles::assemble_analysis_resources(&bundle, None).expect("resources assemble");
    let frames = 7425usize;
    let pcm = synthetic(frames, 2);
    let mut session = AnalysisSession::new(2, 48000, [256, 2048], resources).expect("session");
    let modes = session.select_modes(&pcm).expect("modes");

    let blocksizes = [256i64, 2048i64];
    let mut center = 0i64;
    let mut centers: Vec<i64> = Vec::new();
    for (index, mode) in modes.iter().enumerate() {
        let following = modes.get(index + 1).copied().unwrap_or(0);
        centers.push(center);
        center += blocksizes[*mode as usize] / 4 + blocksizes[following as usize] / 4;
    }

    let last = *centers.last().expect("at least one frame");
    assert!(
        last >= frames as i64,
        "last frame must reach the source length: {last}"
    );
    for center in &centers[..centers.len() - 1] {
        assert!(
            *center < frames as i64,
            "frame at {center} starts past the source length"
        );
    }
    assert_eq!(centers.len(), 10, "paired-build tail emits ten frames here");
}