#!/usr/bin/env python3
"""State-owning PCM-frame psychoacoustic orchestration.

This module is the runtime spine between the independently checked pieces of
the Wwise encoder.  It deliberately owns *all* mutable state that survives a
frame boundary:

transient histories -> mode queue -> floor-envelope channel state/history ->
frame-global spectrum peak.

It accepts PCM and immutable tables/setup objects only.  The session owns the
selector's native ``-1/0/1`` lookahead decisions together with the mode-scan
cursor, so batch and streaming callers share one transition path.

This centralizes ownership; it does not waive numeric acceptance.  The
full-stream MDCT/log boundary is word-exact through immutable trigonometric
tables.  Every selected frame then proceeds through floor and residue packing
before final container assembly.
"""
from __future__ import annotations

from dataclasses import dataclass, field
from typing import Iterator, Mapping, Sequence

from .config import AnalysisProfileResources
from .model import PsyFrame, SpectrumFrame
from .psychoacoustics.pipeline import analyze_long_frame, analyze_short_frame
from .psychoacoustics.seed import SpectrumPeakState
from .psychoacoustics.short import ShortPsyAnalyzer
from ..scheduling.selector import ModeSelector
from .preprocessing.conditioner import InputConditioner
from .preprocessing.detector_input import detector_pcm_streams
from ..scheduling.planner import plan_mode_sequence
from .preprocessing.windowing import WindowedFrame, iter_pcm_windows, iter_planned_pcm_windows
from .transient.detector import TransientDetector


@dataclass
class _ModeScanState:
    next_center: int = 0
    previous_center: int = -1
    previous_mode: int = 0
    current_mode: int = 0

    def has_source_frame(self, source_len: int) -> bool:
        return self.previous_center < 0 or self.previous_center < source_len


@dataclass
class AnalysisSession:
    """The mutable state for one interleaved PCM conversion.

    The object is intentionally single-use for one logical audio stream.
    ``reset`` starts an independent stream and clears all detector/floor
    histories.  The state container accepts any positive channel count so
    ordinary packer/unit tests can exercise it with a smaller mapping.
    """

    channels: int
    sample_rate: int
    blocksizes: tuple[int, int]
    resources: AnalysisProfileResources
    _transient_detector: TransientDetector = field(init=False)
    _mode_selector: ModeSelector = field(init=False)
    _mode_scan: _ModeScanState = field(init=False)
    _short_psy_analyzer: ShortPsyAnalyzer = field(init=False)
    _spectrum_peak: SpectrumPeakState = field(init=False)
    _next_frame_index: int = field(init=False)
    _last_frame_modes: tuple[int, int] | None = field(init=False)
    _input_conditioner: InputConditioner | None = field(init=False)
    _frame_transition_codes: list[int] = field(init=False)
    _transition_codes_captured: bool = field(init=False)
    _eos_training_samples: int = field(init=False)

    def __post_init__(self) -> None:
        if self.channels <= 0:
            raise ValueError("stream needs at least one channel")
        if self.sample_rate <= 0:
            raise ValueError("sample rate must be positive")
        if not isinstance(self.resources, AnalysisProfileResources):
            raise TypeError("analysis resources must be AnalysisProfileResources")
        if len(self.blocksizes) != 2:
            raise ValueError("the checked stream state expects 256/2048 blocks")
        blocksizes = (int(self.blocksizes[0]), int(self.blocksizes[1]))
        if blocksizes != (256, 2048):
            raise ValueError("the checked stream state expects 256/2048 blocks")
        self.blocksizes = blocksizes
        self.reset()

    @property
    def short_bins(self) -> int:
        return self.blocksizes[0] // 2

    @property
    def long_bins(self) -> int:
        return self.blocksizes[1] // 2

    @property
    def transient_quanta(self) -> int:
        """Compatibility view of the detector quantum counter."""
        return self._transient_detector.quanta

    def reset(self) -> None:
        """Clear every cross-frame state field for a new conversion."""
        self._transient_detector = TransientDetector(
            self.channels,
            self.resources.transient,
            self.resources.mdct_looks[self.short_bins],
            self.short_bins,
        )
        # The detector owns one short-bin PCM window and produces one decision
        # queue entry per half-window.  The initial scan position is one
        # long-block half-window into the prefixed detector timeline.
        detector_hop = self.short_bins // 2
        self._mode_selector = ModeSelector(
            hop=detector_hop,
            capacity=self.short_bins,
            cooldown=0,
            generated=0,
            selected=0,
            scan_cursor=self.long_bins,
            queue=[0] * self.short_bins,
        )
        # One allocation owner is shared by the short active path and long
        # regular path.
        self._short_psy_analyzer = ShortPsyAnalyzer(self.channels, profiles=self.resources.short_profiles)
        self._spectrum_peak = SpectrumPeakState()
        self._mode_scan = _ModeScanState()
        self._next_frame_index = 0
        self._last_frame_modes = None
        self._input_conditioner = (
            InputConditioner(self.channels, self.resources.input_conditioner)
            if self.resources.input_conditioner is not None
            else None
        )
        self._frame_transition_codes = []
        self._transition_codes_captured = False
        self._eos_training_samples = self.blocksizes[1]

    def condition_pcm(self, pcm: Sequence[Sequence[float]]) -> tuple[tuple[float, ...], ...]:
        """Condition the next contiguous PCM rows for this stream."""
        if len(pcm) != self.channels:
            raise ValueError("PCM channel count differs from stream")
        if self._input_conditioner is None:
            return tuple(tuple(row) for row in pcm)
        return self._input_conditioner.process(pcm)

    def _consume_analysis_window(self, window: WindowedFrame) -> None:
        """Validate and consume one scheduler-bound analysis frame."""
        if window.index != self._next_frame_index:
            raise RuntimeError(
                f"analysis frames must be contiguous: expected index "
                f"{self._next_frame_index}, got {window.index}"
            )
        if self._last_frame_modes is not None:
            last_current, last_following = self._last_frame_modes
            if (
                window.previous != last_current
                or window.current != last_following
            ):
                raise RuntimeError("adjacent analysis frame modes differ")
        self._next_frame_index += 1
        self._last_frame_modes = (window.current, window.following)

    def _grow_selector_for_quantum(self, index: int) -> None:
        # finish_quantum clears index+2 before it can set the two preceding
        # look-ahead slots.  Keep that write capacity explicit here because
        # this direct PCM ingestion path intentionally does not use
        # ModeSelector.generate's precomputed flags iterator.
        required = index + 3
        if required > self._mode_selector.capacity:
            self._mode_selector.queue.extend([0] * (required - self._mode_selector.capacity))
            self._mode_selector.capacity = required

    def ingest_transient_quantum(self, pcm_by_channel: Sequence[Sequence[float]]) -> int:
        """Run exactly one 128-sample transient quantum across all channels.

        Advance and cap the shared history window, analyze every channel with
        that value, combine their flags, and store the mode-queue decision.
        The return value is the combined transient flag word.
        """
        expected_generated = self.transient_quanta * self._mode_selector.hop
        if self._mode_selector.generated != expected_generated:
            raise RuntimeError("manual transient ingestion cannot be mixed with mode generation")
        history_window = self._mode_selector.begin_quantum()
        quantum_index = self.transient_quanta
        flags = self._transient_detector.analyze_quantum(
            pcm_by_channel, history_window=history_window
        )
        self._grow_selector_for_quantum(quantum_index)
        self._mode_selector.finish_quantum(quantum_index, flags)
        self._mode_selector.generated = self.transient_quanta * self._mode_selector.hop
        return flags

    def ingest_transient_quanta(
        self, quanta: Sequence[Sequence[Sequence[float]]]
    ) -> tuple[int, ...]:
        """Ingest an ordered sequence of raw PCM transient quanta."""
        return tuple(self.ingest_transient_quantum(quantum) for quantum in quanta)

    def mode_selection_status(self, *, center: int, current_mode: int) -> int:
        """Return the mode queue's native ``-1/0/1`` look-ahead decision."""
        return self._mode_selector.scan(
            center=int(center), current_mode=int(current_mode), blocksizes=self.blocksizes
        )

    def _scan_next_mode(self, *, eos: bool) -> tuple[int, int, int, int] | None:
        """Advance one mode decision and preserve its scan-time profile code."""
        state = self._mode_scan
        prefix = self.blocksizes[1] // 2
        status = self.mode_selection_status(
            center=state.next_center + prefix,
            current_mode=state.current_mode,
        )
        if status < 0 and not eos:
            return None
        following = 0 if status < 0 else status
        self._frame_transition_codes.append(
            self._mode_selector.transition_code(
                center=state.next_center + prefix,
                previous_mode=state.previous_mode,
                current_mode=state.current_mode,
                following_mode=following,
                blocksizes=self.blocksizes,
            )
        )
        self._transition_codes_captured = True
        decision = (
            state.next_center,
            state.previous_mode,
            state.current_mode,
            following,
        )
        state.previous_center = state.next_center
        state.next_center += (
            self.blocksizes[state.current_mode] // 4
            + self.blocksizes[following] // 4
        )
        state.previous_mode, state.current_mode = state.current_mode, following
        return decision

    def transition_code(self, window: WindowedFrame) -> int:
        """Derive the psychoacoustic profile code from modes and transients."""
        if self._transition_codes_captured:
            if 0 <= window.index < len(self._frame_transition_codes):
                return self._frame_transition_codes[window.index]
            raise RuntimeError(
                f"transition code missing for frame {window.index}; "
                f"recorded {len(self._frame_transition_codes)}"
            )
        # Window centers use PCM sample zero as their origin.  The detector's
        # absolute timeline starts at the reverse-LPC prefix.
        detector_center = int(window.center) + self.blocksizes[1] // 2
        return self._mode_selector.transition_code(
            center=detector_center,
            previous_mode=window.previous,
            current_mode=window.current,
            following_mode=window.following,
            blocksizes=self.blocksizes,
        )

    def windows(
        self,
        pcm: Sequence[Sequence[float]],
        modes: Sequence[int],
        *,
        terminal_following: int = 1,
    ) -> Iterator[WindowedFrame]:
        """Yield windowed PCM blocks for an already selected mode sequence."""
        if len(pcm) != self.channels:
            raise ValueError("PCM channel count differs from stream")
        return iter_pcm_windows(
            pcm,
            modes,
            blocksizes=self.blocksizes,
            terminal_following=terminal_following,
            frozen_windows=self._frozen_windows(),
        )

    def select_modes(self, pcm: Sequence[Sequence[float]]) -> tuple[int, ...]:
        """Run transient detection and return the emitted mode sequence.

        Generate the source-backed detector timeline first, scan every mode
        decision it can support, then build the EOS predictor from the PCM
        still buffered at that point. Selector positions stay on the absolute
        timeline; scheduled PCM centers are offset by the 1024-sample prefix.
        """
        if len(pcm) != self.channels:
            raise ValueError("PCM channel count differs from stream")
        if self.transient_quanta or self._mode_selector.generated:
            raise RuntimeError("mode selection requires a fresh stream selector")
        source_len = len(pcm[0]) if pcm else 0
        if source_len < 4096 or any(len(channel) != source_len for channel in pcm):
            raise ValueError("mode selection PCM must be equal-length and at least 4096 samples")
        hop = self._mode_selector.hop
        detector_streams = detector_pcm_streams(
            pcm,
            terminal_samples=0,
            blocksizes=self.blocksizes,
        )
        pre_eos_quanta = max(0, len(detector_streams[0]) // hop - 4)
        for quantum_index in range(pre_eos_quanta):
            start = quantum_index * hop
            self.ingest_transient_quantum(
                tuple(
                    tuple(channel[start : start + self.short_bins])
                    for channel in detector_streams
                )
            )

        modes: list[int] = []

        while self._mode_scan.has_source_frame(source_len):
            decision = self._scan_next_mode(eos=False)
            if decision is None:
                break
            modes.append(decision[2])
        self._eos_training_samples = min(
            self.blocksizes[1],
            self.blocksizes[1] // 2 + source_len - self._mode_scan.next_center,
        )
        if self._eos_training_samples <= 32:
            raise RuntimeError("EOS LPC training window is too short")
        detector_streams = detector_pcm_streams(
            pcm,
            terminal_samples=self.blocksizes[1] * 3,
            tail_training=self._eos_training_samples,
            blocksizes=self.blocksizes,
        )
        available = (len(detector_streams[0]) - self.short_bins) // hop + 1
        for quantum_index in range(pre_eos_quanta, available):
            start = quantum_index * hop
            self.ingest_transient_quantum(
                tuple(
                    tuple(channel[start : start + self.short_bins])
                    for channel in detector_streams
                )
            )
        while self._mode_scan.has_source_frame(source_len):
            decision = self._scan_next_mode(eos=True)
            if decision is None:
                raise RuntimeError("EOS mode scan did not emit a decision")
            modes.append(decision[2])
        if modes and modes[-1]:
            self._frame_transition_codes[-1] = 2 | int(
                bool(modes[-2] if len(modes) > 1 else 0)
            )
        return tuple(modes)

    def selected_windows(
        self, pcm: Sequence[Sequence[float]]
    ) -> tuple[tuple[int, ...], Iterator[WindowedFrame]]:
        """Select modes and return their windowed PCM iterator."""
        modes = self.select_modes(pcm)
        plans = plan_mode_sequence(
            modes, blocksizes=self.blocksizes, terminal_following=1
        )
        return modes, iter_planned_pcm_windows(
            pcm,
            plans,
            blocksizes=self.blocksizes,
            frozen_windows=self._frozen_windows(),
            tail_training=self._eos_training_samples,
        )

    def _frozen_windows(self) -> Mapping[int, tuple[float, ...]] | None:
        frozen = self.resources.frozen
        return frozen.window_halves if frozen is not None else None

    def analyze_short(
        self,
        window: WindowedFrame,
        *,
        short_variant: int | None = None,
        q: float = -1.0,
        update_gate: int = 0,
        groups: Sequence[Sequence[float]] | None = None,
    ) -> PsyFrame:
        """Run the exact state-owning short analysis path for one frame."""
        if window.current != 0 or any(
            len(row) != self.blocksizes[0] for row in window.samples
        ):
            raise ValueError(
                f"short analysis expects a {self.blocksizes[0]}-sample scheduled window"
            )
        self._consume_analysis_window(window)
        return self._analyze_short(
            window,
            short_variant=short_variant,
            q=q,
            update_gate=update_gate,
            groups=groups,
        )

    def _analyze_short(
        self,
        window: WindowedFrame,
        *,
        short_variant: int | None,
        q: float = -1.0,
        update_gate: int = 0,
        groups: Sequence[Sequence[float]] | None = None,
    ) -> PsyFrame:
        variant = (
            self.transition_code(window) & 1
            if short_variant is None
            else int(short_variant)
        )
        result = analyze_short_frame(
            window.samples,
            self._short_psy_analyzer,
            self.resources,
            short_variant=variant,
            following_mode=window.following,
            q=float(q),
            update_gate=int(update_gate),
            specmax_state=self._spectrum_peak,
            groups=groups,
        )
        spectrum = SpectrumFrame(
            window,
            tuple(tuple(row) for row in result.coefficients),
            tuple(tuple(row) for row in result.raw_mdct),
            tuple(tuple(row) for row in result.fft),
            tuple(result.channel_specmax),
            result.global_specmax,
        )
        return PsyFrame(
            spectrum,
            tuple(tuple(row) for row in result.remap),
            tuple(tuple(row) for row in result.seed),
            tuple(tuple(row) for row in result.post),
            tuple(tuple(row) for row in result.side),
            tuple(tuple(row) for row in result.coupling_peak),
        )

    def analyze_long(self, window: WindowedFrame) -> PsyFrame:
        """Run the local long static profile and commit a possible 1024->128 edge."""
        if window.current != 1 or any(
            len(row) != self.blocksizes[1] for row in window.samples
        ):
            raise ValueError(
                f"long analysis expects a {self.blocksizes[1]}-sample scheduled window"
            )
        self._consume_analysis_window(window)
        return self._analyze_long(window)

    def _analyze_long(self, window: WindowedFrame) -> PsyFrame:
        # The current long wrapper has the normal raw->state tail.  Preserve
        # the incoming shared allocation first so a following short block can
        # apply the documented 8:1 reduction after the analysis result exists.
        result = analyze_long_frame(
            window.samples,
            resources=self.resources,
            stream=self._short_psy_analyzer,
            long_variant=self.transition_code(window) & 1,
            following_mode=window.following,
            specmax_state=self._spectrum_peak,
        )
        spectrum = SpectrumFrame(
            window,
            tuple(tuple(row) for row in result.coefficients),
            tuple(tuple(row) for row in result.raw_mdct),
            tuple(tuple(row) for row in result.fft),
            tuple(result.channel_specmax),
            result.global_specmax,
        )
        return PsyFrame(
            spectrum,
            tuple(tuple(row) for row in result.remap),
            tuple(tuple(row) for row in result.seed),
            tuple(tuple(row) for row in result.post),
            tuple(tuple(row) for row in result.side),
            tuple(tuple(row) for row in result.coupling_peak),
        )

    def analyze_window(
        self,
        window: WindowedFrame,
        *,
        short_variant: int | None = None,
    ) -> PsyFrame:
        """Dispatch one scheduled block to its short or long local analysis path."""
        expected_size = (
            self.blocksizes[window.current] if window.current in (0, 1) else 0
        )
        if not expected_size or any(
            len(row) != expected_size for row in window.samples
        ):
            raise ValueError("analysis window samples differ from its scheduled mode")
        self._consume_analysis_window(window)
        if window.current == 1:
            return self._analyze_long(window)
        return self._analyze_short(window, short_variant=short_variant)
