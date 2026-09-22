//! The public error surface: its variants and the chain they expose.
//!
//! `variants` matches every public error enum with no `_` arm from a caller's
//! position, so adding a variant stops this build in the same commit that adds
//! it. `source_chain` walks `EncoderError` down through `InternalError` to the
//! kernel-stage error, requiring the innermost cause to be reachable and the
//! leaf to be the concrete inner error. Each module keeps its own test names
//! and assertion messages.

mod common;

mod variants {
    //! The public error enums are exhaustively matchable — and stay that way.
    //!
    //! `docs/reference/standards.md` (Errors) states the rule: a public error enum
    //! carries no `#[non_exhaustive]`, on purpose, because adding a variant is a
    //! breaking change — the compiler must tell every caller that a new failure
    //! mode exists. This file is that rule made compiler-enforced. Each function
    //! below matches one public error enum with **no `_` arm**, from an
    //! integration test crate, which is exactly the caller's position: it sees the
    //! public surface and nothing else.
    //!
    //! Adding a variant to any of these enums now fails to compile here, in the
    //! same commit that adds it, and the failure names the new failure mode.
    //!
    //! Two surfaces are deliberately *not* covered:
    //!
    //! * `wem_capi::WemError` — the C ABI error-code table. That surface has no
    //!   exhaustive matching by design (`include/wem.h` promises the values are
    //!   stable and append-only, never renumbered or reused), so adding a code is
    //!   not a breaking change and a match here would assert the opposite.
    //! * The `#[non_exhaustive]`-free enums of a dependency outside this
    //!   workspace: there are none.
    //!
    //! The conversion sites are the reason the primitives are listed too: the
    //! kernel's own code turns a `FloorFitError` into a `PacketError` at the call
    //! site (`wem-core/src/pack.rs`), so a caller of this crate is a caller of
    //! that enum as well.

    // The matches are the test; nothing needs to run them. `dead_code` is off for
    // exactly that reason, so a variant list can never be "unused" into silence.
    #![allow(dead_code)]

    use wem_analysis::config::AnalysisError;
    use wem_container::error::ContainerError;
    use wem_core::error::{EncoderError, InternalError};
    use wem_profiles::error::ProfileError;
    use wem_scheduling::{PlannerError, SelectorError};
    use wem_vorbis::bitio::BitError;
    use wem_vorbis::codebook::CodebookError;
    use wem_vorbis::floor::Floor1Error;
    use wem_vorbis::floor_fit::FloorFitError;
    use wem_vorbis::packet_encoder::PacketError;
    use wem_vorbis::residue::ResidueError;
    use wem_vorbis::setup::SetupError;

    /// `wem_core::error::EncoderError` — 6 variants.
    pub fn encoder_error(value: &EncoderError) {
        match value {
            EncoderError::ProfileNotFound { .. } => {}
            EncoderError::StateError { .. } => {}
            EncoderError::GeometryMismatch { .. } => {}
            EncoderError::InputTooShort { .. } => {}
            EncoderError::FormatUnsupported { .. } => {}
            EncoderError::Internal(..) => {}
        }
    }

    /// `wem_core::error::InternalError` — 6 variants.
    pub fn internal_error(value: &InternalError) {
        match value {
            InternalError::Profile(..) => {}
            InternalError::Analysis(..) => {}
            InternalError::Packet(..) => {}
            InternalError::Container(..) => {}
            InternalError::Invariant { .. } => {}
            InternalError::Io { .. } => {}
        }
    }

    /// `wem_profiles::error::ProfileError` — 7 variants.
    pub fn profile_error(value: &ProfileError) {
        match value {
            ProfileError::Identity { .. } => {}
            ProfileError::Selection { .. } => {}
            ProfileError::Tables { .. } => {}
            ProfileError::Quality { .. } => {}
            ProfileError::Analysis(..) => {}
            ProfileError::Codebook(..) => {}
            ProfileError::Bit(..) => {}
        }
    }

    /// `wem_analysis::config::AnalysisError` — 5 variants.
    pub fn analysis_error(value: &AnalysisError) {
        match value {
            AnalysisError::Configuration { .. } => {}
            AnalysisError::Geometry { .. } => {}
            AnalysisError::Input { .. } => {}
            AnalysisError::State { .. } => {}
            AnalysisError::Invariant { .. } => {}
        }
    }

    /// `wem_vorbis::packet_encoder::PacketError` — 7 variants.
    pub fn packet_error(value: &PacketError) {
        match value {
            PacketError::Structure { .. } => {}
            PacketError::Analysis { .. } => {}
            PacketError::Coupling { .. } => {}
            PacketError::Codebook(..) => {}
            PacketError::Floor1(..) => {}
            PacketError::FloorFit(..) => {}
            PacketError::Residue(..) => {}
        }
    }

    /// `wem_container::error::ContainerError` — 12 variants.
    pub fn container_error(value: &ContainerError) {
        match value {
            ContainerError::NotRiff => {}
            ContainerError::TruncatedRiff { .. } => {}
            ContainerError::BadChunkId { .. } => {}
            ContainerError::FmtTooShort { .. } => {}
            ContainerError::FmtSize { .. } => {}
            ContainerError::BadSeekTableSize { .. } => {}
            ContainerError::PacketTooLarge { .. } => {}
            ContainerError::TruncatedPacketStream { .. } => {}
            ContainerError::MissingChunk { .. } => {}
            ContainerError::PacketWalkFailed { .. } => {}
            ContainerError::BadEndian { .. } => {}
            ContainerError::TerminalExcessTooLarge { .. } => {}
        }
    }

    /// `wem_vorbis::codebook::CodebookError` — 20 variants.
    pub fn codebook_error(value: &CodebookError) {
        match value {
            CodebookError::RowMissingField { .. } => {}
            CodebookError::LengthlistMismatch { .. } => {}
            CodebookError::DimTooSmall { .. } => {}
            CodebookError::OverpopulatedTree { .. } => {}
            CodebookError::LengthOutOfRange { .. } => {}
            CodebookError::CodePrefixCollision { .. } => {}
            CodebookError::CodeEmbedsLeaf { .. } => {}
            CodebookError::Maptype1RequiresQuantlist => {}
            CodebookError::QuantvalsNonPositive => {}
            CodebookError::QuantlistTooShort { .. } => {}
            CodebookError::EntryOutOfRange { .. } => {}
            CodebookError::EntryUnused { .. } => {}
            CodebookError::EmptyCodebook => {}
            CodebookError::InvalidHuffmanCode => {}
            CodebookError::NotMaptype1 { .. } => {}
            CodebookError::NoVqVector { .. } => {}
            CodebookError::VqMisuse { .. } => {}
            CodebookError::TargetTooShort { .. } => {}
            CodebookError::NoUsableVqEntries => {}
            CodebookError::PackBitsOutOfRange { .. } => {}
        }
    }

    /// `wem_vorbis::bitio::BitError` — 2 variants.
    pub fn bit_error(value: &BitError) {
        match value {
            BitError::OutOfBits => {}
            BitError::BitsOutOfRange { .. } => {}
        }
    }

    /// `wem_vorbis::setup::SetupError` — 2 variants.
    pub fn setup_error(value: &SetupError) {
        match value {
            SetupError::FieldMissing { .. } => {}
            SetupError::Bit(..) => {}
        }
    }

    /// `wem_vorbis::floor::Floor1Error` — 3 variants.
    pub fn floor1_error(value: &Floor1Error) {
        match value {
            Floor1Error::LengthMismatch { .. } => {}
            Floor1Error::TooFewPosts { .. } => {}
            Floor1Error::InvalidMultiplier { .. } => {}
        }
    }

    /// `wem_vorbis::floor_fit::FloorFitError` — 1 variants.
    pub fn floor_fit_error(value: &FloorFitError) {
        match value {
            FloorFitError::CurvesTooShort => {}
        }
    }

    /// `wem_vorbis::residue::ResidueError` — 9 variants.
    pub fn residue_error(value: &ResidueError) {
        match value {
            ResidueError::SilentRequiresEmptyCascade => {}
            ResidueError::UnsupportedClassCount { .. } => {}
            ResidueError::BookIndexOutOfRange { .. } => {}
            ResidueError::UnencodableClassword { .. } => {}
            ResidueError::InvalidPartitionSize => {}
            ResidueError::InvalidChannelCount => {}
            ResidueError::ChannelLayoutMismatch { .. } => {}
            ResidueError::ClassTablesTooShort { .. } => {}
            ResidueError::IncompatibleBookDimension { .. } => {}
        }
    }

    /// `wem_scheduling::planner::PlannerError` — 7 variants.
    pub fn planner_error(value: &PlannerError) {
        match value {
            PlannerError::BlockSizeCount => {}
            PlannerError::BlockSizeNotEven => {}
            PlannerError::ModeNotBit => {}
            PlannerError::AppendCountNegative => {}
            PlannerError::InsufficientSamples { .. } => {}
            PlannerError::NonPositiveAdvance => {}
            PlannerError::StateDiverged => {}
        }
    }

    /// `wem_scheduling::selector::SelectorError` — 6 variants.
    pub fn selector_error(value: &SelectorError) {
        match value {
            SelectorError::HopCapacityNonPositive => {}
            SelectorError::QueueSlotOutOfRange { .. } => {}
            SelectorError::GenerationGeometry => {}
            SelectorError::TransitionLookModesInvalid => {}
            SelectorError::MissingFlagRows => {}
            SelectorError::SurplusFlagRows => {}
        }
    }

    #[test]
    fn a_real_fault_reaches_a_callers_own_arms() {
        // One run through the caller's view, so this file is not only a
        // compile-time guard: a real resolution failure arrives at the caller's
        // own arm, and the match that classified it stays exhaustive.
        let error = wem_core::Encoder::new(
            wem_profiles::WwiseProfile::new(wem_profiles::WwiseVersion::Wwise2013, 2, 44_100)
                .expect("positive geometry"),
        )
        .expect_err("2ch/44100 is not an installed configuration");
        match &error {
            EncoderError::ProfileNotFound { .. } => {}
            other => panic!("expected ProfileNotFound, got {other}"),
        }
        encoder_error(&error);
        internal_error(&InternalError::Invariant { message: "x" });
        profile_error(&ProfileError::quality("quality must be a finite number"));
        container_error(&ContainerError::NotRiff);
        packet_error(&PacketError::Structure {
            message: "master book is missing for class 0".to_string(),
        });
        codebook_error(&CodebookError::Maptype1RequiresQuantlist);
        bit_error(&BitError::OutOfBits);
        setup_error(&SetupError::FieldMissing { field: "x" });
        floor1_error(&Floor1Error::TooFewPosts { posts: 1 });
        floor_fit_error(&FloorFitError::CurvesTooShort);
        residue_error(&ResidueError::InvalidPartitionSize);
        planner_error(&PlannerError::BlockSizeCount);
        selector_error(&SelectorError::HopCapacityNonPositive);
        analysis_error(&AnalysisError::geometry(
            "psy curve length mismatch (want=2, got=1)",
        ));
    }
}

mod classification {
    use wem_analysis::config::{AnalysisError, AnalysisProfileResources};
    use wem_analysis::dsp::psy_geom::ath;
    use wem_analysis::preprocessing::streaming::StreamingPcmFeeder;
    use wem_analysis::preprocessing::windowing::PlannedWindowSource;
    use wem_analysis::session::AnalysisSession;
    use wem_analysis::transient::detector::TransientDetector;
    use wem_core::Encoder;
    use wem_profiles::{WwiseProfile, WwiseVersion};

    use crate::common::fixture_selection;

    #[test]
    fn real_rejections_keep_input_geometry_and_selection_categories() {
        assert!(matches!(
            StreamingPcmFeeder::new(0, [256, 2048]),
            Err(AnalysisError::Input { .. })
        ));
        assert!(matches!(ath(1, 0), Err(AnalysisError::Geometry { .. })));
        assert!(matches!(
            WwiseProfile::new(WwiseVersion::Wwise2013, 0, 44_100),
            Err(wem_profiles::error::ProfileError::Selection { .. })
        ));
    }

    #[test]
    fn caller_pcm_errors_report_actual_channels_and_lengths() {
        let mut feeder = StreamingPcmFeeder::new(2, [256, 2048]).unwrap();
        assert!(matches!(
            feeder.push(&[vec![0.0; 3]]),
            Err(AnalysisError::Input { .. })
        ));
        let error = feeder.push(&[vec![0.0; 3], vec![0.0; 2]]).unwrap_err();
        assert!(matches!(error, AnalysisError::Input { .. }));
        assert_eq!(error.to_string(), "PCM channel 1 has 2 frames, expected 3");

        let plans = wem_scheduling::plan_mode_sequence(&[0, 0], &[256, 2048], 1).unwrap();
        let Err(error) = PlannedWindowSource::new(
            &[vec![0.0; 4096], vec![0.0; 4095]],
            &plans,
            &[256, 2048],
            None,
        ) else {
            panic!("unequal PCM channel lengths were accepted");
        };
        assert!(matches!(error, AnalysisError::Input { .. }));
        assert_eq!(
            error.to_string(),
            "PCM channel 1 has 4095 frames, expected 4096"
        );
    }

    #[test]
    fn session_and_detector_distinguish_bad_input_from_bad_state() {
        let encoder = Encoder::new(fixture_selection()).unwrap();
        let resources = encoder.analysis_resources().clone();
        let mut detector = TransientDetector::new(
            6,
            resources.transient.clone(),
            resources.mdct_looks[&128].clone(),
            128,
        )
        .unwrap();
        assert!(matches!(
            detector.analyze_quantum(&[vec![0.0; 128]], 0),
            Err(AnalysisError::Input { .. })
        ));
        let mut quantum = vec![vec![0.0; 128]; 6];
        quantum[2].pop();
        let error = detector.analyze_quantum(&quantum, 0).unwrap_err();
        assert!(matches!(error, AnalysisError::Input { .. }));
        assert_eq!(
            error.to_string(),
            "detector PCM channel 2 has 127 samples, expected 128"
        );

        let mut session = AnalysisSession::new(6, 44100, [256, 2048], resources).unwrap();
        assert!(matches!(
            session.select_modes(&vec![vec![0.0; 4]; 6]),
            Err(AnalysisError::Input { .. })
        ));
        let mut pcm = vec![vec![0.0; 4096]; 6];
        pcm[3].pop();
        let error = session.select_modes(&pcm).unwrap_err();
        assert!(matches!(error, AnalysisError::Input { .. }));
        assert_eq!(
            error.to_string(),
            "PCM channel 3 has 4095 frames, expected 4096"
        );
        assert!(matches!(
            session.finalize_terminal_transition(0, 1),
            Err(AnalysisError::State { .. })
        ));
    }

    #[test]
    fn inconsistent_quality_metadata_is_a_configuration_error() {
        let encoder = Encoder::new(fixture_selection()).unwrap();
        let resources = encoder.analysis_resources().clone();
        let error = AnalysisProfileResources::new(
            resources.mdct_looks,
            resources.transient,
            resources.short_profiles,
            resources.short_surface,
            resources.short_look,
            resources.long_base,
            resources.long_variants,
            resources.long_floor_looks,
            resources.input_conditioner,
            resources.frozen,
            None,
            true,
        )
        .unwrap_err();
        assert!(matches!(error, AnalysisError::Configuration { .. }));
        assert!(error.to_string().contains("quality value"));
    }

    #[test]
    fn nonfinite_caller_quality_is_never_internal() {
        for quality in [f64::NAN, f64::INFINITY, f64::NEG_INFINITY] {
            let error = Encoder::new_with_quality(fixture_selection(), Some(quality)).unwrap_err();
            assert!(matches!(
                error,
                wem_core::error::EncoderError::StateError { .. }
            ));
            assert_eq!(error.to_string(), "quality must be a finite number");
        }
    }

    #[test]
    fn malformed_analysis_resources_report_observed_geometry() {
        let encoder = Encoder::new(fixture_selection()).unwrap();
        let resources = encoder.analysis_resources();
        assert!(matches!(
            TransientDetector::new(
                6,
                resources.transient.clone(),
                resources.mdct_looks[&128].clone(),
                64,
            ),
            Err(AnalysisError::Geometry { message })
                if message == "transient detector bins (want=128, got=64)"
        ));
        let mut table = resources.long_base.clone();
        table.analysis_profile_u32.truncate(167);
        let error = wem_analysis::config::make_long_floor_envelope_look(&table).unwrap_err();
        assert!(matches!(error, AnalysisError::Configuration { .. }));
        assert_eq!(
            error.to_string(),
            "long floor scalars require at least 168 values, got 167"
        );
    }
}

mod source_chain {
    //! The public error chain must reach the innermost cause.
    //!
    //! `EncoderError` wraps `InternalError`, which wraps the kernel-stage error
    //! (`ProfileError`, `AnalysisError`, `PacketError`, `ContainerError`). With
    //! empty `Error` impls every level reported `source() == None`, so a caller
    //! holding an `EncoderError` could not get from "encoder fault" to the stage
    //! failure that produced it. These suites pin the chain: the layers are
    //! walked through `source()`, and the leaf is the concrete inner error.

    use std::error::Error;

    use wem_analysis::config::AnalysisError;
    use wem_core::error::{EncoderError, InternalError};
    use wem_core::Encoder;
    use wem_profiles::error::ProfileError;

    use crate::common::fixture_selection;

    /// The message chain reachable from `error` through `source()`, head first.
    fn chain(error: &(dyn Error + 'static)) -> Vec<String> {
        let mut messages = vec![error.to_string()];
        let mut current = error.source();
        while let Some(cause) = current {
            messages.push(cause.to_string());
            current = cause.source();
        }
        messages
    }

    #[test]
    fn a_real_selection_fault_reaches_the_profile_error() {
        // A quality request on the 6ch/44100 configuration: that profile ships no
        // quality-curves resource, so profile assembly is where the kernel fails.
        let error = Encoder::new_with_quality(fixture_selection(), Some(0.5))
            .expect_err("the 6ch/44100 profile has no quality-curves resource");

        let internal = error
            .source()
            .expect("EncoderError::Internal must expose its InternalError")
            .downcast_ref::<InternalError>()
            .unwrap_or_else(|| {
                panic!("EncoderError::source() is not the InternalError: {error:?}")
            });
        assert!(
            matches!(internal, InternalError::Profile(_)),
            "expected an InternalError::Profile fault, got {internal}"
        );

        let profile = internal
            .source()
            .expect("InternalError::Profile must expose its ProfileError")
            .downcast_ref::<ProfileError>()
            .unwrap_or_else(|| {
                panic!("InternalError::source() is not the ProfileError: {internal:?}")
            });
        assert!(
            matches!(profile, ProfileError::Quality { .. }),
            "unexpected leaf profile error: {profile}"
        );
        assert!(
            profile.source().is_none(),
            "a leaf profile error must not invent a cause: {profile}"
        );

        let messages = chain(&error);
        assert_eq!(
            messages.len(),
            3,
            "expected encoder -> internal -> profile, got {messages:?}"
        );
        assert!(
            messages[0].starts_with("encoder fault: "),
            "the head message must stay the EncoderError diagnostic: {messages:?}"
        );
        assert!(
            messages[1].contains("quality-curves"),
            "the InternalError message must name the stage fault: {messages:?}"
        );
        assert!(
            messages[2].contains("quality-curves"),
            "the innermost cause must be reachable through source(): {messages:?}"
        );
    }

    #[test]
    fn every_wrapping_layer_appears_in_the_chain() {
        let innermost = AnalysisError::geometry("psy curve length mismatch (want=1024, got=512)");
        let error = EncoderError::Internal(InternalError::Profile(ProfileError::Analysis(
            innermost.clone(),
        )));

        let messages = chain(&error);
        assert_eq!(
            messages.len(),
            4,
            "expected encoder -> internal -> profile -> analysis, got {messages:?}"
        );
        assert_eq!(
            messages[3],
            innermost.to_string(),
            "the analysis error's own Display text must end the chain"
        );
        assert!(
            messages[3].contains("1024") && messages[3].contains("512"),
            "the analysis message must carry its observed values: {messages:?}"
        );
        assert!(
            !messages[3].contains("PsyCurveLengthMismatch"),
            "AnalysisError's Display must be a message, not its Debug rendering: {messages:?}"
        );
        assert!(
            messages[2].starts_with("analysis error: "),
            "the profile layer must keep its message prefix: {messages:?}"
        );
    }

    #[test]
    fn packet_failure_keeps_the_nested_codec_cause() {
        use wem_vorbis::codebook::CodebookError;
        use wem_vorbis::packet_encoder::PacketError;
        let error = EncoderError::Internal(InternalError::Packet(PacketError::Codebook(
            CodebookError::EmptyCodebook,
        )));
        assert_eq!(chain(&error).len(), 4);
        let cause = error.source().unwrap().source().unwrap().source().unwrap();
        assert!(matches!(
            cause.downcast_ref::<CodebookError>(),
            Some(CodebookError::EmptyCodebook)
        ));
    }
}
