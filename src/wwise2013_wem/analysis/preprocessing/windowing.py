"""Materialize hybrid-windowed PCM frames from immutable frame decisions."""

from __future__ import annotations

from dataclasses import dataclass
from typing import Iterator, Sequence

from ...scheduling.model import DEFAULT_BLOCKSIZES, FramePlan
from ...scheduling.planner import (
    _validate_modes,
    plan_mode_sequence,
)


@dataclass(frozen=True)
class WindowedFrame:
    """One scheduler-planned, channel-major PCM window.

    Frame identity and mode transitions are properties of ``plan``.  Keeping
    them there makes the immutable scheduler decision the only source of
    scheduling truth throughout analysis.
    """

    plan: FramePlan
    center: int
    samples: tuple[tuple[float, ...], ...]

    def __post_init__(self) -> None:
        if not isinstance(self.plan, FramePlan):
            raise TypeError("windowed frame requires a FramePlan")

    @property
    def index(self) -> int:
        return self.plan.index

    @property
    def previous(self) -> int:
        return self.plan.previous

    @property
    def current(self) -> int:
        return self.plan.current

    @property
    def following(self) -> int:
        return self.plan.following


def iter_pcm_windows(
    pcm: Sequence[Sequence[float]],
    modes: Sequence[int],
    *,
    blocksizes: Sequence[int] = DEFAULT_BLOCKSIZES,
    terminal_following: int = 1,
) -> Iterator[WindowedFrame]:
    """Yield hybrid-windowed PCM blocks for an already-decided mode stream.

    The selector remains external: callers provide the current-mode sequence
    (or equivalently its already chosen following decisions).  This function
    owns deterministic PCM feeding: LPC priming before packet zero,
    overlap-center advances, short-window projection, and the LPC-padded
    end-of-stream block.
    """
    plans = plan_mode_sequence(
        modes,
        blocksizes=blocksizes,
        terminal_following=terminal_following,
    )
    yield from iter_planned_pcm_windows(pcm, plans, blocksizes=blocksizes)


def iter_planned_pcm_windows(
    pcm: Sequence[Sequence[float]],
    plans: Sequence[FramePlan],
    *,
    blocksizes: Sequence[int] = DEFAULT_BLOCKSIZES,
) -> Iterator[WindowedFrame]:
    """Materialize PCM exclusively from scheduler-owned frame plans."""
    if not plans:
        return
    for index, plan in enumerate(plans):
        _validate_modes(
            blocksizes, plan.previous, plan.current, plan.following
        )
        if plan.index != index:
            raise ValueError("frame plans must be contiguous and zero-based")
        expected_size = int(blocksizes[plan.current])
        if plan.sample_end - plan.sample_start != expected_size:
            raise ValueError("frame plan interval differs from its block mode")
        if index and (
            plans[index - 1].current != plan.previous
            or plans[index - 1].following != plan.current
        ):
            raise ValueError("adjacent frame plan transitions differ")
    if not pcm:
        raise ValueError("PCM feeder needs at least one channel")
    source_len = len(pcm[0])
    if source_len < 4096:
        raise ValueError("PCM feeder needs 4096 samples for Wwise LPC priming")
    if any(len(channel) != source_len for channel in pcm):
        raise ValueError("all PCM channels must have the same frame count")

    # Kept local to prevent the analysis module from depending on the
    # scheduler's WEM parser at import time.
    from ..dsp.lpc import (
        wwise_first_frame_lpc_prime,
        wwise_lpc_from_data,
        wwise_lpc_predict,
    )
    from ..dsp.transform import _f32, apply_vorbis_window

    channels = [[_f32(value) for value in channel] for channel in pcm]
    primes = [wwise_first_frame_lpc_prime(channel) for channel in channels]
    # EOF uses the regular-direction 32-tap predictor (distinct from the
    # reverse 16-tap bootstrap at packet zero).  The largest block view can
    # need nearly one complete long-block view past the source endpoint:
    # EOS can begin less than a short half-block before it.
    tail_count = max(int(size) for size in blocksizes)
    tails = [
        wwise_lpc_predict(
            wwise_lpc_from_data(channel[-4096:], order=32),
            channel[-32:],
            tail_count,
        )
        for channel in channels
    ]
    source_origin = int(blocksizes[1]) // 2
    for plan in plans:
        current = plan.current
        previous = plan.previous
        following = plan.following
        n = int(blocksizes[current])
        start = plan.sample_start - source_origin
        center = start + n // 2
        rows: list[tuple[float, ...]] = []
        for channel, prime, tail in zip(channels, primes, tails):
            raw: list[float] = []
            for sample_index in range(start, start + n):
                if sample_index < 0:
                    prime_index = sample_index + len(prime)
                    value = prime[prime_index] if prime_index >= 0 else 0.0
                elif sample_index < source_len:
                    value = channel[sample_index]
                else:
                    tail_index = sample_index - source_len
                    value = tail[tail_index] if tail_index < len(tail) else 0.0
                raw.append(value)

            # Short blocks do not use pending long-transition flags; their
            # effective window is always the short one.
            window_modes = (0, 0, 0) if current == 0 else (
                previous, current, following
            )
            rows.append(tuple(apply_vorbis_window(raw, blocksizes, *window_modes)))
        yield WindowedFrame(
            plan=plan,
            center=center,
            samples=tuple(rows),
        )


__all__ = ["WindowedFrame", "iter_pcm_windows", "iter_planned_pcm_windows"]
