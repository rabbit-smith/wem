#!/usr/bin/env python3
"""Temporal weighting and history relaxation for psychoacoustic analysis."""

from __future__ import annotations

from dataclasses import dataclass
from typing import Sequence

from ..._f32 import _f32


# Sliding relaxation widths for the 128-bin short-block history recurrence.
SHORT_HISTORY_RELAXATION_WIDTHS = (
    (1,) * 8
    + tuple(width for width in range(2, 25) for _ in range(4))
    + (25,) * 3
    + tuple(range(24, 0, -1))
    + (1,)
)
if len(SHORT_HISTORY_RELAXATION_WIDTHS) != 128:
    raise AssertionError("short history width table has the wrong length")


@dataclass(frozen=True)
class TemporalKernelInputs:
    bins: int
    run_count: int
    tail_count: int
    high_rate: bool
    curve_bias: float
    transition_code: int
    previous_transition: int
    hold_update: int
    analysis_mode: int


@dataclass(frozen=True)
class TemporalKernelResult:
    active: int = 0
    update_state: int = 0
    lower_weight: float = 0.0
    upper_weight: float = 0.0
    state_bias: float = 0.0
    chase_seed: float = 0.0

    def words(self) -> tuple[int | float, ...]:
        return (
            self.active,
            self.update_state,
            self.lower_weight,
            self.upper_weight,
            self.state_bias,
            self.chase_seed,
        )


def compute_temporal_kernel(inputs: TemporalKernelInputs) -> TemporalKernelResult:
    """Compute temporal weights for the active block geometry and transition."""
    if not inputs.high_rate:
        return TemporalKernelResult()

    update = int(not inputs.hold_update or inputs.analysis_mode == 2)
    if not update and inputs.analysis_mode == 0:
        return TemporalKernelResult()
    if inputs.transition_code:
        return TemporalKernelResult(update_state=update)

    if inputs.bins == 128:
        multiplier = 3 if inputs.curve_bias >= 3.0 else 2
        if inputs.previous_transition:
            result = TemporalKernelResult(1, update, 0.7, 0.0, 0.0, 8.0)
        elif inputs.run_count >= 8:
            result = TemporalKernelResult(
                1, update, 0.3, 0.0, 25.0, 0.0
            )
        else:
            result = TemporalKernelResult(
                1,
                update,
                0.7 - (inputs.run_count - 1) / 17.0,
                0.0,
                float(inputs.run_count * multiplier),
                float(8 - inputs.run_count),
            )
        divisor = 8.0
    elif inputs.bins == 256:
        if inputs.previous_transition:
            result = TemporalKernelResult(1, update, 0.6, 0.0, 12.0, 8.0)
        elif inputs.run_count >= 4:
            result = TemporalKernelResult(1, update, 0.2, 0.0, 30.0, 0.0)
        else:
            result = TemporalKernelResult(
                1,
                update,
                0.4 - (inputs.run_count - 1) / 11.0,
                0.0,
                float(2 * (3 * inputs.run_count + 6)),
                float(2 * (4 - inputs.run_count)),
            )
        divisor = 16.0
    else:
        # Long blocks do not run the short peak-continuation branch.
        return TemporalKernelResult(update_state=update)

    lower = result.lower_weight
    if inputs.tail_count:
        lower *= inputs.tail_count / divisor
    if inputs.hold_update and inputs.analysis_mode == 0:
        lower *= 0.2
    return TemporalKernelResult(
        result.active,
        result.update_state,
        lower,
        result.upper_weight,
        result.state_bias,
        result.chase_seed,
    )


def rebase_history(
    inputs: TemporalKernelInputs,
    result: TemporalKernelResult,
    state: Sequence[float],
    history: Sequence[float],
) -> list[float]:
    """Apply the history rebase that precedes width relaxation.

    The caller can then apply :func:`relax_history` with the static width table
    when an n=128/256 path needs it.  n=1024, all transitions, and inactive
    tuples leave history unchanged.
    """
    if len(state) != inputs.bins or len(history) != inputs.bins:
        raise ValueError("state/history length must equal n")
    if not result.active or not result.update_state:
        return [float(value) for value in history]
    if inputs.bins == 128:
        delta = 5.0
    elif inputs.bins == 256:
        delta = 10.0
    else:
        return [float(value) for value in history]
    source = state if inputs.previous_transition else history
    return [_f32(float(value) - delta) for value in source]


def relax_history(
    history: Sequence[float],
    raw: Sequence[float],
    widths: Sequence[int],
    *,
    delta: float,
) -> list[float]:
    """Apply sliding width-table relaxation to one history curve."""
    if not (len(history) == len(raw) == len(widths)):
        raise ValueError("history/raw/widths must have equal length")
    out = [_f32(value) for value in history]
    n = len(out)
    for k, width in enumerate(widths):
        if width <= 1:
            continue
        width = int(width)
        for j in range(1, width):
            index = k + j
            if index >= n:
                break
            if float(raw[k]) - j * 75.0 / width > out[index]:
                denominator = int(widths[index])
                if denominator <= 0:
                    raise ValueError("history width table contains a nonpositive value")
                # Each relaxation is stored as float32 before later comparisons.
                out[index] = _f32(out[index] + delta / denominator)
    return out


def relax_short_history(
    history: Sequence[float], raw: Sequence[float]
) -> list[float]:
    """Apply the complete 128-bin short-history relaxation table."""
    return relax_history(
        history, raw, SHORT_HISTORY_RELAXATION_WIDTHS, delta=5.0
    )


def update_short_history(
    inputs: TemporalKernelInputs,
    state: Sequence[float],
    history: Sequence[float],
    raw: Sequence[float],
    seed: Sequence[float],
    remap: Sequence[float],
    mask_curve: Sequence[float],
    *,
    curve_cap: float,
    q: float = 0.0,
    candidate_bound: int = 0,
) -> list[float]:
    """Update short-block temporal history from local psychoacoustic surfaces."""
    if inputs.bins != 128:
        raise ValueError("short temporal history expects 128 bins")
    arrays = (state, history, raw, seed, remap, mask_curve)
    if any(len(values) != 128 for values in arrays):
        raise ValueError("short temporal history curves must have 128 bins")
    result = compute_temporal_kernel(inputs)
    baseline = rebase_history(inputs, result, state, history)
    if result.active and result.update_state:
        baseline = relax_short_history(baseline, raw)
    else:
        baseline = [_f32(value) for value in baseline]

    if not result.active:
        return baseline
    out = baseline[:]
    subtract = 0.0
    if float(q) >= 0.0 and inputs.curve_bias >= 25.0:
        subtract = float(q) * (inputs.curve_bias - 25.0)
    for index in range(128):
        cap = min(
            _f32(float(mask_curve[index]) + float(remap[index])),
            float(curve_cap),
        )
        candidate = _f32(float(seed[index]) + inputs.curve_bias)
        if index <= candidate_bound:
            candidate = _f32(candidate - subtract)
        if (
            candidate < cap
            and float(state[index]) < cap
            and float(out[index]) + result.state_bias < float(raw[index])
        ):
            if result.update_state:
                out[index] = _f32(raw[index])
    return out
