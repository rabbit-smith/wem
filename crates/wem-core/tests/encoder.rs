//! The batch encoder: its PCM shapes, its plan, and the bytes it produces.
//!
//! Six angles on one object. `pcm_shapes` pins the three accepted PCM forms and
//! the identical rows and containers they must reach; `mode_tail` pins the rule
//! that decides the last frame a source length produces; `reference_bytes`
//! encodes the committed fixture and requires the reference container byte for
//! byte; `two_channel_bytes` does the same for the installed 2ch/48000
//! selection through both the batch and the streaming entry; `profile_registration`
//! pins that configuration's setup packet against the committed container; and
//! `quality_binding` pins that a bound quality is an additive copy that reaches
//! the analysis assembly. Each module keeps its own test names and assertion
//! messages.

mod common;

mod pcm_shapes {
    //! PCM input handling (wem-core).
    //!
    //! Two things are pinned here:
    //! * the rejection conditions of the byte-backed constructors
    //!   (`from_channel_major_le` / `from_interleaved_le`);
    //! * the shape equivalence: the same PCM handed in as channel-major i16 rows,
    //!   as channel-major little-endian bytes, or as interleaved little-endian
    //!   bytes must produce the identical float rows — and therefore the identical
    //!   container bytes. The row shape is the reference: it is the form every
    //!   caller used before the byte-backed shapes existed.

    use wem_core::encoder::{Encoder, Pcm16};
    use wem_core::error::EncoderError;
    use wem_core::usecases::wav::read_pcm16;

    use crate::common::{fixture_selection, fixtures_dir, read_fixture};

    /// Transpose interleaved little-endian s16 bytes into channel-major bytes
    /// (byte level only: no `i16` participates, so the kernel's own decode is what
    /// is under test).
    fn channel_major_from_interleaved(interleaved: &[u8], channels: usize) -> Vec<u8> {
        let frames = interleaved.len() / (channels * 2);
        let mut out = vec![0u8; interleaved.len()];
        for frame in 0..frames {
            for channel in 0..channels {
                let src = (frame * channels + channel) * 2;
                let dst = (channel * frames + frame) * 2;
                out[dst] = interleaved[src];
                out[dst + 1] = interleaved[src + 1];
            }
        }
        out
    }

    #[test]
    fn interleaved_pcm_rejects_unrepresentable_channel_count() {
        let error = Pcm16::from_interleaved_le(48_000, usize::MAX, [0, 0])
            .expect_err("channel-byte multiplication must be checked");
        assert!(matches!(error, EncoderError::StateError { .. }));

        let error = Pcm16::from_interleaved_le(48_000, usize::MAX / 2, [])
            .expect_err("empty PCM must be rejected before channel allocation");
        assert!(matches!(error, EncoderError::StateError { .. }));

        let error = Pcm16::from_channel_major_le(48_000, usize::MAX, [0, 0])
            .expect_err("channel-byte multiplication must be checked here too");
        assert!(matches!(error, EncoderError::StateError { .. }));
    }

    #[test]
    fn interleaved_pcm_rejects_zero_channels_and_partial_frames() {
        assert!(matches!(
            Pcm16::from_interleaved_le(48_000, 0, []),
            Err(EncoderError::StateError { .. })
        ));
        assert!(matches!(
            Pcm16::from_interleaved_le(48_000, 2, [0, 0, 0]),
            Err(EncoderError::GeometryMismatch { .. })
        ));
    }

    /// The new shape carries exactly the validation `new` /
    /// `from_interleaved_le` carry: a positive rate, at least one channel, at
    /// least one frame, and a byte length that is an exact multiple of the frame
    /// width (a trailing partial frame is GEOMETRY_MISMATCH).
    #[test]
    fn channel_major_pcm_rejects_malformed_geometry() {
        // Zero channels.
        assert!(matches!(
            Pcm16::from_channel_major_le(48_000, 0, []),
            Err(EncoderError::StateError { .. })
        ));
        assert!(matches!(
            Pcm16::from_channel_major_le(48_000, 0, [0, 0]),
            Err(EncoderError::StateError { .. })
        ));
        // Zero frames (empty byte buffer).
        assert!(matches!(
            Pcm16::from_channel_major_le(48_000, 2, []),
            Err(EncoderError::StateError { .. })
        ));
        // 6 bytes into 2-channel frames of 4 bytes: a trailing partial frame.
        assert!(matches!(
            Pcm16::from_channel_major_le(48_000, 2, [0, 0, 0, 0, 0, 0]),
            Err(EncoderError::GeometryMismatch { .. })
        ));
        // Non-positive rates are rejected, exactly as `new` rejects them.
        for sample_rate in [0, -44_100] {
            assert!(matches!(
                Pcm16::from_channel_major_le(sample_rate, 2, [0, 0, 0, 0]),
                Err(EncoderError::StateError { .. })
            ));
            assert!(matches!(
                Pcm16::from_interleaved_le(sample_rate, 2, [0, 0, 0, 0]),
                Err(EncoderError::StateError { .. })
            ));
        }
    }

    /// Small, non-square, every sample distinct (both signed-16 extremes
    /// included): a transposition or an off-by-one offset would land a different
    /// value at some `(channel, frame)` — and the expected rows spell out the
    /// normalization statement the kernel must use.
    #[test]
    fn the_three_pcm_shapes_share_one_float_row_view() {
        let channels = 3usize;
        let frames = 5usize;
        let sample_rate = 44_100i64;

        let mut values: Vec<Vec<i16>> = (0..channels)
            .map(|channel| {
                (0..frames)
                    .map(|frame| ((channel * frames + frame) as i16) * 997 - 2_000)
                    .collect()
            })
            .collect();
        values[0][0] = i16::MIN;
        values[channels - 1][frames - 1] = i16::MAX;

        let expected: Vec<Vec<f64>> = values
            .iter()
            .map(|row| row.iter().map(|value| *value as f64 / 32768.0).collect())
            .collect();
        let channel_major: Vec<u8> = values
            .iter()
            .flatten()
            .flat_map(|value| value.to_le_bytes())
            .collect();
        let mut interleaved = Vec::with_capacity(channels * frames * 2);
        for frame in 0..frames {
            for row in &values {
                interleaved.extend_from_slice(&row[frame].to_le_bytes());
            }
        }

        let row_pcm = Pcm16::new(sample_rate, values).expect("rows build");
        // The byte buffers were built for this one comparison: they are handed
        // over by value, which is the shape the kernel now accepts.
        let channel_pcm = Pcm16::from_channel_major_le(sample_rate, channels, channel_major)
            .expect("bytes build");
        let interleaved_pcm =
            Pcm16::from_interleaved_le(sample_rate, channels, interleaved).expect("bytes build");

        // Geometry is reported identically by every shape.
        for pcm in [&row_pcm, &channel_pcm, &interleaved_pcm] {
            assert_eq!(pcm.sample_rate(), sample_rate);
            assert_eq!(pcm.channel_count(), channels);
            assert_eq!(pcm.frame_count(), frames as i64);
        }

        // The whole point: the float rows are identical, sample for sample.
        for (name, pcm) in [
            ("channel-major bytes", &channel_pcm),
            ("interleaved bytes", &interleaved_pcm),
        ] {
            assert_eq!(
                pcm.to_float_rows(),
                expected,
                "{name} float rows differ from the row-major reference"
            );
        }
        assert_eq!(row_pcm.to_float_rows(), expected);

        // Equality is shape-independent: same rate, geometry and samples.
        assert_eq!(row_pcm, channel_pcm);
        assert_eq!(row_pcm, interleaved_pcm);
        assert_ne!(
            row_pcm,
            Pcm16::new(sample_rate, vec![vec![0i16; frames]; channels]).expect("zeros build")
        );
    }

    /// The fixture PCM in all three shapes: one encode each, byte-identical to the
    /// recorded container (the row shape's bytes are the reference).
    #[test]
    fn the_three_pcm_shapes_encode_to_identical_containers() {
        let wav = read_pcm16(&fixtures_dir().join("input.wav")).expect("input.wav reads");
        assert_eq!(wav.channels(), 6);
        assert_eq!(wav.sample_rate(), 44_100);
        let interleaved = wav.interleaved_le_bytes();
        let channel_major = channel_major_from_interleaved(interleaved, wav.channels());

        let rows: Vec<Vec<i16>> = (0..wav.channels())
            .map(|channel| {
                (0..wav.frames())
                    .map(|frame| {
                        let offset = (frame * wav.channels() + channel) * 2;
                        i16::from_le_bytes([interleaved[offset], interleaved[offset + 1]])
                    })
                    .collect()
            })
            .collect();

        let row_pcm = Pcm16::new(44_100, rows).expect("rows build");
        let channel_pcm = Pcm16::from_channel_major_le(44_100, wav.channels(), channel_major)
            .expect("channel-major bytes build");
        let interleaved_pcm = Pcm16::from_interleaved_le(44_100, wav.channels(), interleaved)
            .expect("interleaved bytes build");
        assert_eq!(row_pcm, channel_pcm);
        assert_eq!(row_pcm, interleaved_pcm);

        let encoder = Encoder::new(fixture_selection()).expect("selection resolves");
        let reference = row_pcm.to_float_rows();
        let mut containers = Vec::new();
        for (name, pcm) in [
            ("row-major", &row_pcm),
            ("channel-major bytes", &channel_pcm),
            ("interleaved bytes", &interleaved_pcm),
        ] {
            assert_eq!(
                pcm.to_float_rows(),
                reference,
                "{name} float rows differ from the row-major reference"
            );
            let result = encoder.encode_pcm(pcm).expect("encode runs");
            containers.push((name, result.data));
        }

        let recorded = read_fixture("reference.wem");
        for (name, bytes) in &containers {
            assert_eq!(
                bytes, &recorded,
                "{name} container differs from reference.wem at file level"
            );
        }
        assert_eq!(containers[0].1, containers[1].1);
        assert_eq!(containers[0].1, containers[2].1);
    }

    /// A geometry that disagrees with the profile is rejected whichever shape the
    /// PCM arrived in (the shape affects storage, never validation).
    #[test]
    fn wrong_profile_geometry_rejects_every_pcm_shape() {
        let encoder = Encoder::new(fixture_selection()).expect("selection resolves");
        // 2 channels / 48 kHz: geometry differs from the 6ch/44100 profile.
        let frames = 5_000usize;
        let channel_major = vec![0u8; frames * 2 * 2];
        let mut interleaved = vec![0u8; frames * 2 * 2];
        for frame in 0..frames {
            // Distinct per (channel, frame): a silent buffer would not show that
            // the geometry check runs before any sample is read.
            interleaved[frame * 4] = (frame % 251) as u8;
            interleaved[frame * 4 + 2] = (frame % 253) as u8;
        }

        let shapes = [
            (
                "rows",
                Pcm16::new(48_000, vec![vec![0i16; frames], vec![1i16; frames]])
                    .expect("rows build"),
            ),
            (
                "channel-major bytes",
                Pcm16::from_channel_major_le(48_000, 2, channel_major).expect("bytes build"),
            ),
            (
                "interleaved bytes",
                Pcm16::from_interleaved_le(48_000, 2, interleaved).expect("bytes build"),
            ),
        ];
        for (name, pcm) in shapes {
            match encoder.encode_pcm(&pcm) {
                Err(EncoderError::GeometryMismatch { .. }) => {}
                Err(other) => panic!("{name}: expected GeometryMismatch, got {other:?}"),
                Ok(_) => panic!("{name}: wrong geometry must be rejected"),
            }
        }
    }
}

mod mode_tail {
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
    use wem_profiles::{compiled_profile_for_selection, WwiseProfile, WwiseVersion};
    use wem_scheduling::FramePlan;

    /// Analysis resources of the installed 2ch/48000 configuration, resolved from
    /// a structured selection against the compiled profile carrier.
    fn two_channel_resources() -> wem_analysis::config::AnalysisProfileResources {
        let selection =
            WwiseProfile::new(WwiseVersion::Wwise2013, 2, 48_000).expect("2ch/48000 selection");
        let compiled =
            compiled_profile_for_selection(selection).expect("installed 2ch profile resolves");
        wem_profiles::assemble_analysis_resources(&compiled, None).expect("resources assemble")
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
}

mod reference_bytes {
    //! End-to-end reference parity test: the Rust kernel must reproduce the
    //! reference WEM byte-for-byte.
    //!
    //! Oracle: `tests/fixtures/reference.wem`, produced by the Python encoder
    //! from `tests/fixtures/input.wav`.
    //!
    //! This test asserts file-level byte equality (`Vec<u8> ==`); the digest of
    //! that artifact is never re-typed here — the bytes are the claim.

    use wem_core::encoder::{Encoder, Pcm16};
    use wem_core::error::EncoderError;
    use wem_core::stream::StreamSession;
    use wem_core::usecases::wav::read_pcm16;
    use wem_core::{WwiseProfile, WwiseVersion};

    use crate::common::{fixture_selection, fixtures_dir, read_fixture};

    /// Read the fixture WAV and hand it to the encoder the way the CLI does.
    fn encode_fixture() -> (wem_core::EncodeResult, Encoder) {
        let wav = read_pcm16(&fixtures_dir().join("input.wav")).expect("input.wav reads");
        assert_eq!(wav.channels(), 6);
        assert_eq!(wav.sample_rate(), 44100);
        assert_eq!(wav.frames(), 139398);
        let encoder = Encoder::new(fixture_selection()).expect("selection resolves");
        let pcm = wav.to_pcm16().expect("wav converts to Pcm16");
        let result = encoder.encode_pcm(&pcm).expect("encode runs");
        (result, encoder)
    }

    // ---------------------------------------------------------------------------
    // 1. Reference byte parity (file-level, Vec<u8> equality)
    // ---------------------------------------------------------------------------

    #[test]
    fn encode_is_byte_identical_to_reference_wem() {
        let (result, _encoder) = encode_fixture();
        let reference = read_fixture("reference.wem");
        // The whole claim: the kernel's bytes are the reference container's bytes.
        assert_eq!(
            result.data, reference,
            "rust WEM differs from reference.wem at file level"
        );
        // Stats mirror the Python reference expectations: the observations the
        // kernel made, plus the container length read off the bytes above.
        let stats = &result.stats;
        assert_eq!(stats.pcm_frames, 139398);
        assert_eq!(stats.channels, 6);
        assert_eq!(stats.audio_packets, 205);
        assert_eq!(stats.short_packets, 77);
        assert_eq!(stats.long_packets, 128);
        assert_eq!(
            result.len(),
            108771,
            "the container length is the reference's"
        );
        // Which profile produced those bytes needs no label: selection is exactly
        // one profile per geometry or a rejection, so `fixture_selection()` (the
        // selection this test itself passed to `Encoder::new`) denotes the one
        // installed 6ch/44100 configuration, and the byte equality above is what
        // pins that those are the bytes it produces.
    }

    // ---------------------------------------------------------------------------
    // 2. StreamSession: chunk boundaries must not affect the output bytes
    // ---------------------------------------------------------------------------

    fn fixture_le_bytes() -> Vec<u8> {
        let wav = read_pcm16(&fixtures_dir().join("input.wav")).expect("input.wav reads");
        // The streaming cases slice the bytes repeatedly, so they take an owned
        // copy of the WAV's borrow.
        wav.interleaved_le_bytes().to_vec()
    }

    #[test]
    fn stream_session_with_seven_uneven_chunks_matches_encode_pcm() {
        let bytes = fixture_le_bytes();
        let channels = 6usize;
        let bytes_per_frame = 2 * channels;
        let total_frames = bytes.len() / bytes_per_frame;
        // Seven unequal frame-count chunks summing to the full stream.
        let frame_boundaries: Vec<usize> =
            vec![0, 17000, 40500, 70600, 85600, 110100, 129100, total_frames];
        assert_eq!(frame_boundaries.len(), 8, "seven chunks");
        assert!(
            frame_boundaries
                .windows(2)
                .all(|w| w[0] < w[1] && w[1] <= total_frames),
            "uneven, in-range chunks"
        );
        assert!(
            frame_boundaries
                .windows(2)
                .map(|w| w[1] - w[0])
                .collect::<Vec<_>>()
                .windows(2)
                .all(|w| w[0] != w[1]),
            "all chunk sizes differ"
        );

        let mut session = StreamSession::for_selection(fixture_selection()).expect("session opens");
        for window in frame_boundaries.windows(2) {
            let start = window[0] * bytes_per_frame;
            let end = window[1] * bytes_per_frame;
            session
                .push_pcm_chunk(&bytes[start..end])
                .expect("chunk pushes");
        }
        let streamed = session.finish().expect("finish");
        let (oneshot, encoder) = encode_fixture();
        assert_eq!(
            streamed.data, oneshot.data,
            "streamed encode differs from encode_pcm"
        );
        assert_eq!(streamed.data, read_fixture("reference.wem"));
        assert_eq!(streamed.stats, oneshot.stats);
        // The container walk exposes the reply packet stream: the setup
        // packet (seq 0) followed by the audio packets in encoding order.
        let parts = wem_container::load_wem_parts_bytes(&streamed.data).expect("wem parts");
        assert!(parts.is_wwise_vorbis);
        let setup = encoder.profile().setup_packet().expect("setup reads");
        assert_eq!(parts.setup_packet, Some(setup.clone()));
        assert_eq!(parts.audio_packets.len(), 205);
        let mut reassembled = Vec::new();
        for packet in std::iter::once(setup.as_slice())
            .chain(parts.audio_packets.iter().map(|p| p.as_slice()))
        {
            reassembled.extend_from_slice(&(packet.len() as u16).to_le_bytes());
            reassembled.extend_from_slice(packet);
        }
        assert!(
            parts.data_raw.starts_with(&reassembled),
            "data chunk payload does not start with the reply packet stream"
        );
    }

    // ---------------------------------------------------------------------------
    // 3. Error paths (same semantics as the Python encoder / v1 codes)
    // ---------------------------------------------------------------------------

    #[test]
    fn input_too_short_rejects_4095_frames() {
        let wav = read_pcm16(&fixtures_dir().join("input.wav")).expect("input.wav reads");
        let frames = 4095usize;
        let samples_per_frame = wav.channels() * 2;
        let bytes = &wav.interleaved_le_bytes()[0..frames * samples_per_frame];
        let pcm = Pcm16::from_interleaved_le(44100, 6, bytes).expect("pcm parses");
        let encoder = Encoder::new(fixture_selection()).expect("selection resolves");
        match encoder.encode_pcm(&pcm) {
            Err(EncoderError::InputTooShort { want, got }) => {
                assert_eq!(want, 4096);
                assert_eq!(got, 4095);
            }
            Err(other) => panic!("expected InputTooShort, got {other:?}"),
            Ok(_) => panic!("4095 frames must be rejected"),
        }
    }

    #[test]
    fn stream_session_input_too_short_rejects_4095_frames() {
        let wav = read_pcm16(&fixtures_dir().join("input.wav")).expect("input.wav reads");
        let bytes: Vec<u8> = wav.interleaved_le_bytes()[0..4095 * wav.channels() * 2].to_vec();
        let mut session = StreamSession::for_selection(fixture_selection()).expect("session opens");
        session.push_pcm_chunk(&bytes).expect("chunk pushes");
        match session.finish() {
            Err(EncoderError::InputTooShort { want, got }) => {
                assert_eq!(want, 4096);
                assert_eq!(got, 4095);
            }
            Err(other) => panic!("expected InputTooShort, got {other:?}"),
            Ok(_) => panic!("4095 frames must be rejected"),
        }
    }

    #[test]
    fn wrong_profile_geometry_is_rejected() {
        let encoder = Encoder::new(fixture_selection()).expect("selection resolves");
        // 2 channels / 48 kHz: geometry differs from the 6ch/44100 profile.
        let pcm = Pcm16::new(48000, vec![vec![0i16; 5000], vec![0i16; 5000]]).expect("pcm builds");
        match encoder.encode_pcm(&pcm) {
            Err(EncoderError::GeometryMismatch { .. }) => {}
            Err(other) => panic!("expected GeometryMismatch, got {other:?}"),
            Ok(_) => panic!("wrong geometry must be rejected"),
        }
    }

    #[test]
    fn unresolvable_selections_are_rejected() {
        // 2ch/44100 is not an installed configuration (the installed 2ch
        // profile is 48000 Hz): the selection must be rejected, never
        // satisfied by a neighbouring geometry.
        let selection = WwiseProfile::new(WwiseVersion::Wwise2013, 2, 44_100).expect("selection");
        match Encoder::new(selection) {
            Err(EncoderError::ProfileNotFound { requested }) => {
                assert_eq!(requested, "2ch/44100Hz/2013");
            }
            Err(other) => panic!("expected ProfileNotFound, got {other:?}"),
            Ok(_) => panic!("unresolvable selection must be rejected"),
        }
        match StreamSession::for_selection(selection) {
            Err(EncoderError::ProfileNotFound { requested }) => {
                assert_eq!(requested, "2ch/44100Hz/2013");
            }
            Err(other) => panic!("expected ProfileNotFound, got {other:?}"),
            Ok(_) => panic!("unresolvable selection must not open a session"),
        }
    }

    #[test]
    fn stream_lifecycle_violations_are_state_errors() {
        // Chunk before Init.
        let mut early = StreamSession::new();
        match early.push_pcm_chunk(&[0u8; 12]) {
            Err(EncoderError::StateError { .. }) => {}
            Err(other) => panic!("chunk before Init: expected StateError, got {other:?}"),
            Ok(_) => panic!("chunk before Init must fail"),
        }

        // Finish before Init.
        match early.finish() {
            Err(EncoderError::StateError { .. }) => {}
            Err(other) => panic!("finish before Init: expected StateError, got {other:?}"),
            Ok(_) => panic!("finish before Init must fail"),
        }

        // Chunk after Finish.
        let mut session = StreamSession::for_selection(fixture_selection()).expect("session opens");
        session
            .push_pcm_chunk(&[0u8; 12 * 1000])
            .expect("chunk pushes");
        match session.finish() {
            Err(EncoderError::InputTooShort { .. }) => {}
            Err(other) => panic!("short finish: expected InputTooShort, got {other:?}"),
            Ok(_) => panic!("short stream must fail"),
        }
        match session.push_pcm_chunk(&[0u8; 12]) {
            Err(EncoderError::StateError { .. }) => {}
            Err(other) => panic!("chunk after Finish: expected StateError, got {other:?}"),
            Ok(_) => panic!("chunk after Finish must fail"),
        }
        // Terminal: a second Finish is also a state error.
        match session.finish() {
            Err(EncoderError::StateError { .. }) => {}
            Err(other) => panic!("second Finish: expected StateError, got {other:?}"),
            Ok(_) => panic!("second Finish must fail"),
        }
    }

    #[test]
    fn chunk_partial_frame_is_a_geometry_mismatch() {
        let mut session = StreamSession::for_selection(fixture_selection()).expect("session opens");
        // 6 channels -> 12 bytes per frame; 13 leaves a trailing partial frame.
        match session.push_pcm_chunk(&[0u8; 13]) {
            Err(EncoderError::GeometryMismatch { .. }) => {}
            Err(other) => panic!("partial frame: expected GeometryMismatch, got {other:?}"),
            Ok(_) => panic!("partial frame must fail"),
        }
    }

    /// The result carries the container without the caller reaching into `data`:
    /// `len`/`is_empty` describe it and `write_to` persists it, naming the path
    /// in the error when the write fails.
    #[test]
    fn encode_result_length_and_write_to_round_trip() {
        let pcm =
            Pcm16::from_interleaved_le(44_100, 6, fixture_le_bytes()).expect("fixture PCM parses");
        let result = Encoder::new(fixture_selection())
            .expect("encoder builds")
            .encode_pcm(&pcm)
            .expect("encode succeeds");

        assert_eq!(result.len(), result.data.len());
        assert_eq!(
            result.len(),
            read_fixture("reference.wem").len(),
            "the container length is the committed reference's"
        );
        assert!(!result.is_empty());

        let path = std::env::temp_dir().join("wem-encode-result-write-to.wem");
        let _ = std::fs::remove_file(&path);
        result.write_to(&path).expect("write_to succeeds");
        assert_eq!(
            std::fs::read(&path).expect("written file reads"),
            result.data,
            "write_to must persist exactly the container bytes"
        );
        std::fs::remove_file(&path).expect("temporary output is removable");

        // A path that cannot be written reports the path it failed on.
        let unwritable = std::env::temp_dir().join("wem-does-not-exist-dir/out.wem");
        match result.write_to(&unwritable) {
            Err(EncoderError::Internal(wem_core::InternalError::Io { message })) => {
                assert!(
                    message.contains("wem-does-not-exist-dir"),
                    "the write error must name the path: {message}"
                );
            }
            other => panic!("expected an Io internal error, got {other:?}"),
        }
    }
}

mod two_channel_bytes {
    //! The installed 2ch/48000 selection end to end: the same committed reference
    //! WEM the batch encoder and the streaming session must produce.
    //!
    //! The profile bundle that drives both paths is resolved with
    //! `wem_profiles::compiled_profile_for_selection` — a structured selection against the
    //! compiled-in profile bundle, never a profile name or a profile tree. The
    //! profile identity is read off that carrier; the bytes compared below are the
    //! claim.

    use wem_core::encoder::Encoder;
    use wem_core::stream::StreamSession;
    use wem_core::usecases::wav::read_pcm16;
    use wem_profiles::resolve_wem_profile_selection;

    use crate::common::{two_channel_dir, two_channel_selection};

    #[test]
    fn two_channel_selection_encodes_the_committed_two_channel_wem() {
        let encoder = Encoder::new(two_channel_selection()).expect("2ch selection resolves");
        // The profile label is derived from the identity the selection resolves
        // to, never a literal: the encoder and the free resolver must label the
        // same profile.
        let resolved =
            resolve_wem_profile_selection(two_channel_selection()).expect("2ch resolves");
        assert_eq!(encoder.profile().label(), resolved.label());

        let wav = read_pcm16(&two_channel_dir().join("tone_high.wav")).expect("2ch WAV reads");
        let pcm = wav.to_pcm16().expect("2ch WAV converts to Pcm16");
        let result = encoder.encode_pcm(&pcm).expect("2ch encode runs");
        // Byte equality covers the profile identity and its setup packet too:
        // the seq-0 packet of this container *is* the 2ch/48k setup packet.
        assert_eq!(
            result.data,
            std::fs::read(two_channel_dir().join("tone_high.wem")).expect("tone_high.wem reads")
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
            std::fs::read(two_channel_dir().join("tone_high.wem")).expect("tone_high.wem reads")
        );
    }
}

mod profile_registration {
    //! The 2ch/48000 profile: its setup packet, codebooks, and quality curves are
    //! registered from the paired build together with its psychoacoustic
    //! calibration. The setup and encoder resources are complete.
    //!
    //! The profile comes from `wem_profiles::compiled_profile_for_selection` — a
    //! structured selection resolved against the compiled profile carrier, never a
    //! profile name or a profile tree. The setup packet is pinned by comparing its
    //! bytes against the seq-0 reply packet of the committed two-channel reference
    //! container, never by re-typing its digest.

    use wem_profiles::{
        compiled_profile_for_selection, embedded_registry, normalize_quality_factor,
    };

    use crate::common::{fixture_selection, two_channel_dir, two_channel_selection};

    /// The 2ch/48000 setup packet as the committed two-channel reference container
    /// carries it: the seq-0 reply packet of `tests/data/2ch-reference/tone_high.wem`
    /// (`include/wem.h`: seq 0 is the setup packet, framed by its u16 LE length).
    fn committed_two_channel_setup_packet() -> Vec<u8> {
        let wem = std::fs::read(two_channel_dir().join("tone_high.wem"))
            .expect("committed 2ch reference container reads");
        let parts = wem_container::load_wem_parts_bytes(&wem).expect("wem parts load");
        parts
            .setup_packet
            .expect("the committed container carries a setup packet")
            .to_vec()
    }

    #[test]
    fn two_channel_profile_exposes_its_quality_curves() {
        // The 2ch/48000 profile ships its quality curves and setup packet.
        let compiled =
            compiled_profile_for_selection(two_channel_selection()).expect("2ch profile resolves");
        let model = compiled.encoder_profile().expect("encoder profile");
        assert!(model.setup_available());
        assert!(model.pending_reason().is_none());
        // The setup bytes are the committed container's setup bytes: identity is
        // proven against the artifact, not against a hand-written digest.
        assert_eq!(
            compiled.setup_packet().expect("setup packet"),
            committed_two_channel_setup_packet(),
            "the profile's setup packet must be the committed container's seq-0 packet"
        );

        let curves = compiled
            .quality_curves()
            .expect("curves read")
            .expect("profile registers the curves table");
        assert_eq!(curves.breakpoints().len(), 13);
        assert_eq!(curves.breakpoints()[0], -0.2);
        assert_eq!(curves.breakpoints()[12], 1.0);
        assert_eq!(curves.curve_names().count(), 4);

        // The curves evaluate on the normalized axis (shared pin: q = 4.0).
        let (values, extrapolated) = curves
            .evaluate_result(normalize_quality_factor(4.0))
            .expect("evaluate");
        assert!(!extrapolated);
        assert_eq!(values["desc31.psy_double"], 18.0000015);
    }

    #[test]
    fn two_channel_profile_lists_in_the_registry_as_setup_available() {
        let registry = embedded_registry().expect("registry loads");
        assert_eq!(registry.len(), 2);
        let profile = registry
            .resolve_selection(two_channel_selection())
            .expect("2ch selection resolves");
        // The label is derived from the identity the registry resolves for the
        // selection, never a literal.
        assert_eq!(
            profile.label(),
            compiled_profile_for_selection(two_channel_selection())
                .expect("2ch profile resolves")
                .label()
        );
        assert!(profile.setup_available());
        // The setup bytes the registry's profile hands back are the committed
        // container's, so its declared digest is never re-typed as a literal.
        assert_eq!(
            profile.setup_packet().expect("setup packet"),
            committed_two_channel_setup_packet(),
            "the registry's 2ch profile must hand back the committed container's setup packet"
        );
        assert!(profile.pending_reason().is_none());

        // The two installed selections resolve to distinct setup packets and
        // distinct channel layouts, so the packet is a consequence of the
        // selection, never a selector.
        let six_channel = registry
            .resolve_selection(fixture_selection())
            .expect("6ch selection resolves");
        assert_ne!(six_channel.setup_bytes(), profile.setup_bytes());
        assert_ne!(
            six_channel.key().channel_layout(),
            profile.key().channel_layout()
        );
    }
}

mod quality_binding {
    //! Quality pass-through (wem-core level): a quality factor bound to the
    //! profile is forwarded to the analysis assembly; with quality=None the
    //! historical reference bytes are unchanged.

    use wem_core::encoder::Encoder;
    use wem_core::usecases::wav::read_pcm16;
    use wem_profiles::{resolve_wem_profile_selection, resolve_wem_profile_selection_quality};

    use crate::common::{fixture_selection, fixtures_dir, read_fixture};

    #[test]
    fn selection_resolution_returns_additive_quality_copies() {
        let base =
            resolve_wem_profile_selection(fixture_selection()).expect("6ch profile resolves");
        assert!(base.setup_available());
        assert_eq!(base.quality(), None);

        let bound = resolve_wem_profile_selection_quality(fixture_selection(), Some(4.0))
            .expect("quality-bound copy");
        assert_eq!(bound.quality(), Some(4.0));
        assert!(bound.setup_available());
        // The bound copy is additive: re-resolving the selection is unbound.
        assert_eq!(
            resolve_wem_profile_selection(fixture_selection())
                .expect("re-resolves")
                .quality(),
            None
        );

        // Non-finite quality is rejected.
        assert!(
            resolve_wem_profile_selection_quality(fixture_selection(), Some(f64::NAN)).is_err()
        );
    }

    #[test]
    fn quality_none_encode_bytes_match_the_reference_container() {
        let encoder = Encoder::new_with_quality(fixture_selection(), None).expect("encoder builds");
        let wav = read_pcm16(&fixtures_dir().join("input.wav")).expect("input.wav reads");
        let pcm = wav.to_pcm16().expect("wav converts to Pcm16");
        let encoded = encoder.encode_pcm(&pcm).expect("encode runs");
        // Byte equality against the committed reference is the whole claim:
        // quality=None must reproduce the historical reference container exactly.
        assert_eq!(
            encoded.data,
            read_fixture("reference.wem"),
            "quality=None must reproduce the historical reference bytes exactly"
        );
    }
}
