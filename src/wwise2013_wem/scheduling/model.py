"""Immutable values shared by frame scheduling and PCM materialization."""

from __future__ import annotations

from dataclasses import dataclass


DEFAULT_BLOCKSIZES = (256, 2048)


@dataclass(frozen=True)
class SchedulerState:
    """Persistent overlap state, expressed without PCM buffers."""

    previous: int
    current: int
    cursor: int
    filled: int
    buffer_base: int = 0
    emitted: int = 0


@dataclass(frozen=True)
class FramePlan:
    """One ready analysis frame and its exact source interval."""

    index: int
    previous: int
    current: int
    following: int
    sample_start: int
    sample_end: int
    required_filled: int
    advance: int


__all__ = ["DEFAULT_BLOCKSIZES", "FramePlan", "SchedulerState"]
