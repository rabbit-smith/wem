//! The kernel's long-lived public types are printable for a caller writing
//! diagnostics, and their `Debug` output is a *summary*.
//!
//! Both halves matter, and both are checked here:
//!
//! * the trait is required at compile time for every type a caller holds
//!   across calls (`Encoder`, `StreamSession`) and for the state types the
//!   analysis and scheduling domains expose (`AnalysisSession`,
//!   `TransientDetector`, `ModeSelector`, `InputConditioner`,
//!   `StreamingPcmFeeder`), so dropping a `Debug` impl fails this build;
//! * the rendering is bounded: these values carry frozen profile tables and
//!   per-frame vectors, and "printable" must not mean "prints a hundred
//!   thousand numbers". Each case asserts the length of the rendered string,
//!   which a table dump cannot satisfy.

use wem_analysis::config::InputConditionerConfig;
use wem_analysis::preprocessing::conditioner::InputConditioner;
use wem_analysis::preprocessing::streaming::StreamingPcmFeeder;
use wem_analysis::session::AnalysisSession;
use wem_analysis::transient::detector::TransientDetector;
use wem_core::{Encoder, StreamSession, WwiseProfile, WwiseVersion};
use wem_scheduling::ModeSelector;

/// The fixture selection: 6ch/44.1kHz, the compiled configuration.
fn selection() -> WwiseProfile {
    WwiseProfile::new(WwiseVersion::Wwise2013, 6, 44_100).expect("fixture geometry is in domain")
}

fn encoder() -> Encoder {
    Encoder::new(selection()).expect("6ch/44100 is a compiled configuration")
}

/// What "summary" means here: a bounded rendering. The values below hold
/// hundreds of table entries and thousands of samples, so anything in the
/// thousands of bytes is a dump.
const SUMMARY_LIMIT: usize = 512;

fn assert_summary<T: std::fmt::Debug>(value: &T, must_contain: &[&str]) -> String {
    let rendered = format!("{value:?}");
    assert!(
        rendered.len() < SUMMARY_LIMIT,
        "Debug for {} must summarise, not dump: {} bytes rendered: {rendered}",
        std::any::type_name::<T>(),
        rendered.len()
    );
    for needle in must_contain {
        assert!(
            rendered.contains(needle),
            "Debug for {} must carry {needle:?}: {rendered}",
            std::any::type_name::<T>()
        );
    }
    rendered
}

#[test]
fn encoder_renders_its_selection_and_resource_sizes() {
    let rendered = assert_summary(
        &encoder(),
        &["Encoder", "channels: 6", "sample_rate: 44100", "codebooks:"],
    );
    // The frozen tables are reported by size, never printed: the setup packet
    // is a few hundred bytes and the analysis tables are thousands of values.
    assert!(
        rendered.contains("setup_packet_len:"),
        "the setup packet is summarised by length: {rendered}"
    );
}

#[test]
fn stream_session_renders_its_lifecycle_position() {
    // The unopened state a shell constructs before a selection is known.
    assert_summary(
        &StreamSession::new(),
        &["StreamSession", "initialized: false", "finished: false"],
    );

    let mut opened = StreamSession::for_selection(selection()).expect("selection resolves");
    let rendered = assert_summary(
        &opened,
        &["StreamSession", "initialized: true", "pcm_frames: 0"],
    );
    assert!(
        !rendered.contains("modes"),
        "the per-frame vectors are not printed: {rendered}"
    );

    // The terminal position reads the same way: the summary is the lifecycle
    // state, not the packets that went through it.
    let minimum = vec![0u8; 4096 * 6 * 2];
    opened
        .push_pcm_chunk(&minimum)
        .expect("a 4096-frame chunk is accepted");
    opened.finish().expect("the minimum stream finishes");
    assert_summary(
        &opened,
        &[
            "StreamSession",
            "initialized: true",
            "finished: true",
            "pcm_frames: 4096",
        ],
    );
}

#[test]
fn analysis_state_types_render_summaries() {
    let encoder = encoder();
    let resources = encoder.analysis_resources().clone();

    let session = AnalysisSession::new(6, 44_100, [256, 2048], resources.clone())
        .expect("fixture geometry builds an analysis session");
    assert_summary(
        &session,
        &[
            "AnalysisSession",
            "channels: 6",
            "sample_rate: 44100",
            "next_frame_index: 0",
        ],
    );

    let mdct = resources.mdct_looks[&128].clone();
    let detector = TransientDetector::new(6, resources.transient.clone(), mdct, 128)
        .expect("fixture detector geometry");
    assert_summary(
        &detector,
        &["TransientDetector", "channels: 6", "bins: 128", "quanta: 0"],
    );

    let conditioner = InputConditioner::new(
        6,
        &InputConditionerConfig::new(0.5).expect("coefficient is in domain"),
    )
    .expect("fixture conditioner geometry");
    assert_summary(
        &conditioner,
        &["InputConditioner", "coefficient: 0.5", "channels: 6"],
    );

    let feeder = StreamingPcmFeeder::new(6, [256, 2048]).expect("fixture feeder geometry");
    assert_summary(
        &feeder,
        &["StreamingPcmFeeder", "channels: 6", "total_samples: 0"],
    );
}

#[test]
fn mode_selector_renders_every_counter() {
    let selector =
        ModeSelector::new(64, 128, 0, 0, 0, 1024, vec![0; 128]).expect("fixture selector geometry");
    assert_summary(
        &selector,
        &[
            "ModeSelector",
            "hop: 64",
            "capacity: 128",
            "generated: 0",
            "selected: 0",
            "queue_len: 128",
        ],
    );
}

/// The trait bound, stated once for every type: this function's body is never
/// called, and the bound is checked when it is instantiated.
#[test]
fn every_long_lived_type_is_debug() {
    fn assert_debug<T: std::fmt::Debug>() {}
    assert_debug::<Encoder>();
    assert_debug::<StreamSession>();
    assert_debug::<AnalysisSession>();
    assert_debug::<TransientDetector>();
    assert_debug::<ModeSelector>();
    assert_debug::<InputConditioner>();
    assert_debug::<StreamingPcmFeeder>();
}
