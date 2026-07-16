"""Pure readiness and overlap planning for Wwise analysis frames."""

from __future__ import annotations

from dataclasses import replace
from typing import Sequence

from .model import DEFAULT_BLOCKSIZES, FramePlan, SchedulerState


def _validate_modes(blocksizes: Sequence[int], *modes: int) -> None:
    if len(blocksizes) != 2:
        raise ValueError("the Wwise scheduler has exactly short/long block sizes")
    if any(size < 2 or size & 1 for size in blocksizes):
        raise ValueError("block sizes must be positive even values")
    if any(mode not in (0, 1) for mode in modes):
        raise ValueError("mode must be 0 (short) or 1 (long)")


def initial_state(
    blocksizes: Sequence[int] = DEFAULT_BLOCKSIZES,
) -> SchedulerState:
    """Return the overlap state before the first PCM append."""
    _validate_modes(blocksizes, 0, 0)
    long_half = int(blocksizes[1]) // 2
    return SchedulerState(0, 0, long_half, long_half)


def required_samples(
    state: SchedulerState,
    following: int,
    blocksizes: Sequence[int] = DEFAULT_BLOCKSIZES,
) -> int:
    """Return the exact readiness threshold for a chosen following mode."""
    _validate_modes(blocksizes, state.previous, state.current, following)
    current_size = int(blocksizes[state.current])
    following_size = int(blocksizes[following])
    return (
        state.cursor
        + following_size // 4
        + current_size // 4
        + following_size // 2
    )


def emit_block(
    state: SchedulerState,
    following: int,
    blocksizes: Sequence[int] = DEFAULT_BLOCKSIZES,
) -> tuple[FramePlan, SchedulerState]:
    """Emit one frame plan and return the post-slide state."""
    _validate_modes(blocksizes, state.previous, state.current, following)
    required = required_samples(state, following, blocksizes)
    if state.filled < required:
        raise ValueError(
            f"need {required} buffered samples for mode "
            f"{state.current}->{following}; have {state.filled}"
        )

    current_size = int(blocksizes[state.current])
    following_size = int(blocksizes[following])
    long_half = int(blocksizes[1]) // 2
    sample_start = state.buffer_base + state.cursor - current_size // 2
    advance = (
        state.cursor + following_size // 4 + current_size // 4 - long_half
    )
    if advance <= 0:
        raise AssertionError("scheduler ring advance must be positive")

    plan = FramePlan(
        index=state.emitted,
        previous=state.previous,
        current=state.current,
        following=following,
        sample_start=sample_start,
        sample_end=sample_start + current_size,
        required_filled=required,
        advance=advance,
    )
    next_state = SchedulerState(
        previous=state.current,
        current=following,
        cursor=long_half,
        filled=state.filled - advance,
        buffer_base=state.buffer_base + advance,
        emitted=state.emitted + 1,
    )
    return plan, next_state


def append_samples(state: SchedulerState, count: int) -> SchedulerState:
    """Return the same state after the feeder appends ``count`` samples."""
    if count < 0:
        raise ValueError("append count must be non-negative")
    return replace(state, filled=state.filled + count)


def plan_mode_sequence(
    modes: Sequence[int],
    *,
    blocksizes: Sequence[int] = DEFAULT_BLOCKSIZES,
    terminal_following: int = 1,
) -> tuple[FramePlan, ...]:
    """Turn a complete mode sequence into its immutable frame plans.

    ``FramePlan`` intervals use the scheduler timeline, whose zero precedes
    PCM sample zero by one long half-block.  PCM materialization applies that
    single origin offset; all overlap advances and transition modes remain
    owned here.
    """
    if not modes:
        return ()
    _validate_modes(blocksizes, *modes, terminal_following)

    long_half = int(blocksizes[1]) // 2
    state = SchedulerState(
        previous=0,
        current=int(modes[0]),
        cursor=long_half,
        filled=long_half,
    )
    plans: list[FramePlan] = []
    for index, current in enumerate(modes):
        if state.current != int(current):
            raise AssertionError("mode sequence and scheduler state diverged")
        following = (
            int(modes[index + 1])
            if index + 1 < len(modes)
            else int(terminal_following)
        )
        required = required_samples(state, following, blocksizes)
        state = append_samples(state, max(0, required - state.filled))
        plan, state = emit_block(state, following, blocksizes)
        plans.append(plan)
    return tuple(plans)


__all__ = [
    "append_samples",
    "emit_block",
    "initial_state",
    "plan_mode_sequence",
    "required_samples",
]
