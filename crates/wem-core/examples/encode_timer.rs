//! Fixture-encode timer for wem-core.
//!
//! Splits one fixture encode into its pipeline stages so regression
//! reports can name the hot stage instead of guessing
//! (crates/AGENTS.md: "wem-core keeps a fixture-encode timer").
//!
//! Run:
//!   cargo run -p wem-core --release --example encode_timer

use std::time::Instant;

use wem_analysis::session::AnalysisSession;
use wem_container::wem::build_vorbis_wem;
use wem_core::pack::pack_analysis_packet;
use wem_core::{Encoder, Pcm16, MIN_PCM_FRAMES};

fn main() {
    let fixtures = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../..")
        .join("tests/fixtures");
    let raw = std::fs::read(fixtures.join("input.wav")).expect("input.wav reads");
    let wav = wem_core::usecases::wav::parse_pcm16(&raw).expect("input.wav parses");
    let pcm: Pcm16 = wav.to_pcm16().expect("pcm16 converts");
    assert!(
        pcm.frame_count() >= MIN_PCM_FRAMES as i64,
        "fixture must satisfy the frame minimum"
    );

    let encoder = Encoder::from_profile("wwise2013-6ch-44100").expect("profile loads");

    // Warm-up: page faults, allocator growth, I-cache.
    let warmup = encoder.encode_pcm(&pcm).expect("warm-up encode");
    assert_eq!(
        warmup.data.len(),
        108771,
        "warm-up encode must match the golden size"
    );

    let mut session = AnalysisSession::new(
        encoder.profile().channels(),
        encoder.profile().sample_rate(),
        encoder.profile().block_sizes(),
        encoder.analysis_resources().clone(),
    )
    .expect("analysis session builds");

    let rows = pcm.to_float_rows();

    let start = Instant::now();
    let (modes, windows) = session.selected_windows(&rows).expect("modes select");
    let mode_selection_ms = start.elapsed().as_secs_f64() * 1e3;

    let start = Instant::now();
    let analyses: Vec<_> = windows
        .into_iter()
        .map(|window| {
            session
                .analyze_window(window, None)
                .expect("frame analyzes")
        })
        .collect();
    let analysis_ms = start.elapsed().as_secs_f64() * 1e3;

    let start = Instant::now();
    let audio_packets: Vec<Vec<u8>> = analyses
        .iter()
        .map(|analysis| {
            pack_analysis_packet(
                encoder.setup(),
                encoder.codebooks(),
                analysis,
                encoder.profile().channels() as u32,
            )
            .expect("frame packs")
        })
        .collect();
    let vorbis_packing_ms = start.elapsed().as_secs_f64() * 1e3;

    let start = Instant::now();
    let mut fields = *encoder.container_plan().fmt();
    fields.dw_total_pcm_frames = pcm.frame_count() as u32;
    let mut packets = vec![encoder.setup_packet().to_vec()];
    packets.extend(audio_packets);
    let _built = build_vorbis_wem(
        fields,
        &packets,
        encoder.container_plan().seek_table(),
        encoder.container_plan().endian(),
        encoder.container_plan().extra_chunks(),
        true,
        None,
    )
    .expect("container builds");
    let container_ms = start.elapsed().as_secs_f64() * 1e3;

    let short = modes.iter().filter(|&&m| m == 0).count();
    let long = modes.iter().filter(|&&m| m == 1).count();
    println!(
        "encode_timer: mode_selection {:.2}ms | analysis {:.2}ms | vorbis_packing {:.2}ms | container {:.2}ms | ({} packets: {} short / {} long)",
        mode_selection_ms,
        analysis_ms,
        vorbis_packing_ms,
        container_ms,
        short + long,
        short,
        long,
    );
}
