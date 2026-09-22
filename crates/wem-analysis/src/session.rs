//! State-owning PCM-frame psychoacoustic orchestration.
//!
//! Mirrors Python `wwise_wem/analysis/session.py`. This is the runtime spine
//! that owns every mutable field surviving a frame boundary: transient
//! histories, the mode queue, floor-envelope channel state/history, and the
//! frame-global spectrum peak. It accepts PCM and immutable tables only.

use crate::config::{AnalysisError, AnalysisProfileResources};
use crate::model::{PsyFrame, SpectrumFrame};
use crate::preprocessing::conditioner::InputConditioner;
use crate::preprocessing::detector_input::detector_pcm_streams;
use crate::preprocessing::windowing::{iter_pcm_windows, iter_planned_pcm_windows, WindowedFrame};
use crate::psychoacoustics::pipeline::{analyze_long_frame, analyze_short_frame};
use crate::psychoacoustics::seed::SpectrumPeakState;
use crate::psychoacoustics::short::ShortPsyAnalyzer;
use crate::transient::detector::TransientDetector;
use wem_scheduling::{plan_mode_sequence, ModeSelector, SelectorError};

fn selector_err(e: SelectorError) -> AnalysisError {
    use AnalysisError::*;
    match e {
        SelectorError::HopCapacityNonPositive => SessionChannelsNonPositive { channels: 0 },
        SelectorError::QueueSlotOutOfRange { .. } => AnalysisFrameNotContiguous {
            expected: 0,
            got: 0,
        },
        _ => UnsupportedGeometry {
            reason: "mode selector rejected a state transition",
        },
    }
}

/// One mode decision captured at the selector's scan point.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ModeScanDecision {
    pub center: i64,
    pub previous: i64,
    pub current: i64,
    pub following: i64,
}

/// Cross-frame mode selection history. AnalysisSession is its sole owner for
/// both batch and streaming conversions.
#[derive(Debug, Clone, Copy)]
struct ModeScanState {
    next_center: i64,
    previous_center: i64,
    previous_mode: i64,
    current_mode: i64,
}

impl ModeScanState {
    fn fresh() -> Self {
        Self {
            next_center: 0,
            previous_center: -1,
            previous_mode: 0,
            current_mode: 0,
        }
    }

    fn has_source_frame(&self, source_len: i64) -> bool {
        self.previous_center < 0 || self.previous_center < source_len
    }
}

/// The mutable state for one interleaved PCM conversion
/// (Python `AnalysisSession`).
pub struct AnalysisSession {
    pub channels: i64,
    pub sample_rate: i64,
    pub blocksizes: [i64; 2],
    pub resources: AnalysisProfileResources,
    transient_detector: TransientDetector,
    mode_selector: ModeSelector,
    mode_scan: ModeScanState,
    short_psy_analyzer: ShortPsyAnalyzer,
    spectrum_peak: SpectrumPeakState,
    next_frame_index: i64,
    last_frame_modes: Option<(i64, i64)>,
    input_conditioner: Option<InputConditioner>,
    frame_transition_codes: Vec<i64>,
    transition_codes_captured: bool,
    eos_training_samples: i64,
}

/// A summary, deliberately: the geometry, the frame cursor and the counts of
/// the state that grows with the stream.
///
/// The profile resources are thousands of frozen table values and the
/// per-frame vectors are as long as the encode is, so neither is printed; the
/// fields here are what tells two sessions apart — which geometry one runs on
/// and how far it has got.
impl std::fmt::Debug for AnalysisSession {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("AnalysisSession")
            .field("channels", &self.channels)
            .field("sample_rate", &self.sample_rate)
            .field("blocksizes", &self.blocksizes)
            .field("next_frame_index", &self.next_frame_index)
            .field("last_frame_modes", &self.last_frame_modes)
            .field("input_conditioner", &self.input_conditioner.is_some())
            .field("frame_transition_codes", &self.frame_transition_codes.len())
            .field("transition_codes_captured", &self.transition_codes_captured)
            .field("eos_training_samples", &self.eos_training_samples)
            .finish()
    }
}

impl AnalysisSession {
    /// Validate the construction inputs (Python `__post_init__`).
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        channels: i64,
        sample_rate: i64,
        blocksizes: [i64; 2],
        resources: AnalysisProfileResources,
    ) -> Result<Self, AnalysisError> {
        use AnalysisError::*;
        if channels <= 0 {
            return Err(SessionChannelsNonPositive { channels });
        }
        if sample_rate <= 0 {
            return Err(SessionSampleRateNonPositive { sample_rate });
        }
        if blocksizes != [256, 2048] {
            return Err(SessionBlockSizeMismatch {
                got: [blocksizes[0], blocksizes[1]],
            });
        }
        let transient_detector = TransientDetector::new(
            channels,
            resources.transient.clone(),
            resources.mdct_looks[&128].clone(),
            128,
        )
        .map_err(|_| UnsupportedGeometry {
            reason: "transient detector geometry",
        })?;
        let mode_selector =
            ModeSelector::new(64, 128, 0, 0, 0, 1024, vec![0; 128]).map_err(selector_err)?;
        let short_psy_analyzer =
            ShortPsyAnalyzer::new(channels, resources.short_profiles.clone(), None, None).map_err(
                |_| UnsupportedGeometry {
                    reason: "short psychoacoustic analyzer geometry",
                },
            )?;
        let input_conditioner = resources
            .input_conditioner
            .as_ref()
            .map(|config| InputConditioner::new(channels, config))
            .transpose()?;
        let mut session = Self {
            channels,
            sample_rate,
            blocksizes,
            resources,
            // Placeholders replaced by reset().
            transient_detector,
            mode_selector,
            mode_scan: ModeScanState::fresh(),
            short_psy_analyzer,
            spectrum_peak: SpectrumPeakState::new(),
            next_frame_index: 0,
            last_frame_modes: None,
            input_conditioner,
            frame_transition_codes: Vec::new(),
            transition_codes_captured: false,
            eos_training_samples: blocksizes[1],
        };
        session.reset();
        Ok(session)
    }

    /// The short block's transform bins (Python `short_bins` property).
    pub fn short_bins(&self) -> i64 {
        self.blocksizes[0] / 2
    }

    /// The long block's transform bins (Python `long_bins` property).
    pub fn long_bins(&self) -> i64 {
        self.blocksizes[1] / 2
    }

    /// Compatibility view of the detector quantum counter (Python
    /// `transient_quanta` property).
    pub fn transient_quanta(&self) -> i64 {
        self.transient_detector.quanta
    }

    /// Clear every cross-frame state field for a new conversion
    /// (Python `reset`).
    pub fn reset(&mut self) {
        let short_bins = self.short_bins();
        let long_bins = self.long_bins();
        if let Ok(detector) = TransientDetector::new(
            self.channels,
            self.resources.transient.clone(),
            self.resources.mdct_looks[&short_bins].clone(),
            short_bins,
        ) {
            self.transient_detector = detector;
        }
        let detector_hop = short_bins / 2;
        if let Ok(selector) = ModeSelector::new(
            detector_hop,
            short_bins,
            0,
            0,
            0,
            long_bins,
            vec![0; short_bins as usize],
        ) {
            self.mode_selector = selector;
        }
        if let Ok(analyzer) = ShortPsyAnalyzer::new(
            self.channels,
            self.resources.short_profiles.clone(),
            None,
            None,
        ) {
            self.short_psy_analyzer = analyzer;
        }
        self.spectrum_peak = SpectrumPeakState::new();
        self.mode_scan = ModeScanState::fresh();
        self.next_frame_index = 0;
        self.last_frame_modes = None;
        self.frame_transition_codes.clear();
        self.transition_codes_captured = false;
        self.eos_training_samples = self.blocksizes[1];
        if let Some(conditioner) = self.input_conditioner.as_mut() {
            conditioner.reset();
        }
    }

    /// Condition the next contiguous PCM rows for this stream.
    pub fn condition_pcm(&mut self, pcm: &[Vec<f64>]) -> Result<Vec<Vec<f64>>, AnalysisError> {
        if pcm.len() as i64 != self.channels {
            return Err(AnalysisError::InputConditionerChannelCountMismatch {
                want: self.channels,
                got: pcm.len() as i64,
            });
        }
        match self.input_conditioner.as_mut() {
            Some(conditioner) => conditioner.process(pcm),
            None => Ok(pcm.to_vec()),
        }
    }

    /// Validate and consume one scheduler-bound analysis frame
    /// (Python `_consume_analysis_window`).
    fn consume_analysis_window(&mut self, window: &WindowedFrame) -> Result<(), AnalysisError> {
        use AnalysisError::*;
        if window.index() != self.next_frame_index {
            return Err(AnalysisFrameNotContiguous {
                expected: self.next_frame_index,
                got: window.index(),
            });
        }
        if let Some((last_current, last_following)) = self.last_frame_modes {
            if window.previous() != last_current || window.current() != last_following {
                return Err(AdjacentAnalysisFrameModesDiffer);
            }
        }
        self.next_frame_index += 1;
        self.last_frame_modes = Some((window.current(), window.following()));
        Ok(())
    }

    /// Grow the selector queue before writing a quantum's decision
    /// (Python `_grow_selector_for_quantum`).
    fn grow_selector_for_quantum(&mut self, index: i64) {
        let required = index + 3;
        if required > self.mode_selector.capacity {
            let extra = required - self.mode_selector.capacity;
            self.mode_selector
                .queue
                .extend(std::iter::repeat_n(0, extra as usize));
            self.mode_selector.capacity = required;
        }
    }

    /// Run exactly one 128-sample transient quantum across all channels
    /// (Python `ingest_transient_quantum`).
    pub fn ingest_transient_quantum(
        &mut self,
        pcm_by_channel: &[Vec<f64>],
    ) -> Result<i64, AnalysisError> {
        use AnalysisError::*;
        let expected_generated = self.transient_quanta() * self.mode_selector.hop;
        if self.mode_selector.generated != expected_generated {
            return Err(ManualIngestionMixed);
        }
        let history_window = self.mode_selector.begin_quantum();
        let quantum_index = self.transient_quanta();
        let flags = self
            .transient_detector
            .analyze_quantum(pcm_by_channel, history_window)?;
        self.grow_selector_for_quantum(quantum_index);
        self.mode_selector
            .finish_quantum(quantum_index, flags)
            .map_err(selector_err)?;
        self.mode_selector.generated = self.transient_quanta() * self.mode_selector.hop;
        Ok(flags)
    }

    /// Ingest an ordered sequence of raw PCM transient quanta
    /// (Python `ingest_transient_quanta`).
    pub fn ingest_transient_quanta(
        &mut self,
        quanta: &[Vec<Vec<f64>>],
    ) -> Result<Vec<i64>, AnalysisError> {
        quanta
            .iter()
            .map(|quantum| self.ingest_transient_quantum(quantum))
            .collect()
    }

    /// Return the mode queue's native -1/0/1 look-ahead decision
    /// (Python `mode_selection_status`).
    pub fn mode_selection_status(
        &mut self,
        center: i64,
        current_mode: i64,
    ) -> Result<i64, AnalysisError> {
        self.mode_selector
            .scan(center, current_mode, &self.blocksizes)
            .map_err(selector_err)
    }

    /// Derive the psychoacoustic profile code from modes and transients
    /// (Python `transition_code`).
    pub fn transition_code(&self, window: &WindowedFrame) -> Result<i64, AnalysisError> {
        if self.transition_codes_captured {
            return self
                .frame_transition_codes
                .get(window.index() as usize)
                .copied()
                .ok_or(AnalysisError::TransitionCodeMissing {
                    index: window.index(),
                    recorded: self.frame_transition_codes.len(),
                });
        }
        let detector_center = window.center + self.blocksizes[1] / 2;
        self.mode_selector
            .transition_code(
                detector_center,
                window.previous(),
                window.current(),
                window.following(),
                &self.blocksizes,
            )
            .map_err(selector_err)
    }

    /// The center used by the next selector scan.
    pub fn next_mode_center(&self) -> i64 {
        self.mode_scan.next_center
    }

    /// The following mode captured by the most recent scan.
    pub fn pending_following_mode(&self) -> i64 {
        self.mode_scan.current_mode
    }

    /// Whether the batch mode loop still owns a source-backed frame.
    pub fn mode_scan_has_source_frame(&self, source_len: i64) -> bool {
        self.mode_scan.has_source_frame(source_len)
    }

    /// Run one mode-selector scan and capture its transition code before any
    /// later scan can advance the selector cursor.
    pub fn scan_next_mode(&mut self, eos: bool) -> Result<Option<ModeScanDecision>, AnalysisError> {
        let state = self.mode_scan;
        let prefix = self.blocksizes[1] / 2;
        let status = self.mode_selection_status(state.next_center + prefix, state.current_mode)?;
        if status < 0 && !eos {
            return Ok(None);
        }
        let following = if status < 0 { 0 } else { status };
        let code = self
            .mode_selector
            .transition_code(
                state.next_center + prefix,
                state.previous_mode,
                state.current_mode,
                following,
                &self.blocksizes,
            )
            .map_err(selector_err)?;
        self.frame_transition_codes.push(code);
        self.transition_codes_captured = true;
        self.mode_scan.previous_center = state.next_center;
        self.mode_scan.next_center += self.blocksizes[state.current_mode as usize] / 4
            + self.blocksizes[following as usize] / 4;
        self.mode_scan.previous_mode = state.current_mode;
        self.mode_scan.current_mode = following;
        Ok(Some(ModeScanDecision {
            center: state.next_center,
            previous: state.previous_mode,
            current: state.current_mode,
            following,
        }))
    }

    /// Apply the scheduler's terminal-following override to the last long
    /// frame. Short-frame profile codes do not depend on neighboring modes.
    pub fn finalize_terminal_transition(
        &mut self,
        previous_mode: i64,
        current_mode: i64,
    ) -> Result<(), AnalysisError> {
        let recorded = self.frame_transition_codes.len();
        let code = self
            .frame_transition_codes
            .last_mut()
            .ok_or(AnalysisError::TransitionCodeMissing { index: 0, recorded })?;
        if current_mode == 1 {
            *code = 2 | i64::from(previous_mode != 0);
        }
        Ok(())
    }

    /// Yield windowed PCM blocks for an already selected mode sequence
    /// (Python `windows`).
    pub fn windows(
        &self,
        pcm: &[Vec<f64>],
        modes: &[i64],
        terminal_following: i64,
    ) -> Result<Vec<WindowedFrame>, AnalysisError> {
        if (pcm.len() as i64) != self.channels {
            return Err(AnalysisError::PcmChannelsUnequal {
                want: self.channels,
                got: pcm.len() as i64,
            });
        }
        iter_pcm_windows(
            pcm,
            modes,
            &self.blocksizes,
            terminal_following,
            self.frozen_windows(),
            None,
        )
    }

    /// Run transient detection and return the emitted mode sequence
    /// (Python `select_modes`).
    pub fn select_modes(&mut self, pcm: &[Vec<f64>]) -> Result<Vec<i64>, AnalysisError> {
        use AnalysisError::*;
        if (pcm.len() as i64) != self.channels {
            return Err(PcmChannelsUnequal {
                want: self.channels,
                got: pcm.len() as i64,
            });
        }
        if self.transient_quanta() != 0 || self.mode_selector.generated != 0 {
            return Err(ModeSelectionNotFresh);
        }
        let source_len = pcm.first().map(|channel| channel.len() as i64).unwrap_or(0);
        if source_len < 4096 || pcm.iter().any(|channel| channel.len() as i64 != source_len) {
            return Err(ModeSelectionPcmInvalid { frames: source_len });
        }
        let hop = self.mode_selector.hop;
        let mut detector_streams = detector_pcm_streams(pcm, None, 0, None, &self.blocksizes)?;
        let pre_eos_quanta = (detector_streams[0].len() as i64 / hop - 4).max(0);
        for quantum_index in 0..pre_eos_quanta {
            let start = (quantum_index * hop) as usize;
            let end = start + self.short_bins() as usize;
            let quantum: Vec<Vec<f64>> = detector_streams
                .iter()
                .map(|channel| channel[start..end].to_vec())
                .collect();
            self.ingest_transient_quantum(&quantum)?;
        }

        let mut modes: Vec<i64> = Vec::new();

        while self.mode_scan_has_source_frame(source_len) {
            let Some(decision) = self.scan_next_mode(false)? else {
                break;
            };
            modes.push(decision.current);
        }

        self.eos_training_samples = self.blocksizes[1]
            .min(self.blocksizes[1] / 2 + source_len - self.mode_scan.next_center);
        if self.eos_training_samples <= 32 {
            return Err(UnsupportedGeometry {
                reason: "EOS LPC training window is too short",
            });
        }
        detector_streams = detector_pcm_streams(
            pcm,
            None,
            self.blocksizes[1] * 3,
            Some(self.eos_training_samples),
            &self.blocksizes,
        )?;
        let available = (detector_streams[0].len() as i64 - self.short_bins()) / hop + 1;
        for quantum_index in pre_eos_quanta..available {
            let start = (quantum_index * hop) as usize;
            let end = start + self.short_bins() as usize;
            let quantum: Vec<Vec<f64>> = detector_streams
                .iter()
                .map(|channel| channel[start..end].to_vec())
                .collect();
            self.ingest_transient_quantum(&quantum)?;
        }

        while self.mode_scan_has_source_frame(source_len) {
            let decision = self.scan_next_mode(true)?.ok_or(UnsupportedGeometry {
                reason: "EOS mode scan did not emit a decision",
            })?;
            modes.push(decision.current);
        }
        if modes.last() == Some(&1) {
            let previous = modes.iter().rev().nth(1).copied().unwrap_or(0);
            self.finalize_terminal_transition(previous, 1)?;
        }
        Ok(modes)
    }

    /// Select modes and return their windowed PCM frames
    /// (Python `selected_windows`).
    pub fn selected_windows(
        &mut self,
        pcm: &[Vec<f64>],
    ) -> Result<(Vec<i64>, Vec<WindowedFrame>), AnalysisError> {
        let modes = self.select_modes(pcm)?;
        let plans = plan_mode_sequence(&modes, &self.blocksizes, 1)
            .map_err(|_| AnalysisError::FramePlanIntervalMismatch)?;
        let windows = iter_planned_pcm_windows(
            pcm,
            &plans,
            &self.blocksizes,
            self.frozen_windows(),
            Some(self.eos_training_samples),
        )?;
        Ok((modes, windows))
    }

    /// The frozen Vorbis window halves, if present (Python `_frozen_windows`).
    fn frozen_windows(&self) -> Option<&std::collections::HashMap<i64, Vec<f32>>> {
        self.resources.frozen.as_ref().map(|f| &f.window_halves)
    }

    /// Run the exact state-owning short analysis path for one frame
    /// (Python `analyze_short`).
    pub fn analyze_short(
        &mut self,
        window: WindowedFrame,
        short_variant: Option<i64>,
        q: f64,
        hold_update: i64,
        groups: Option<&[Vec<f64>]>,
    ) -> Result<PsyFrame, AnalysisError> {
        if window.current() != 0
            || window
                .samples
                .iter()
                .any(|row| row.len() as i64 != self.blocksizes[0])
        {
            return Err(AnalysisError::ShortAnalysisWindowGeometry {
                want: self.blocksizes[0],
            });
        }
        self.consume_analysis_window(&window)?;
        self.analyze_short_inner(window, short_variant, q, hold_update, groups)
    }

    fn analyze_short_inner(
        &mut self,
        window: WindowedFrame,
        short_variant: Option<i64>,
        q: f64,
        hold_update: i64,
        groups: Option<&[Vec<f64>]>,
    ) -> Result<PsyFrame, AnalysisError> {
        let variant = match short_variant {
            None => self.transition_code(&window)? & 1,
            Some(v) => v,
        };
        let result = analyze_short_frame(
            &window.samples,
            &mut self.short_psy_analyzer,
            &self.resources,
            variant,
            window.following(),
            q,
            hold_update,
            crate::config::NEGATIVE_INFINITY_DB as f64,
            Some(&mut self.spectrum_peak),
            groups,
        )?;
        let spectrum = SpectrumFrame {
            window,
            coefficients: result.coefficients,
            raw_mdct: result.raw_mdct,
            fft: result.fft,
            channel_specmax: result.channel_specmax,
            global_specmax: result.global_specmax,
        };
        Ok(PsyFrame {
            spectrum,
            remap: result.remap,
            seed: result.seed,
            post: result.post,
            side: result.side,
            coupling_peak: result.coupling_peak,
        })
    }

    /// Run the local long static profile and commit a possible 1024->128 edge
    /// (Python `analyze_long`).
    pub fn analyze_long(&mut self, window: WindowedFrame) -> Result<PsyFrame, AnalysisError> {
        if window.current() != 1
            || window
                .samples
                .iter()
                .any(|row| row.len() as i64 != self.blocksizes[1])
        {
            return Err(AnalysisError::LongAnalysisWindowGeometry {
                want: self.blocksizes[1],
            });
        }
        self.consume_analysis_window(&window)?;
        self.analyze_long_inner(window)
    }

    fn analyze_long_inner(&mut self, window: WindowedFrame) -> Result<PsyFrame, AnalysisError> {
        // Compute the transition code before taking the mutable borrows so the
        // immutable and mutable self-borrows do not overlap in one expression.
        let long_variant = self.transition_code(&window)? & 1;
        let following_mode = window.following();
        let result = analyze_long_frame(
            &window.samples,
            crate::config::NEGATIVE_INFINITY_DB as f64,
            &self.resources,
            None,
            Some(&mut self.short_psy_analyzer),
            long_variant,
            following_mode,
            Some(&mut self.spectrum_peak),
        )?;
        let spectrum = SpectrumFrame {
            window,
            coefficients: result.coefficients,
            raw_mdct: result.raw_mdct,
            fft: result.fft,
            channel_specmax: result.channel_specmax,
            global_specmax: result.global_specmax,
        };
        Ok(PsyFrame {
            spectrum,
            remap: result.remap,
            seed: result.seed,
            post: result.post,
            side: result.side,
            coupling_peak: result.coupling_peak,
        })
    }

    /// Dispatch one scheduled block to its short or long local analysis path
    /// (Python `analyze_window`).
    pub fn analyze_window(
        &mut self,
        window: WindowedFrame,
        short_variant: Option<i64>,
    ) -> Result<PsyFrame, AnalysisError> {
        let expected_size = if window.current() == 0 || window.current() == 1 {
            self.blocksizes[window.current() as usize]
        } else {
            0
        };
        if expected_size == 0
            || window
                .samples
                .iter()
                .any(|row| row.len() as i64 != expected_size)
        {
            return Err(AnalysisError::AnalysisWindowSamplesMismatch);
        }
        self.consume_analysis_window(&window)?;
        if window.current() == 1 {
            self.analyze_long_inner(window)
        } else {
            self.analyze_short_inner(window, short_variant, -1.0, 0, None)
        }
    }
}
