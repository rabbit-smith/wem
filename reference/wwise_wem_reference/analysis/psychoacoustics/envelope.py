#!/usr/bin/env python3
"""Temporal floor-envelope shaping and history transitions."""
from __future__ import annotations

from dataclasses import dataclass
from typing import Sequence

from ..config import (
    LongFloorEnvelopeLook,
    WwisePsyLook,
    WwisePsyLongTables,
    make_long_floor_envelope_look,
)
from ..._f32 import _f32

@dataclass
class FloorEnvelopeScratch:
    """Per-channel scratch owned by the regular floor-envelope stage pass.

    Short regular calls retain the source spectrum in current curve and fill history curve with the
    -5.0 sentinel.  The observed long inactive call also retains the source spectrum but
    preserves its supplied history words.  Later-frame branch reconstruction
    can extend this holder without changing either local first-pass surface.
    """

    current_curve: list[float]
    history_curve: list[float]

def shape_floor_envelope(
    target_curve: Sequence[float],
    psycho_curve: Sequence[float],
    previous_curve: Sequence[float],
    base_curve: Sequence[float],
    side_curve: Sequence[float],
    mask_curve: Sequence[float],
    *,
    curve_bias: float,
    q: float,
    curve_cap: float,
    history_start: int,
    history_end: int,
    state_active: bool = False,
    history_weight: float = 0.0,
    side_gain: float = 1.0,
    mode: int = 1,
    source_curve: Sequence[float] | None = None,
) -> tuple[list[float], list[float], int | None]:
    """Port the no-peak branch of ``floor-envelope shaping routine``.

    The pointer arithmetic in the reference encoder is deliberately flattened here.  The
    first argument is the seed curve (the ATH/seed curve written by ``floor-seed stage``), while
    ``base_curve`` is the ``remap delta`` scratch read from the remap buffer.  ``source_curve``
    is the source spectrum (the log curve kept for history/blend checks); it defaults to
    the target curve for the small synthetic self-test.  The return value is
    the shaped envelope, side curve, and optional break index.

    ``break_index`` records entry into the later peak branch.  Returning it
    keeps the regular pass useful for packet diagnostics without silently
    treating that branch as matched.
    """
    n = len(target_curve)
    arrays = (psycho_curve, previous_curve, base_curve, side_curve, mask_curve)
    if any(len(values) != n for values in arrays):
        raise ValueError("floor-envelope stage curves must have equal lengths")
    if source_curve is None:
        source_curve = target_curve
    if len(source_curve) != n:
        raise ValueError("floor-envelope stage source curve must have equal length")
    if n == 0:
        return [], [], None
    if mode != 1:
        raise ValueError("floor-envelope stage regular port currently targets the regular branch")

    output = [0.0] * n
    side_output = [_f32(value) for value in side_curve]
    stop = int(history_start)
    blend_end = int(history_end)
    subtract = 0.0
    if float(q) >= 0.0 and float(curve_bias) >= 25.0:
        subtract = float(q) * (float(curve_bias) - 25.0)
    break_index: int | None = None

    for index in range(n):
        # remap delta scratch is a float buffer: the mask+remap candidate is
        # spilled before its cap comparison and later side attenuation.
        floor_value = min(
            _f32(float(mask_curve[index]) + float(base_curve[index])),
            float(curve_cap),
        )
        target = float(target_curve[index]) + float(curve_bias)
        if index <= stop:
            target -= subtract

        # floor-envelope stage enters the regular path directly unless the active state asks for
        # the peak continuation and the previous curve fails that comparison.
        if (
            state_active
            and target < floor_value
            and float(psycho_curve[index]) < floor_value
            and float(previous_curve[index]) < float(psycho_curve[index])
        ):
            break_index = index
            break

        if floor_value <= target:
            chosen = target
            if index > stop and index < blend_end:
                current = float(source_curve[index])
                if current < target:
                    if current >= floor_value:
                        chosen = current
                    else:
                        chosen = target - (target - floor_value) * float(
                            history_weight
                        )
            output[index] = _f32(chosen)
        else:
            output[index] = _f32(floor_value)

        # The reference encoder scales the side curve from the floor candidate rather than the
        # selected output envelope. Both the delta and attenuation factor retain
        # explicit float32 boundaries for deterministic long-frame coefficients.
        delta = _f32(floor_value - float(source_curve[index]))
        distance = delta + 17.20000076293945
        if delta <= -17.200001:
            factor = _f32(1.0 - distance * 0.0003 * float(side_gain))
        else:
            factor = _f32(1.0 - distance * 0.005 * float(side_gain))
            if factor < 0.0:
                factor = 0.0001
        side_output[index] = _f32(side_output[index] * factor)

    return output, side_output, break_index


def make_floor_envelope_scratch(n: int) -> FloorEnvelopeScratch:
    """Allocate the observed cleared first-call scratches."""
    if n <= 0:
        raise ValueError("floor-envelope stage scratch length must be positive")
    return FloorEnvelopeScratch([0.0] * n, [0.0] * n)


def make_channel_floor_envelope_scratch() -> FloorEnvelopeScratch:
    """Allocate one actual cross-block floor-envelope stage state pair.

    Runtime pointer strides establish a 1024-float state allocation and a
    128-float history allocation per channel.  A long-frame observation may include more
    than 128 history values only because its generic vector dump crosses into
    adjacent channel memory; those trailing words have no long-branch
    semantics.  Use this constructor for a stream scheduler; retain
    :func:`make_floor_envelope_scratch` for same-size diagnostic calls.
    """
    return FloorEnvelopeScratch([0.0] * 1024, [0.0] * 128)


def prepare_short_to_long_history(raw_short: Sequence[float]) -> list[float]:
    """Expand a 128-bin short floor-envelope stage state curve to 1024 bins.

    At a short→long transition the reference encoder's tail case writes each short raw log
    value into the corresponding contiguous eight-bin state group.  The
    shared state allocation is therefore 1024 bins even while the short floor
    pass itself handles only 128 curve bins.
    """
    if len(raw_short) != 128:
        raise ValueError("short→long floor-envelope stage state expansion expects 128 bins")
    return [_f32(value) for value in raw_short for _ in range(8)]


def prepare_long_to_short_history(
    raw_long: Sequence[float], previous_state: Sequence[float]
) -> list[float]:
    """Apply a long→short transition's 8:1 minimum state reduction.

    The case-2 tail overwrites only state bins ``[0:128]`` with the minimum
    of each eight-bin long raw-log group.  Bins ``[128:1024]`` retain their
    existing shared-allocation contents until a later short→long expansion.
    """
    if len(raw_long) != 1024 or len(previous_state) != 1024:
        raise ValueError("long→short floor-envelope stage state reduction expects 1024 bins")
    out = [_f32(value) for value in previous_state]
    for index in range(128):
        out[index] = _f32(min(float(value) for value in raw_long[index * 8 : index * 8 + 8]))
    return out


def shape_first_floor_envelope(
    seed: Sequence[float],
    remap: Sequence[float],
    raw: Sequence[float],
    look: WwisePsyLook,
    scratch: FloorEnvelopeScratch,
    *,
    q: float,
    side: Sequence[float] | None = None,
) -> tuple[list[float], list[float]]:
    """Run the profile-bound first regular floor-envelope stage call.

    ``state_active`` is false for this cleared first call.  The explicit
    scratch writeback is intentionally kept here rather than hidden inside
    ``shape_floor_envelope`` because later packets take a different state
    branch.
    """
    n = len(raw)
    if (
        len(seed) != n
        or len(remap) != n
        or len(look.mask_curves) < 2
        or len(look.mask_curves[1]) != n
        or len(scratch.current_curve) != n
        or len(scratch.history_curve) != n
    ):
        raise ValueError("first regular floor-envelope stage buffers must share one length")
    if side is None:
        side = [0.0] * n
    post, side_out, break_index = shape_floor_envelope(
        seed,
        scratch.current_curve,
        scratch.history_curve,
        remap,
        side,
        look.mask_curves[1],
        curve_bias=look.regular_curve_bias,
        q=q,
        curve_cap=look.regular_curve_cap,
        history_start=0,
        history_end=0,
        state_active=False,
        source_curve=raw,
    )
    if break_index is not None:
        raise AssertionError("cleared first regular floor-envelope stage call entered peak branch")
    scratch.current_curve[:] = [_f32(value) for value in raw]
    scratch.history_curve[:] = [-5.0] * n
    return post, side_out


def shape_first_long_floor_envelope(
    seed: Sequence[float],
    remap: Sequence[float],
    raw: Sequence[float],
    side: Sequence[float],
    scratch: FloorEnvelopeScratch,
    *,
    q: float = -1.0,
    look: LongFloorEnvelopeLook | None = None,
    table: WwisePsyLongTables | None = None,
    transition: int = 2,
    same_run: int = 1,
) -> tuple[list[float], list[float]]:
    """Run a fresh long profile's inactive regular floor-envelope stage invocation.

    The first six long-channel calls reach the `(inactive, update-state)`
    temporal-kernel result. Consequently their state/history hold is bypassed;
    the routine leaves history curve intact.  The normal `the normal long-frame transition` tail
    copies current raw MDCT-log values to current curve; the long→short
    `(2,0)` tail instead performs the documented 8:1 state reduction.

    ``table`` is accepted for dependency injection in regression tests.  A
    normal encoder call reads the checked static long table exactly once via
    :func:`make_long_floor_envelope_look`.
    """
    if look is None:
        if table is None:
            raise ValueError("floor-envelope stage requires look or table")
        look = make_long_floor_envelope_look(table)
    n = look.n
    if not (
        len(seed) == len(remap) == len(raw) == len(side) == n
        and len(scratch.current_curve) == n
        and len(scratch.history_curve) in (128, n)
    ):
        raise ValueError("long regular floor-envelope stage needs 1024 state bins and 128-or-1024 history bins")
    # history inside the generic floor loop.  Supply a scratch-length view to
    # its shared arithmetic helper when modelling the real 128-word history
    # allocation; preserve a full diagnostic vector unchanged when provided.
    history_curve: Sequence[float] = (
        scratch.history_curve if len(scratch.history_curve) == n else [0.0] * n
    )
    post, side_out, break_index = shape_floor_envelope(
        seed,
        scratch.current_curve,
        history_curve,
        remap,
        side,
        look.mask_curve,
        curve_bias=look.curve_bias,
        q=q,
        curve_cap=look.curve_cap,
        history_start=look.history_start,
        history_end=n,
        state_active=False,
        history_weight=0.0,
        side_gain=look.side_gain,
        source_curve=raw,
    )
    if break_index is not None:
        raise AssertionError("fresh long regular floor-envelope stage call entered peak branch")
    if (int(transition), int(same_run)) == (2, 0):
        scratch.current_curve[:] = prepare_long_to_short_history(raw, scratch.current_curve)
    else:
        scratch.current_curve[:] = [_f32(value) for value in raw]
    # The checked long path does not write the fresh history allocation.
    # Preserve its values rather than reusing the short-mode -5 sentinel.
    return post, side_out
