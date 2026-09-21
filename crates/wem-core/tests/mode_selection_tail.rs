//! Mode-selection tail rule (paired-build parity).
//!
//! The paired build emits frames while the *previous* frame's center still lies
//! inside the PCM, so the final frame runs one hop past the source length -- its
//! own hop, not a fixed prefix.  A constant `center < source_len + prefix` bound
//! overshoots by a fixed amount instead and emits trailing frames the build does
//! not (the 2ch/48k reference stream is seven short frames shorter than that
//! bound produces).

use wem_analysis::config::AnalysisError;
use wem_analysis::preprocessing::windowing::WindowedFrame;
use wem_analysis::session::AnalysisSession;
use wem_profiles::{bundle_for_selection, WwiseProfile, WwiseVersion};
use wem_scheduling::FramePlan;

/// Analysis resources of the installed 2ch/48000 configuration, resolved from
/// a structured selection against the compiled-in profile bundle.
fn two_channel_resources() -> wem_analysis::config::AnalysisProfileResources {
    let selection =
        WwiseProfile::new(WwiseVersion::Wwise2013, 2, 48_000).expect("2ch/48000 selection");
    let bundle = bundle_for_selection(selection).expect("installed 2ch profile resolves");
    wem_profiles::assemble_analysis_resources(&bundle, None).expect("resources assemble")
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
    let resources = two_channel_resources();
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

#[test]
fn captured_transition_codes_never_fall_back_to_advanced_selector_state() {
    let resources = two_channel_resources();
    let mut session = AnalysisSession::new(2, 48000, [256, 2048], resources).expect("session");

    assert!(matches!(
        session.finalize_terminal_transition(0, 1),
        Err(AnalysisError::TransitionCodeMissing { recorded: 0, .. })
    ));

    let modes = session
        .select_modes(&synthetic(4096, 2))
        .expect("mode selection");
    let missing_index = modes.len() as i64;
    let missing = WindowedFrame {
        plan: FramePlan {
            index: missing_index,
            previous: 0,
            current: 0,
            following: 0,
            sample_start: 0,
            sample_end: 256,
            required_filled: 256,
            advance: 128,
        },
        center: 0,
        samples: vec![vec![0.0; 256]; 2],
    };
    assert_eq!(
        session.transition_code(&missing),
        Err(AnalysisError::TransitionCodeMissing {
            index: missing_index,
            recorded: modes.len(),
        })
    );
}
