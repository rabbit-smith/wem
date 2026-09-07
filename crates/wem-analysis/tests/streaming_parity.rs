//! Streaming feeder parity against the batch windowing kernel.
//!
//! The incremental `StreamingPcmFeeder` must select exactly the samples
//! the batch `iter_planned_pcm_windows` selects, for realistic mode
//! lists (the batch session's own mode loop), so the two paths cannot
//! drift on any frame they both produce.

use wem_analysis::preprocessing::streaming::{
    StreamingPcmFeeder, STREAM_DETECTOR_HOP, STREAM_DETECTOR_WINDOW,
};
use wem_analysis::preprocessing::windowing::iter_planned_pcm_windows;
use wem_analysis::session::AnalysisSession;
use wem_scheduling::plan_mode_sequence;

fn repo_root() -> std::path::PathBuf {
    std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .and_then(|p| p.parent())
        .expect("repo root")
        .to_path_buf()
}

fn sine_like(frames: i64) -> Vec<Vec<f64>> {
    (0..6)
        .map(|ch| {
            (0..frames)
                .map(|i| {
                    let phase = (i as f64 + ch as f64 * 3.0) * 0.017;
                    (phase as u32 % 628) as f64 * 0.01
                })
                .collect()
        })
        .collect()
}

#[test]
fn streaming_frame_rows_match_batch_materialization() {
    let frames = 9000i64;
    let pcm = sine_like(frames);

    let data_dir =
        wem_profiles::DataDir::from_profiles_dir(repo_root().join("src/wwise_wem/data/profiles"));
    let bundle = wem_profiles::load_profile_bundle(&data_dir, None, false).expect("profile loads");
    let resources = wem_profiles::assemble_analysis_resources(&bundle).expect("resources assemble");
    let frozen = resources
        .frozen
        .as_ref()
        .expect("frozen windows present")
        .window_halves
        .clone();

    // Realistic mode list: the batch session's own mode loop (which stops
    // at source_len + prefix), not a fabricated sequence.
    let mut session = AnalysisSession::new(6, 44100, [256, 2048], resources).expect("session");
    let modes = session.select_modes(&pcm).expect("select modes");
    let plans = plan_mode_sequence(&modes, &[256, 2048], 1).expect("plans");

    // Batch reference.
    let batch_frames =
        iter_planned_pcm_windows(&pcm, &plans, &[256, 2048], Some(&frozen)).expect("batch");

    // Streaming path: chunked push, then EOS. The first chunk (3000
    // samples) precedes the 4096-sample prime batch, so its drain is
    // empty — quanta flow only from the second chunk on.
    let mut feeder = StreamingPcmFeeder::new(6, [256, 2048]).expect("feeder");
    let mut drained = 0usize;
    let mut offset = 0usize;
    for chunk in [3000usize, 3000, 3000] {
        let rows: Vec<Vec<f64>> = pcm
            .iter()
            .map(|channel| channel[offset..offset + chunk].to_vec())
            .collect();
        offset += chunk;
        feeder.push(&rows).expect("push");
        drained += feeder.drain_completed_quanta().expect("drain").len();
    }
    feeder.finish_source().expect("finish");
    drained += feeder.drain_completed_quanta().expect("drain tail").len();
    let batch_len =
        (feeder.detector_stream_length() - STREAM_DETECTOR_WINDOW) / STREAM_DETECTOR_HOP + 1;
    assert_eq!(
        drained as i64, batch_len,
        "every detector quantum must be handed out exactly once"
    );
    // All samples are retained (9000 < ring bound), so every plan window
    // is materializable.
    for (index, batch) in batch_frames.iter().enumerate() {
        let plan = &plans[index];
        let raw = feeder
            .frame_raw_rows(plan, &[256, 2048])
            .unwrap_or_else(|e| panic!("frame {index} not ready: {e:?}"));
        let window_modes = if plan.current == 0 {
            (0, 0, 0)
        } else {
            (plan.previous, plan.current, plan.following)
        };
        let streamed: Vec<Vec<f64>> = raw
            .iter()
            .map(|row| {
                wem_analysis::dsp::transform::apply_vorbis_window(
                    row,
                    &[256, 2048],
                    window_modes.0,
                    window_modes.1,
                    window_modes.2,
                    Some(&frozen),
                )
                .expect("window applies")
            })
            .collect();
        if streamed != batch.samples {
            let (ch_row, bat_row) = streamed
                .iter()
                .zip(batch.samples.iter())
                .find(|(x, y)| x != y)
                .unwrap();
            let idx = ch_row
                .iter()
                .zip(bat_row.iter())
                .position(|(x, y)| x != y)
                .unwrap();
            panic!(
                "frame {index} diverges at idx {idx}: stream={} batch={} (plan start={}, end={}, total={})",
                ch_row[idx],
                bat_row[idx],
                plan.sample_start,
                plan.sample_end,
                feeder.total_samples()
            );
        }
    }
}

#[test]
fn streaming_detector_quanta_match_batch_views() {
    use wem_analysis::preprocessing::streaming::{
        STREAM_DETECTOR_HOP, STREAM_DETECTOR_WINDOW, STREAM_TAIL_SAMPLES,
    };

    let frames = 9000i64;
    let pcm = sine_like(frames);
    let batch_quanta = wem_analysis::preprocessing::detector_input::iter_detector_quanta(
        &pcm,
        STREAM_DETECTOR_HOP,
        STREAM_DETECTOR_WINDOW,
        None,
        STREAM_TAIL_SAMPLES,
        &[256, 2048],
    )
    .expect("batch quanta");

    let mut feeder = StreamingPcmFeeder::new(6, [256, 2048]).expect("feeder");
    let mut offset = 0usize;
    for chunk in [2000usize, 3000, 4000] {
        let rows: Vec<Vec<f64>> = pcm
            .iter()
            .map(|channel| channel[offset..offset + chunk].to_vec())
            .collect();
        offset += chunk;
        feeder.push(&rows).expect("push");
        let windows = feeder.drain_completed_quanta().expect("drain");
        let base = feeder.quanta_extracted_through() - windows.len() as i64;
        for (delta, window) in windows.iter().enumerate() {
            assert_eq!(
                window,
                &batch_quanta[(base + delta as i64) as usize],
                "quantum {} diverges from the batch detector stream",
                base + delta as i64
            );
        }
    }
    feeder.finish_source().expect("finish");
    let tail_windows = feeder.drain_completed_quanta().expect("drain");
    let base = feeder.quanta_extracted_through() - tail_windows.len() as i64;
    for (delta, window) in tail_windows.iter().enumerate() {
        assert_eq!(window, &batch_quanta[(base + delta as i64) as usize]);
    }
    assert_eq!(
        feeder.quanta_extracted_through() as usize,
        batch_quanta.len()
    );
}

#[test]
fn huge_single_chunk_keeps_every_quantum() {
    // One giant chunk (the bytes_parity regression shape): every quantum
    // must still be extractable as it completes, none lost to eviction.
    let frames = 30000i64;
    let pcm = sine_like(frames);
    let batch_quanta = wem_analysis::preprocessing::detector_input::iter_detector_quanta(
        &pcm,
        64,
        128,
        None,
        8192,
        &[256, 2048],
    )
    .expect("batch quanta");

    let mut feeder = StreamingPcmFeeder::new(6, [256, 2048]).expect("feeder");
    let rows: Vec<Vec<f64>> = pcm.clone();
    feeder.push(&rows).expect("one giant push");
    let windows = feeder.drain_completed_quanta().expect("drain");
    let base = feeder.quanta_extracted_through() - windows.len() as i64;
    for (delta, window) in windows.iter().enumerate() {
        assert_eq!(
            window,
            &batch_quanta[(base + delta as i64) as usize],
            "giant-chunk quantum {} diverges",
            base + delta as i64
        );
    }
    feeder.settle();
    feeder.finish_source().expect("finish");
    let tail_windows = feeder.drain_completed_quanta().expect("drain");
    let base = feeder.quanta_extracted_through() - tail_windows.len() as i64;
    for (delta, window) in tail_windows.iter().enumerate() {
        assert_eq!(window, &batch_quanta[(base + delta as i64) as usize],);
    }
    assert_eq!(
        feeder.quanta_extracted_through() as usize,
        batch_quanta.len()
    );
}
