#!/usr/bin/env python3
"""Short-block psychoacoustic floor-envelope analysis.

This is the short-block counterpart to the long analysis in ``pipeline.py``.  It
owns both parts that mutate the shared 128-bin state: history
rebase/relaxation, followed by peak continuation, floor-envelope shaping,
side attenuation, and eight-bin group-work updates.

The input profile is immutable setup data from the installed profile's
``psychoacoustics/short-profiles.json``. Per-call work vectors come from the normal
encoder scheduler; this module depends only on those runtime inputs.
"""

from __future__ import annotations

import struct
from dataclasses import dataclass
from typing import Sequence

from .temporal import TemporalKernelInputs, relax_short_history, compute_temporal_kernel
from ..config import ShortPsyProfile


N = 128
_NEG_17_2 = -17.20000076293945
# These tuning literals retain the exact double values of their source float
# spellings.  Rounding them to float before the final store loses one ULP on a
# handful of active short bins.
_F64_0_1 = 0.10000000149011612
_F64_0_2 = 0.20000000298023224
_F64_0_3 = 0.30000001192092896


def _f32(value: float) -> float:
    """Round at a float32 storage boundary."""
    return struct.unpack("<f", struct.pack("<f", float(value)))[0]


def _u32_f32(value: int) -> float:
    return struct.unpack("<f", struct.pack("<I", int(value) & 0xFFFFFFFF))[0]


@dataclass(frozen=True)
class ShortPsyKernel:
    """Temporal weights and state controls for short envelope analysis."""

    active: int
    update_history: int
    lower_weight: float
    upper_weight: float
    state_bias: float
    chase_seed: float

    @classmethod
    def from_words(cls, words: Sequence[int]) -> "ShortPsyKernel":
        if len(words) != 6:
            raise ValueError("short psychoacoustic kernel must contain six words")
        return cls(
            int(words[0]),
            int(words[1]),
            _u32_f32(int(words[2])),
            _u32_f32(int(words[3])),
            _u32_f32(int(words[4])),
            _u32_f32(int(words[5])),
        )


@dataclass(frozen=True)
class ShortPsyChannelResult:
    """All mutable output surfaces of one 128-bin channel analysis."""

    post: list[float]
    side: list[float]
    history: list[float]
    groups: list[float]
    peak_bins: int


@dataclass
class PsyChannelState:
    """Persistent state allocation shared by one encoder channel.

    The state curve has 1024 entries even while a short frame uses only its
    first 128; a short-to-long boundary expands the short curve across that
    allocation.  History is a distinct 128-float allocation.  Group work
    deliberately does not live here because it is prepared per call.
    """

    state: list[float]
    history: list[float]

    @classmethod
    def fresh(cls) -> "PsyChannelState":
        return cls([0.0] * 1024, [0.0] * N)


@dataclass
class PsyTemporalState:
    """Previous transition, equal-run count, and saturating tail count.

    These fields advance once after the complete channel group, not after
    each individual channel.
    """

    previous_transition: int = 0
    run_count: int = 0
    tail_count: int = 0


@dataclass(frozen=True)
class PsyFrameControls:
    """Scheduler-derived controls used for one short six-channel group."""

    transition: int
    previous_transition: int
    run_count: int
    tail_count: int
    following_mode: int
    profile_key: str


@dataclass(frozen=True)
class ShortPsyFrameResult:
    """Post/side outputs plus the exact controls consumed by a frame."""

    info: PsyFrameControls
    channels: tuple[ShortPsyChannelResult, ...]


class ShortPsyAnalyzer:
    """Own active-short psychoacoustic state for every channel.

    ``short_variant`` chooses immutable short profile zero or one.  The mode
    selector owns that decision; this class owns its deterministic temporal
    recurrence and each channel's state/history allocations.  Caller-built
    group-work vectors remain per-frame inputs rather than cross-frame state.
    """

    def __init__(
        self,
        channel_count: int = 6,
        *,
        profiles: Sequence[ShortPsyProfile],
        channels: Sequence[PsyChannelState] | None = None,
        temporal: PsyTemporalState | None = None,
    ) -> None:
        if channel_count <= 0:
            raise ValueError("short psychoacoustic analyzer needs at least one channel")
        source_profiles = tuple(profiles)
        if len(source_profiles) != 2:
            raise ValueError("short psychoacoustic analyzer needs exactly two profiles")
        self.profiles = source_profiles
        self.channels = (
            [PsyChannelState.fresh() for _ in range(channel_count)]
            if channels is None
            else list(channels)
        )
        if len(self.channels) != channel_count:
            raise ValueError("short psychoacoustic channel-state count differs")
        for channel in self.channels:
            if len(channel.state) != 1024 or len(channel.history) != N:
                raise ValueError("short psychoacoustic channel state has wrong geometry")
        self.temporal = temporal if temporal is not None else PsyTemporalState()

    def _frame_controls(self, short_variant: int, following_mode: int) -> PsyFrameControls:
        if short_variant not in (0, 1):
            raise ValueError("short psychoacoustic variant must be 0 or 1")
        if following_mode not in (0, 1):
            raise ValueError("short psychoacoustic following mode must be 0 or 1")
        # A short frame uses its selected profile directly.  Long frames use
        # the two higher transition codes in the shared state path.
        transition = int(short_variant)
        profile = self.profiles[transition]
        return PsyFrameControls(
            transition=transition,
            previous_transition=int(self.temporal.previous_transition),
            run_count=int(self.temporal.run_count),
            tail_count=int(self.temporal.tail_count),
            following_mode=int(following_mode),
            profile_key=profile.key,
        )

    def _advance_temporal_state(self, transition: int) -> None:
        """Advance temporal counters once after the channel group."""
        previous = int(self.temporal.previous_transition)
        tail = int(self.temporal.tail_count)
        if transition >= 2:
            tail = 0
        if previous or transition != 1:
            if tail and tail < 8:
                tail += 1
        else:
            tail = 1
        run = int(self.temporal.run_count) + 1 if previous == transition else 1
        self.temporal.previous_transition = int(transition)
        self.temporal.run_count = run
        self.temporal.tail_count = tail

    @staticmethod
    def _commit_state_transition(
        channel: PsyChannelState,
        raw: Sequence[float],
        *,
        info: PsyFrameControls,
        update_state: int,
    ) -> None:
        """Commit the short state curve for the following block geometry."""
        if not update_state:
            return
        if info.transition not in (0, 1):
            raise ValueError("short analysis cannot apply a long transition code")
        if info.following_mode:
            channel.state[:] = [_f32(value) for value in raw for _ in range(8)]
        else:
            channel.state[:N] = [_f32(value) for value in raw]

    def process_frame(
        self,
        remaps: Sequence[Sequence[float]],
        seeds: Sequence[Sequence[float]],
        sides: Sequence[Sequence[float]],
        raws: Sequence[Sequence[float]],
        *,
        short_variant: int,
        following_mode: int,
        q: float = -1.0,
        update_gate: int = 0,
        groups: Sequence[Sequence[float]] | None = None,
    ) -> ShortPsyFrameResult:
        """Run all channel calls of one short packet/frame.

        ``groups`` is optional because peak-group output is not an input to
        the floor, side, or persistent-state paths.  Omitting it creates a
        fresh zero work vector with the selected profile's natural size.
        """
        vectors = (remaps, seeds, sides, raws)
        count = len(self.channels)
        if any(len(values) != count for values in vectors):
            raise ValueError("short psychoacoustic frame channel count differs")
        if groups is not None and len(groups) != count:
            raise ValueError("short psychoacoustic group-work count differs")
        info = self._frame_controls(short_variant, following_mode)
        profile = self.profiles[info.transition]
        kernel_state = compute_temporal_kernel(
            TemporalKernelInputs(
                bins=N,
                run_count=info.run_count,
                tail_count=info.tail_count,
                high_rate=True,
                curve_bias=profile.candidate_bias_by_mode[1],
                transition_code=info.transition,
                previous_transition=info.previous_transition,
                update_gate=int(update_gate),
                analysis_mode=1,
            )
        )
        kernel = ShortPsyKernel(
            kernel_state.active,
            kernel_state.update_state,
            _f32(kernel_state.lower_weight),
            _f32(kernel_state.upper_weight),
            _f32(kernel_state.state_bias),
            _f32(kernel_state.chase_seed),
        )
        group_count = (N + profile.group_span - 1) // profile.group_span
        results: list[ShortPsyChannelResult] = []
        for index, (remap, seed, side, raw, channel) in enumerate(
            zip(remaps, seeds, sides, raws, self.channels)
        ):
            work = (
                groups[index]
                if groups is not None
                else [0.0] * group_count
            )
            result = shape_short_floor_envelope(
                profile,
                remap,
                seed,
                side,
                raw,
                channel.state[:N],
                channel.history,
                work,
                kernel,
                mode=1,
                q=q,
                tail_count=info.tail_count,
                previous_transition=info.previous_transition,
            )
            channel.history[:] = result.history
            self._commit_state_transition(
                channel, raw, info=info, update_state=kernel.update_history
            )
            results.append(result)
        self._advance_temporal_state(info.transition)
        return ShortPsyFrameResult(info, tuple(results))

    def commit_long_state(
        self,
        raws: Sequence[Sequence[float]],
        *,
        long_variant: int,
        following_mode: int,
        update_gate: int = 0,
    ) -> PsyFrameControls:
        """Advance the shared state allocation through a 1024-bin frame.

        Long floor arithmetic belongs to :func:`pipeline.analyze_long_frame`.
        This small bridge owns only its observable state edge so the next
        short call sees the correct 128-bin prefix: long-to-long stores the
        raw curve directly, while long-to-short reduces each eight-bin group
        by its minimum.  The long branch preserves short history.
        """
        if long_variant not in (0, 1):
            raise ValueError("long psychoacoustic variant must be 0 or 1")
        if following_mode not in (0, 1):
            raise ValueError("long psychoacoustic following mode must be 0 or 1")
        if len(raws) != len(self.channels) or any(len(raw) != 1024 for raw in raws):
            raise ValueError("long state bridge needs one 1024-bin raw curve per channel")
        transition = 2 | int(long_variant)
        info = PsyFrameControls(
            transition=transition,
            previous_transition=int(self.temporal.previous_transition),
            run_count=int(self.temporal.run_count),
            tail_count=int(self.temporal.tail_count),
            following_mode=int(following_mode),
            profile_key="long_state_only",
        )
        # Long-frame peak continuation is inactive; the update gate only
        # controls whether the state curve is committed.
        if not int(update_gate):
            for raw, channel in zip(raws, self.channels):
                if following_mode:
                    channel.state[:] = [_f32(value) for value in raw]
                else:
                    channel.state[:N] = [
                        _f32(min(float(value) for value in raw[index * 8 : index * 8 + 8]))
                        for index in range(N)
                    ]
        self._advance_temporal_state(transition)
        return info


def _prepare_history(
    raw: Sequence[float],
    state: Sequence[float],
    history: Sequence[float],
    kernel: ShortPsyKernel,
    *,
    previous_transition: int,
) -> list[float]:
    """Prepare the rebased and relaxed 128-bin history curve.

    An inactive kernel leaves history unchanged.  This keeps the helper tied
    to the computed temporal controls rather than duplicating selector state.
    """
    if not kernel.active or not kernel.update_history:
        return [_f32(value) for value in history]
    source = state if previous_transition else history
    rebased = [_f32(float(value) - 5.0) for value in source]
    return relax_short_history(rebased, raw)


def shape_short_floor_envelope(
    profile: ShortPsyProfile,
    remap: Sequence[float],
    seed: Sequence[float],
    side: Sequence[float],
    raw: Sequence[float],
    state: Sequence[float],
    history: Sequence[float],
    groups: Sequence[float],
    kernel: ShortPsyKernel,
    *,
    mode: int,
    q: float,
    tail_count: int,
    previous_transition: int,
) -> ShortPsyChannelResult:
    """Shape the active 128-bin floor envelope for one channel.

    Arithmetic is rounded at every observed float32 storage boundary.  Those
    boundaries affect later branch comparisons as well as output words.
    """
    vectors = (remap, seed, side, raw, state, history)
    if any(len(values) != N for values in vectors):
        raise ValueError("short psychoacoustic vectors must each contain 128 values")
    if mode < 0 or mode >= len(profile.mask_curves):
        raise ValueError("short psychoacoustic mode is outside the curve table")
    if mode >= len(profile.candidate_bias_by_mode):
        raise ValueError("short psychoacoustic mode is outside the bias table")
    expected_groups = (N + profile.group_span - 1) // profile.group_span
    if len(groups) != expected_groups:
        raise ValueError("short psychoacoustic group work has the wrong length")

    curve = profile.mask_curves[mode]
    candidate_bias = profile.candidate_bias_by_mode[mode]
    subtract = 0.0
    if float(q) >= 0.0 and candidate_bias >= 25.0:
        # Preserve extended precision until the product is stored as float32.
        subtract = _f32(float(q) * (candidate_bias - 25.0))

    # Prepare history before the envelope loop rather than requiring callers
    # to stage a temporary buffer.
    history_out = _prepare_history(
        raw, state, history, kernel, previous_transition=previous_transition
    )
    post = [0.0] * N
    side_out = [_f32(value) for value in side]
    groups_out = [_f32(value) for value in groups]

    # The chase minimum remains zero throughout this path.  It gates only the
    # group-work minimum and the negative side-factor clamp.
    chase_min = 0.0
    peak_bins = 0
    low_band, middle_band, fine_band = profile.band_limits

    for index in range(N):
        # Materialize both the cap and candidate as float32 before comparing.
        cap = _f32(float(curve[index]) + float(remap[index]))
        if profile.curve_cap < cap:
            cap = _f32(profile.curve_cap)
        candidate = _f32(float(seed[index]) + candidate_bias)
        if index <= profile.candidate_bound:
            candidate = _f32(candidate - subtract)

        peak = (
            bool(kernel.active)
            and candidate < cap
            and float(state[index]) < cap
            and _f32(float(history_out[index]) + kernel.state_bias) < float(raw[index])
        )
        marked_peak = False

        if peak:
            peak_bins += 1
            # This is the only raw overwrite after baseline relaxation.  It
            # must precede future frames that observe the output history.
            if kernel.update_history:
                history_out[index] = _f32(raw[index])

            weight = (
                kernel.upper_weight
                if float(state[index]) >= float(raw[index])
                else kernel.lower_weight
            )

            # Optionally chase strong peaks in the low-frequency region.
            if (
                not tail_count
                and index < profile.peak_cutoff
                and cap - float(state[index]) > 20.0
                and float(raw[index]) - float(state[index]) > 25.0
            ):
                marked_peak = True
                if candidate > -100.0 and float(raw[index]) - candidate < 48.0:
                    delta_state = float(raw[index]) - float(state[index])
                    reduction = (
                        kernel.chase_seed
                        if delta_state >= 35.0
                        else _f32(
                            (35.0 - delta_state)
                            * _F64_0_1
                            * kernel.chase_seed
                        )
                    )
                    candidate = _f32(max(_f32(candidate - reduction), -100.0))
                    if float(raw[index]) - candidate > 48.0:
                        candidate = _f32(float(raw[index]) - 48.0)

            # The nesting matters: bins 8 and 9 use width 20 rather than the
            # lower-band width-10 branch.
            if index <= low_band:
                if index <= middle_band:
                    width = 10.0
                    weight = _f32(
                        weight * (_F64_0_3 if index <= fine_band else 0.5)
                    )
                else:
                    width = 20.0
            else:
                width = 30.0

            # Keep the cap/candidate difference in extended precision through
            # optional smoothing and weighting; store only the final product.
            gap = float(cap) - float(candidate)
            if gap > width:
                gap = width + (gap - width) * _F64_0_1
            ceiling = _f32(cap - _f32(gap * weight))
            value = _f32(
                float(state[index]) if float(state[index]) >= ceiling else ceiling
            )

            # Peak marks restrain a curve more than 20 dB above the state
            # curve, floored at -140 dB.  Using raw here would preserve
            # history but shift marked peak posts by several dB.
            if marked_peak:
                state_floor = max(float(state[index]), -140.0)
                separation = _f32(value - state_floor)
                if separation > 20.0:
                    value = _f32(value - (separation - 20.0) * _F64_0_2)
        else:
            # Inactive bins use the cap directly.
            value = cap

        # Only active peaks update caller-owned group work.  Preserve those
        # writes for the next frame while ordinary bins leave the vector
        # untouched.
        if peak:
            group_index = index // profile.group_span
            if marked_peak:
                groups_out[group_index] = -1.0
            elif chase_min < groups_out[group_index]:
                groups_out[group_index] = _f32(chase_min)

        # The middle range is dormant for installed profiles whose candidate
        # bound is 9999, but retaining it keeps alternate profiles complete.
        if value <= candidate:
            if index <= profile.candidate_bound or index >= profile.peak_cutoff:
                output = candidate
            elif float(raw[index]) < candidate:
                if float(raw[index]) >= value:
                    output = _f32(raw[index])
                else:
                    output = _f32(
                        candidate
                        - _f32(candidate - value) * profile.blend_weight
                    )
            else:
                output = candidate
        else:
            output = value
        post[index] = _f32(output)

        # Side attenuation references the intermediate envelope value rather
        # than the post value.  A zero chase minimum selects the 1e-4 fallback
        # for negative factors.
        delta = _f32(value - float(raw[index]))
        distance = delta - _NEG_17_2
        if delta <= -17.200001:
            factor = _f32(
                1.0 - distance * 0.0003 * profile.side_gain
            )
        else:
            factor = _f32(
                1.0 - distance * 0.005 * profile.side_gain
            )
        if delta > -17.200001 and chase_min > factor:
            factor = 0.0001
        side_out[index] = _f32(side_out[index] * factor)

    return ShortPsyChannelResult(post, side_out, history_out, groups_out, peak_bins)
