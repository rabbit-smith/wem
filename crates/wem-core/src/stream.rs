//! Streaming encode session: the core streaming lifecycle.
//!
//! Exactly one `Init` (profile selection) as the first call, zero or more
//! PCM `chunk`s, then exactly one `Finish`. Every lifecycle violation is
//! reported as an [`EncoderError`](crate::error::EncoderError) variant —
//! none panics, and none changes output bytes for valid streams.
//!
//! This lifecycle, the reply framing, and the error codes are pinned for
//! the cross-language shells in `include/wem.h` (see crates/AGENTS.md,
//! "C ABI contract"). The C ABI, PyO3 and wasm shells are thin wrappers
//! over this session; the reply side is a packet sequence (seq 0 = setup
//! packet, then audio packets) followed by the container summary, which
//! [`EncodeResult`](crate::encoder::EncodeResult) plus
//! [`load_wem_parts_bytes`](wem_container::load_wem_parts_bytes) provide.
//!
//! # Incremental emission and bounded memory (true streaming)
//!
//! `StreamSession` no longer accumulates the whole PCM stream. Each
//! [`push_pcm_chunk`](Self::push_pcm_chunk) returns the packets that just
//! completed (setup packet first, then audio packets in encoding order),
//! and only these bounded structures survive:
//!
//! * a per-channel ring of the most recent
//!   [`STREAM_RING_KEEP`](wem_analysis::preprocessing::streaming::STREAM_RING_KEEP)
//!   samples (9216) — the 4096-sample LPC tail model, the 8192-sample
//!   detector end-of-stream tail reach (8192 + 1024 prime offset), and
//!   every in-flight 2048-sample frame window fit inside it;
//! * the cached first-frame LPC prime (1024 samples per channel);
//! * the emitted audio packets (the encoded output itself, required by the
//!   `Finish` container assembly — the only state that still grows with
//!   stream length, proportionally to the output, never the input);
//! * the transient-mode queue (grows with the number of 64-sample detector
//!   quanta, i.e. with length, but at 8 bytes per 64 samples = 1 bit per
//!   sample).
//!
//! Frames are emitted as soon as their mode decision and their window are
//! fully determined by the samples received so far. A frame whose mode
//! look-ahead scan would reach past the retained detector frontier — or
//! whose window reaches the stream endpoint — is held back and flushed at
//! `finish` once the end-of-stream tail is available. The documented
//! delayed tail (worst case, all short blocks) is the last 2048 frame
//! centers before the endpoint plus the 1024 centers past it: at most
//! 3072 center samples ≈ 24 short frames (≈ 72 KB of i16 PCM at 6
//! channels) — independent of total stream length.
//!
//! Chunk boundaries never affect the output bytes: every emitted frame is
//! processed with exactly the inputs the complete-stream encode would have
//! used (same detector stream, same mode scans, same window samples), so
//! any chunking reproduces `encode_pcm` byte-for-byte.

use wem_analysis::dsp::transform::apply_vorbis_window;
use wem_analysis::preprocessing::streaming::StreamingPcmFeeder;
use wem_analysis::preprocessing::windowing::WindowedFrame;
use wem_analysis::session::AnalysisSession;
use wem_profiles::data::DataDir;
use wem_profiles::registry::installed_registry;
use wem_scheduling::{append_samples, emit_block, required_samples, FramePlan, SchedulerState};

use crate::encoder::{EncodeResult, EncodeStats, Encoder, MIN_PCM_FRAMES};
use crate::error::{EncoderError, InternalError};
use crate::pack::pack_analysis_frame;

/// Profile selection reference.
///
/// A hard identity assertion (setup SHA-256) plus an optional soft name
/// cross-check; this type intentionally carries only these two fields.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProfileRef {
    /// Hard identity assertion: must exactly match an installed profile's
    /// setup SHA-256 (lowercase hex).
    pub setup_sha256: String,
    /// Soft cross-check: compared against the resolved profile name; a
    /// mismatch is a state error (the reference semantics reject it).
    pub name: Option<String>,
}

impl ProfileRef {
    /// A reference that asserts both the setup digest and the name.
    pub fn with_name(setup_sha256: impl Into<String>, name: impl Into<String>) -> Self {
        Self {
            setup_sha256: setup_sha256.into(),
            name: Some(name.into()),
        }
    }
}

/// One packet emitted by a streaming encode.
///
/// Emission order is the reply stream order: the first emitted packet is
/// the Vorbis setup packet (seq 0); every later one is an audio packet in
/// encoding order.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StreamPacket {
    /// Raw packet bytes as they appear inside the WEM data chunk.
    pub data: Vec<u8>,
}

/// Frame centers within this many samples of the stream endpoint are not
/// fully determined yet (mode look-ahead reaches the detector frontier);
/// they are flushed at `finish` with the end-of-stream tail.
///
/// Bound derivation (blocksizes 256/2048, detector hop 64 / window 128,
/// prime 1024): a scan at center C decides at detector-stream boundary
/// C + 2624 (long-mode worst case) and never reads at or beyond its
/// `generated - hop` limit. With M source samples received, the ingested
/// quantum count is floor((M + 896) / 64), so the limit is
/// 64 * floor((M + 896) / 64); covering the boundary needs M >= C + 1791
/// in the worst alignment. 2048 keeps a whole long block of margin.
const STREAM_LOOKAHEAD: i64 = 2048;

/// One streaming encode session (the core streaming lifecycle).
///
/// Build with [`StreamSession::new`] and drive `init_profile` ->
/// `push_pcm_chunk`* -> `finish`, or use the convenience constructor
/// [`StreamSession::for_profile_ref`].
pub struct StreamSession {
    initialized: bool,
    finished: bool,
    encoder: Option<Encoder>,
    pipeline: Option<StreamPipeline>,
}

/// The incremental encode state (only exists after `Init`).
struct StreamPipeline {
    channels: i64,
    blocksizes: [i64; 2],
    session: AnalysisSession,
    feeder: StreamingPcmFeeder,
    /// Mode-loop cursor (batch `select_modes` loop variables).
    center: i64,
    current_mode: i64,
    /// Decided current modes: `modes[k]` is frame k's mode.
    modes: Vec<i64>,
    /// Incremental planner state (batch `plan_mode_sequence` state);
    /// `None` until the first emitted plan.
    planner_state: Option<SchedulerState>,
    /// Emitted frame plans (frame k's plan at index k).
    plans: Vec<FramePlan>,
    /// For every pending plan (indices `plans.len()..modes.len()-1`), the
    /// mode-loop center *after* the deciding iteration: the batch loop
    /// terminates when that center reaches `source_len + prefix`, which
    /// is unknowable until `finish` — so the plan's `following` (the
    /// scanned mode, or the `terminal_following` rule at EOS) is decided
    /// then.
    pending_centers: Vec<i64>,
    /// Frames fully analyzed and packed so far.
    frames_done: i64,
    /// Audio packets in encoding order.
    audio_packets: Vec<Vec<u8>>,
    /// Whether the setup packet already left through a `push`.
    setup_emitted: bool,
}

impl StreamPipeline {
    fn new(encoder: &Encoder) -> Result<Self, EncoderError> {
        let profile = encoder.profile();
        let channels = profile.channels();
        let blocksizes = profile.block_sizes();
        let sample_rate = profile.sample_rate();
        let session = AnalysisSession::new(
            channels,
            sample_rate,
            blocksizes,
            encoder.analysis_resources().clone(),
        )
        .map_err(|error| EncoderError::Internal(InternalError::Analysis(error)))?;
        let feeder = StreamingPcmFeeder::new(channels, blocksizes)
            .map_err(|error| EncoderError::Internal(InternalError::Analysis(error)))?;
        Ok(Self {
            channels,
            blocksizes,
            session,
            feeder,
            center: 0,
            current_mode: 0,
            modes: Vec::new(),
            planner_state: None,
            plans: Vec::new(),
            pending_centers: Vec::new(),
            frames_done: 0,
            audio_packets: Vec::new(),
            setup_emitted: false,
        })
    }

    fn total(&self) -> i64 {
        self.feeder.total_samples()
    }

    /// Whether a mode-loop scan at `center` is fully determined by the
    /// samples received so far (see `STREAM_LOOKAHEAD`).
    fn scan_safe(&self, center: i64) -> bool {
        self.total() >= MIN_PCM_FRAMES as i64 && self.total() >= center + STREAM_LOOKAHEAD
    }

    /// Ingest every completed detector quantum, in order, one at a time
    /// (the batch path ingests the same quanta up front; streaming
    /// ingests them as they complete, in the same order).
    fn ingest_completed_quanta(&mut self) -> Result<(), EncoderError> {
        let StreamPipeline {
            session, feeder, ..
        } = self;
        let mut error: Option<EncoderError> = None;
        feeder
            .for_each_completed_quantum(|_, window| {
                if error.is_none() {
                    error = session
                        .ingest_transient_quantum(window)
                        .err()
                        .map(|err| EncoderError::Internal(InternalError::Analysis(err)));
                }
                Ok(())
            })
            .map_err(|error| EncoderError::Internal(InternalError::Analysis(error)))?;
        match error {
            Some(error) => Err(error),
            None => Ok(()),
        }
    }

    /// Run one mode-loop iteration (the batch `select_modes` body):
    /// scan, record the current mode, advance the cursor. The scanned
    /// `following` stays in `current_mode`; plan emission is separate
    /// (the batch `terminal_following` rule may override it at EOS).
    fn mode_iteration(&mut self) -> Result<(), EncoderError> {
        let status = self
            .session
            .mode_selection_status(self.center + self.blocksizes[1] / 2, self.current_mode)
            .map_err(|error| EncoderError::Internal(InternalError::Analysis(error)))?;
        let following = if status < 0 { 0 } else { status };
        self.modes.push(self.current_mode);
        self.center += self.blocksizes[self.current_mode as usize] / 4
            + self.blocksizes[following as usize] / 4;
        self.current_mode = following;
        // Record the center after this iteration: it decides whether
        // plan `modes.len() - 1` is the terminal one at EOS.
        self.pending_centers.push(self.center);
        Ok(())
    }

    /// Emit one frame plan (the batch `plan_mode_sequence` step) for
    /// frame `index`, whose `current` mode is `modes[index]`.
    fn emit_plan(&mut self, index: i64, following: i64) -> Result<(), EncoderError> {
        if index as usize != self.plans.len() || index as usize + 1 > self.modes.len() {
            return Err(EncoderError::Internal(InternalError::Invariant {
                message: "plan emission order diverged",
            }));
        }
        let mut state = match self.planner_state.take() {
            None => SchedulerState::new(
                0,
                self.modes[index as usize],
                self.blocksizes[1] / 2,
                self.blocksizes[1] / 2,
            ),
            Some(state) => {
                if state.current != self.modes[index as usize] || state.emitted != index {
                    return Err(EncoderError::Internal(InternalError::Invariant {
                        message: "mode sequence and scheduler state diverged",
                    }));
                }
                state
            }
        };
        let invariant = || {
            EncoderError::Internal(InternalError::Invariant {
                message: "planner rejected a mode transition",
            })
        };
        let required =
            required_samples(&state, following, &self.blocksizes).map_err(|_| invariant())?;
        state =
            append_samples(&state, (required - state.filled).max(0)).map_err(|_| invariant())?;
        let (plan, state) =
            emit_block(&state, following, &self.blocksizes).map_err(|_| invariant())?;
        self.planner_state = Some(state);
        self.plans.push(plan);
        // Plan emission is strictly in order: each one consumes the
        // front pending-center entry (center after its deciding scan).
        self.pending_centers.remove(0);
        Ok(())
    }

    /// Analyze and pack one planned frame (the batch per-frame loop body).
    fn emit_frame(&mut self, plan: &FramePlan, encoder: &Encoder) -> Result<Vec<u8>, EncoderError> {
        let raw = self
            .feeder
            .frame_raw_rows(plan, &self.blocksizes)
            .map_err(|error| EncoderError::Internal(InternalError::Analysis(error)))?;
        // Short blocks do not use pending long-transition flags; their
        // effective window is always the short one (batch rule).
        let window_modes = if plan.current == 0 {
            (0, 0, 0)
        } else {
            (plan.previous, plan.current, plan.following)
        };
        let frozen = self
            .session
            .resources
            .frozen
            .as_ref()
            .map(|frozen| &frozen.window_halves);
        let mut samples = Vec::with_capacity(raw.len());
        for row in &raw {
            samples.push(
                apply_vorbis_window(
                    row,
                    &self.blocksizes,
                    window_modes.0,
                    window_modes.1,
                    window_modes.2,
                    frozen,
                )
                .map_err(|error| EncoderError::Internal(InternalError::Analysis(error)))?,
            );
        }
        let center =
            plan.sample_start - self.blocksizes[1] / 2 + self.blocksizes[plan.current as usize] / 2;
        let windowed = WindowedFrame {
            plan: *plan,
            center,
            samples,
        };
        let analysis = self
            .session
            .analyze_window(windowed, None)
            .map_err(|error| EncoderError::Internal(InternalError::Analysis(error)))?;
        let packet = pack_analysis_frame(
            encoder.setup(),
            encoder.codebooks(),
            &analysis,
            self.channels as u32,
        )
        .map_err(|error| EncoderError::Internal(InternalError::Packet(error)))?;
        Ok(packet.packet)
    }

    /// Emit every frame and plan that is fully determined so far.
    /// Returns the newly completed audio packets (in order).
    fn pump(&mut self, encoder: &Encoder) -> Result<Vec<Vec<u8>>, EncoderError> {
        let mut packets = Vec::new();
        loop {
            // 1) Emit all planned frames (plan existence implies their
            //    windows lie inside the retained ring — see the bound
            //    comments on `STREAM_LOOKAHEAD` and the ring keep).
            while self.frames_done < self.plans.len() as i64 {
                let plan = self.plans[self.frames_done as usize];
                let packet = self.emit_frame(&plan, encoder)?;
                self.frames_done += 1;
                self.audio_packets.push(packet.clone());
                packets.push(packet);
            }
            let index = self.plans.len() as i64;
            if index < self.modes.len() as i64 {
                // A plan is pending its EOS decision: it is non-terminal
                // (batch loop continues) whenever its post-iteration
                // center is strictly below `total + prefix`.
                let post_center = self.pending_centers[0];
                if post_center < self.total() + self.blocksizes[1] / 2 {
                    self.emit_plan(index, self.current_mode)?;
                    continue;
                }
                // Possibly terminal: it may still become non-terminal as
                // more data arrives; only `finish` can decide. A pending
                // plan also blocks further scans (the batch loop would
                // not have advanced either).
                break;
            }
            // 2) index == modes.len(): run the next scan once its
            //    look-ahead frontier is inside the received samples.
            if self.scan_safe(self.center) {
                self.mode_iteration()?;
                continue;
            }
            break;
        }
        Ok(packets)
    }

    /// The `finish` continuation: end-of-stream tail, remaining quanta,
    /// remaining mode decisions, and the deferred tail frames.
    fn finish_source(&mut self, encoder: &Encoder) -> Result<(), EncoderError> {
        self.feeder
            .finish_source()
            .map_err(|error| EncoderError::Internal(InternalError::Analysis(error)))?;
        self.ingest_completed_quanta()?;
        let stop_center = self.total() + self.blocksizes[1] / 2;
        loop {
            let index = self.plans.len() as i64;
            if index < self.modes.len() as i64 {
                // A plan is pending its EOS decision: decide it now that
                // the endpoint is known.
                let post_center = self.pending_centers[0];
                if post_center < stop_center {
                    // Batch loop continues: the scanned following holds.
                    self.emit_plan(index, self.current_mode)?;
                    continue;
                }
                // Batch loop terminates after this frame: the
                // `terminal_following` rule overrides the scan.
                self.emit_plan(index, 1)?;
                self.pending_centers.clear();
                break;
            }
            // index == modes.len(): the batch loop runs one more
            // iteration while its center stays below the stop line.
            if self.center < stop_center {
                self.mode_iteration()?;
                continue;
            }
            break;
        }
        while self.frames_done < self.plans.len() as i64 {
            let plan = self.plans[self.frames_done as usize];
            let packet = self.emit_frame(&plan, encoder)?;
            self.frames_done += 1;
            self.audio_packets.push(packet);
        }
        Ok(())
    }
}

impl StreamSession {
    /// Create an uninitialized session (the stream before `Init`).
    pub fn new() -> Self {
        Self {
            initialized: false,
            finished: false,
            encoder: None,
            pipeline: None,
        }
    }

    /// Open the session on one installed profile (`Init`).
    ///
    /// - `PROFILE_NOT_FOUND` when no installed profile's setup SHA-256 matches.
    /// - `STATE_ERROR` on a second Init or a soft `name` cross-check failure.
    pub fn init_profile(&mut self, ref_: &ProfileRef) -> Result<(), EncoderError> {
        if self.initialized || self.finished {
            return Err(EncoderError::StateError {
                message: "Init must be the first request".into(),
            });
        }
        let data = DataDir::from_env()?;
        let registry = installed_registry(&data)?;
        let wanted = ref_.setup_sha256.to_lowercase();
        let candidate = registry
            .list()
            .into_iter()
            .find(|profile| profile.setup_sha256() == wanted);
        let profile = match candidate {
            Some(profile) => profile,
            None => {
                return Err(EncoderError::ProfileNotFound {
                    requested: ref_.setup_sha256.clone(),
                })
            }
        };
        if let Some(name) = &ref_.name {
            if name != profile.name() {
                return Err(EncoderError::StateError {
                    message: format!(
                        "profile name mismatch: setup_sha256 resolves to \
                         '{}', not '{name}'",
                        profile.name()
                    ),
                });
            }
        }
        let encoder = Encoder::from_profile_model(&profile, None)?;
        let pipeline = StreamPipeline::new(&encoder)?;
        self.encoder = Some(encoder);
        self.pipeline = Some(pipeline);
        self.initialized = true;
        Ok(())
    }

    /// Convenience constructor: `new()` + `init_profile()`.
    pub fn for_profile_ref(ref_: &ProfileRef) -> Result<Self, EncoderError> {
        let mut session = Self::new();
        session.init_profile(ref_)?;
        Ok(session)
    }

    /// Open the session on one profile carried entirely as bytes (`Init`
    /// from a bytes bundle; the threadless / wasm32-unknown-unknown entry).
    ///
    /// `index` / `files` are the profile bytes as documented on
    /// [`Encoder::from_profile_bytes`]; every logical resource is SHA-256
    /// verified on load. The reference is then asserted against the
    /// selected profile with the same semantics as [`init_profile`](Self::init_profile):
    ///
    /// - `PROFILE_NOT_FOUND` when the bundle's setup SHA-256 does not match
    ///   `ref_.setup_sha256`.
    /// - `STATE_ERROR` on a soft `name` cross-check failure.
    pub fn for_profile_ref_bytes(
        ref_: &ProfileRef,
        index: &[u8],
        files: impl IntoIterator<Item = (String, Vec<u8>)>,
    ) -> Result<Self, EncoderError> {
        let encoder = Encoder::from_profile_bytes(index, files)?;
        let profile = encoder.profile();
        if profile.setup_sha256() != ref_.setup_sha256.to_lowercase() {
            return Err(EncoderError::ProfileNotFound {
                requested: ref_.setup_sha256.clone(),
            });
        }
        if let Some(name) = &ref_.name {
            if name != profile.name() {
                return Err(EncoderError::StateError {
                    message: format!(
                        "profile name mismatch: setup_sha256 resolves to \n                         '{}', not '{name}'",
                        profile.name()
                    ),
                });
            }
        }
        let pipeline = StreamPipeline::new(&encoder)?;
        Ok(Self {
            initialized: true,
            finished: false,
            encoder: Some(encoder),
            pipeline: Some(pipeline),
        })
    }

    /// Push one chunk of little-endian signed-16 interleaved PCM bytes
    /// and return the packets that just completed.
    ///
    /// `STATE_ERROR` before Init or after Finish; `GEOMETRY_MISMATCH` when
    /// the chunk carries a trailing partial PCM frame. An empty chunk is a
    /// no-op (chunk boundaries never affect the output bytes).
    ///
    /// Internally the chunk is processed in bounded segments, so even one
    /// huge chunk materializes only `SEGMENT_FRAMES` of float rows at a
    /// time — input memory stays bounded by the streaming contract.
    ///
    /// Emitted packets follow the reply order: the first packet
    /// ever emitted is the setup packet (seq 0), then audio packets in
    /// encoding order. Frames whose mode look-ahead or end-of-stream tail
    /// is not determined yet are withheld and flushed at `finish`.
    pub fn push_pcm_chunk(&mut self, data: &[u8]) -> Result<Vec<StreamPacket>, EncoderError> {
        if !self.initialized {
            return Err(EncoderError::StateError {
                message: "Init is required before chunks".into(),
            });
        }
        if self.finished {
            return Err(EncoderError::StateError {
                message: "chunks are not allowed after Finish".into(),
            });
        }
        let channels = self
            .encoder
            .as_ref()
            .map(|encoder| encoder.profile().channels() as usize)
            .ok_or_else(|| EncoderError::StateError {
                message: "Init is required before chunks".into(),
            })?;
        if !data.len().is_multiple_of(2 * channels) {
            return Err(EncoderError::GeometryMismatch {
                message: "chunk carries a trailing partial PCM frame".into(),
            });
        }

        let mut emitted: Vec<StreamPacket> = Vec::new();
        if data.is_empty() {
            return Ok(emitted);
        }
        let encoder = self.encoder.as_ref().expect("Init stores the encoder");
        let setup_packet = encoder.setup_packet().to_vec();
        let pipeline = self.pipeline.as_mut().expect("Init stores the pipeline");

        // Process the chunk in bounded segments: the float-row
        // conversion, the completed-quanta scratch and the ring all
        // stay sized by `SEGMENT_FRAMES`, never by the whole chunk.
        let bytes_per_segment = SEGMENT_FRAMES * channels * 2;
        let mut offset = 0usize;
        let mut packets: Vec<Vec<u8>> = Vec::new();
        while offset < data.len() {
            let end = data.len().min(offset + bytes_per_segment);
            let rows = chunk_to_float_rows(&data[offset..end], channels);
            pipeline
                .feeder
                .push(&rows)
                .map_err(|error| EncoderError::Internal(InternalError::Analysis(error)))?;
            pipeline.ingest_completed_quanta()?;
            packets.extend(pipeline.pump(encoder)?);
            // Evict only after emission: deferred frames still reference
            // the oldest retained samples.
            pipeline.feeder.settle();
            offset = end;
        }

        if !packets.is_empty() && !pipeline.setup_emitted {
            emitted.push(StreamPacket { data: setup_packet });
            pipeline.setup_emitted = true;
        }
        emitted.extend(packets.into_iter().map(|data| StreamPacket { data }));
        Ok(emitted)
    }

    /// Mark the end of the PCM stream and complete the encode
    /// (`Finish`).
    ///
    /// `STATE_ERROR` when Init did not run or Finish already did;
    /// `INPUT_TOO_SHORT` below the 4096-frame minimum; otherwise the full
    /// encode result (container bytes + stats). The session is terminal
    /// after this call regardless of the outcome, matching the stream
    /// contract.
    pub fn finish(&mut self) -> Result<EncodeResult, EncoderError> {
        if !self.initialized || self.finished {
            return Err(EncoderError::StateError {
                message: "Finish must come exactly once, after Init".into(),
            });
        }
        self.finished = true;
        let encoder = self
            .encoder
            .as_ref()
            .ok_or_else(|| EncoderError::StateError {
                message: "Init is required before chunks".into(),
            })?;
        let pipeline = self
            .pipeline
            .as_mut()
            .ok_or_else(|| EncoderError::StateError {
                message: "Init is required before chunks".into(),
            })?;
        let total = pipeline.total();
        if total < MIN_PCM_FRAMES as i64 {
            return Err(EncoderError::InputTooShort {
                want: MIN_PCM_FRAMES,
                got: total as u32,
            });
        }
        pipeline.finish_source(encoder)?;

        if pipeline.modes.len() as i64 != pipeline.audio_packets.len() as i64 {
            return Err(EncoderError::Internal(InternalError::Invariant {
                message: "analysis window and mode counts diverged",
            }));
        }

        let mut fields = *encoder.container_plan().fmt();
        fields.dw_total_pcm_frames = total as u32;

        let mut packets = Vec::with_capacity(1 + pipeline.audio_packets.len());
        packets.push(encoder.setup_packet().to_vec());
        packets.extend(pipeline.audio_packets.iter().cloned());

        let built = wem_container::wem::build_vorbis_wem(
            fields,
            &packets,
            encoder.container_plan().seek_table(),
            encoder.container_plan().endian(),
            encoder.container_plan().extra_chunks(),
            true,
            None,
        )?;

        let short_packets = pipeline.modes.iter().filter(|&&mode| mode == 0).count() as i64;
        let long_packets = pipeline.modes.iter().filter(|&&mode| mode == 1).count() as i64;
        let bytes = built.wem_bytes.len() as i64;
        Ok(EncodeResult {
            data: built.wem_bytes,
            stats: EncodeStats {
                pcm_frames: total,
                channels: pipeline.channels,
                audio_packets: short_packets + long_packets,
                short_packets,
                long_packets,
                bytes,
                metadata_source: encoder.container_plan().metadata_source().to_string(),
            },
        })
    }

    /// The PCM frame count accumulated so far (streaming observability).
    pub fn pcm_frames(&self) -> i64 {
        self.pipeline
            .as_ref()
            .map(|pipeline| pipeline.total())
            .unwrap_or(0)
    }
}

/// PCM frame count per internal processing segment (bounds the
/// transient float-row conversion of one huge chunk; see
/// [`StreamSession::push_pcm_chunk`]).
const SEGMENT_FRAMES: usize = 16384;

/// Decode one interleaved chunk into channel-major float rows with exactly
/// the legacy normalization (`i16 as f64 / 32768.0`, applied only here).
fn chunk_to_float_rows(data: &[u8], channels: usize) -> Vec<Vec<f64>> {
    let frames = data.len() / (2 * channels);
    let mut rows = vec![Vec::with_capacity(frames); channels];
    for frame in 0..frames {
        for (slot, row) in rows.iter_mut().enumerate() {
            let offset = (frame * channels + slot) * 2;
            let sample = i16::from_le_bytes([data[offset], data[offset + 1]]);
            row.push(sample as f64 / 32768.0);
        }
    }
    rows
}

impl Default for StreamSession {
    fn default() -> Self {
        Self::new()
    }
}
