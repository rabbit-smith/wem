//! State-owning PCM-frame psychoacoustic orchestration.
//!
//! Mirrors Python `wwise_wem/analysis/session.py`. This is the runtime spine
//! that owns every mutable field surviving a frame boundary: transient
//! histories, the mode queue, floor-envelope channel state/history, and the
//! frame-global spectrum peak. It accepts PCM and immutable tables only.

use crate::config::{AnalysisError, AnalysisProfileResources};
use crate::model::{PsyFrame, SpectrumFrame};
use crate::preprocessing::detector_input::iter_detector_quanta;
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

/// The mutable state for one interleaved PCM conversion
/// (Python `AnalysisSession`).
pub struct AnalysisSession {
    pub channels: i64,
    pub sample_rate: i64,
    pub blocksizes: [i64; 2],
    pub resources: AnalysisProfileResources,
    transient_detector: TransientDetector,
    mode_selector: ModeSelector,
    short_psy_analyzer: ShortPsyAnalyzer,
    spectrum_peak: SpectrumPeakState,
    next_frame_index: i64,
    last_frame_modes: Option<(i64, i64)>,
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
        let mut session = Self {
            channels,
            sample_rate,
            blocksizes,
            resources,
            // Placeholders replaced by reset().
            transient_detector,
            mode_selector,
            short_psy_analyzer,
            spectrum_peak: SpectrumPeakState::new(),
            next_frame_index: 0,
            last_frame_modes: None,
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
        self.next_frame_index = 0;
        self.last_frame_modes = None;
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
        let quanta = iter_detector_quanta(pcm, 64, 128, None, 8192, &self.blocksizes)?;
        for quantum in quanta {
            self.ingest_transient_quantum(&quantum)?;
        }

        let prefix = self.blocksizes[1] / 2;
        let stop_center = source_len + prefix;
        let mut center = 0;
        let mut current = 0;
        let mut modes: Vec<i64> = Vec::new();
        while center < stop_center {
            let status = self.mode_selection_status(center + prefix, current)?;
            let following = if status < 0 { 0 } else { status };
            modes.push(current);
            center +=
                self.blocksizes[current as usize] / 4 + self.blocksizes[following as usize] / 4;
            current = following;
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
        let windows =
            iter_planned_pcm_windows(pcm, &plans, &self.blocksizes, self.frozen_windows())?;
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
        update_gate: i64,
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
        self.analyze_short_inner(window, short_variant, q, update_gate, groups)
    }

    fn analyze_short_inner(
        &mut self,
        window: WindowedFrame,
        short_variant: Option<i64>,
        q: f64,
        update_gate: i64,
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
            update_gate,
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
