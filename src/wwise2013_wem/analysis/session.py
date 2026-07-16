#!/usr/bin/env python3
"""State-owning PCM-frame psychoacoustic orchestration.

This module is the runtime spine between the independently checked pieces of
the Wwise encoder.  It deliberately owns *all* mutable state that survives a
frame boundary:

transient histories -> mode queue -> floor-envelope channel state/history ->
frame-global spectrum peak.

It accepts PCM and immutable tables/setup objects only.  The selector exposes
its native ``-1/0/1`` result: mapping a lookahead
decision to an actual emitted mode belongs to the caller that owns PCM
availability and frame readiness.

This centralizes ownership; it does not waive numeric acceptance.  The
full-stream MDCT/log boundary is word-exact through immutable trigonometric
tables.  Every selected frame then proceeds through floor and residue packing
before final container assembly.
"""
from __future__ import annotations

from dataclasses import dataclass, field
from typing import Iterator, Sequence

from .config import AnalysisProfileResources
from .model import PsyFrame, SpectrumFrame
from .psychoacoustics.pipeline import analyze_long_frame, analyze_short_frame
from .psychoacoustics.seed import SpectrumPeakState
from .psychoacoustics.short import ShortPsyAnalyzer
from ..scheduling.selector import ModeSelector
from .preprocessing.detector_input import iter_detector_quanta
from ..scheduling.planner import plan_mode_sequence
from .preprocessing.windowing import WindowedFrame, iter_pcm_windows, iter_planned_pcm_windows
from .transient.detector import TransientDetector

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
    _short_psy_analyzer: ShortPsyAnalyzer = field(init=False)
    _spectrum_peak: SpectrumPeakState = field(init=False)
    _next_frame_index: int = field(init=False)
    _last_frame_modes: tuple[int, int] | None = field(init=False)

    def __post_init__(self) -> None:
        if self.channels <= 0:
            raise ValueError("stream needs at least one channel")
        if self.sample_rate <= 0:
            raise ValueError("sample rate must be positive")
        if not isinstance(self.resources, AnalysisProfileResources):
            raise TypeError("analysis resources must be AnalysisProfileResources")
        blocksizes = tuple(int(value) for value in self.blocksizes)
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
        self._next_frame_index = 0
        self._last_frame_modes = None

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

    def transition_code(self, window: WindowedFrame) -> int:
        """Derive the psychoacoustic profile code from modes and transients."""
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
        )

    def select_modes(self, pcm: Sequence[Sequence[float]]) -> tuple[int, ...]:
        """Run transient detection and return the emitted mode sequence.

        The detector is geometry-independent, so its complete LPC-padded
        timeline can be generated before scanning block decisions.  Selector
        positions stay in that absolute timeline; scheduled PCM centers are
        offset by the 1024-sample detector prefix.
        """
        if len(pcm) != self.channels:
            raise ValueError("PCM channel count differs from stream")
        if self.transient_quanta or self._mode_selector.generated:
            raise RuntimeError("mode selection requires a fresh stream selector")
        source_len = len(pcm[0]) if pcm else 0
        if source_len < 4096 or any(len(channel) != source_len for channel in pcm):
            raise ValueError("mode selection PCM must be equal-length and at least 4096 samples")
        for quantum in iter_detector_quanta(pcm, blocksizes=self.blocksizes):
            self.ingest_transient_quantum(quantum)

        prefix = self.blocksizes[1] // 2
        stop_center = source_len + prefix
        center = 0
        current = 0
        modes: list[int] = []
        while center < stop_center:
            status = self.mode_selection_status(
                center=center + prefix, current_mode=current
            )
            # End of stream falls back to short mode when no additional
            # look-ahead decision exists.  A complete normal stream remains
            # at status 0/1 through its final emitted block.
            following = 0 if status < 0 else status
            modes.append(current)
            center += (
                self.blocksizes[current] // 4
                + self.blocksizes[following] // 4
            )
            current = following
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
            pcm, plans, blocksizes=self.blocksizes
        )

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
