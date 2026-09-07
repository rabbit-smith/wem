#!/usr/bin/env python3
"""Transient-driven short/long block mode selection.

The lower-level detector supplies one combined integer flag word per
64-sample quantum.  This module owns queue timing, look-ahead scanning, and
the psychoacoustic profile variant derived for each scheduled frame.
"""
from __future__ import annotations

from dataclasses import dataclass
from typing import Iterable, Sequence


@dataclass
class ModeSelector:
    """Persistent transient queue and short/long selection cursors."""

    hop: int
    capacity: int
    cooldown: int
    generated: int
    selected: int
    scan_cursor: int
    queue: list[int]

    def __post_init__(self) -> None:
        if self.hop <= 0 or self.capacity <= 0:
            raise ValueError("selector hop/capacity must be positive")
        if len(self.queue) < self.capacity:
            self.queue.extend([0] * (self.capacity - len(self.queue)))

    def _check_slot(self, index: int) -> None:
        if index < 0 or index >= self.capacity:
            raise ValueError(f"selector queue slot {index} exceeds capacity")

    def begin_quantum(self) -> int:
        """Advance and return the shared detector history window.

        Every channel in one quantum receives the same value, and the window
        saturates after 24 consecutive non-resetting quanta.
        """
        self.cooldown = min(self.cooldown + 1, 24)
        return self.cooldown

    def finish_quantum(self, index: int, flags: int) -> None:
        """Store one already-computed multichannel transient flag word."""
        self._check_slot(index + 2)
        self.queue[index + 2] = 0
        if flags & 1:
            self._check_slot(index + 1)
            self.queue[index] = 1
            self.queue[index + 1] = 1
        if flags & 2:
            self.queue[index] = 1
            if index:
                self.queue[index - 1] = 1
        if flags & 4:
            self.cooldown = -1

    def apply_quantum(self, index: int, flags: int) -> None:
        """Advance history and store one precomputed transient result."""
        self.begin_quantum()
        self.finish_quantum(index, flags)

    def generate(
        self, *, filled: int, flags_by_quantum: Iterable[int]
    ) -> int:
        """Fill queue entries through the current buffered-PCM target.

        The supplied iterator contains one already-combined integer flag word
        for each quantum, rather than one value per channel.
        """
        target = filled // self.hop - 4
        target = max(0, target)
        # Grow before writing the two-slot look-ahead and retain six spare
        # entries for the next generation step.
        required_capacity = target + 6
        if required_capacity > self.capacity:
            self.queue.extend([0] * (required_capacity - self.capacity))
            self.capacity = required_capacity
        start = max(0, self.generated // self.hop)
        if target < start:
            return 0
        flags = iter(flags_by_quantum)
        written = 0
        for index in range(start, target):
            try:
                value = next(flags)
            except StopIteration as error:
                raise ValueError("mode generation lacks transient flag rows") from error
            self.apply_quantum(index, int(value))
            written += 1
        try:
            next(flags)
        except StopIteration:
            pass
        else:
            raise ValueError("mode generation received surplus transient flag rows")
        self.generated = target * self.hop
        return written

    def scan(
        self,
        *,
        center: int,
        current_mode: int,
        blocksizes: Sequence[int],
    ) -> int:
        """Return the selector's native ``-1/0/1`` look-ahead decision."""
        if len(blocksizes) != 2 or current_mode not in (0, 1):
            raise ValueError("selector expects Wwise short/long block sizes")
        boundary = (
            int(blocksizes[current_mode]) // 4
            + center
            + int(blocksizes[1]) // 2
            + int(blocksizes[0]) // 4
        )
        limit = self.generated - self.hop
        position = self.scan_cursor
        if position >= limit:
            return -1
        while True:
            if position >= boundary:
                return 1
            self.scan_cursor = position
            slot = position // self.hop
            self._check_slot(slot)
            if self.queue[slot] and position > center:
                self.selected = position
                return 0
            position += self.hop
            if position >= limit:
                return -1

    def has_transient_in_window(
        self,
        *,
        center: int,
        previous_mode: int,
        current_mode: int,
        following_mode: int,
        blocksizes: Sequence[int],
    ) -> bool:
        """Return whether the frame's overlap window contains a transient."""
        if len(blocksizes) != 2 or any(
            mode not in (0, 1)
            for mode in (previous_mode, current_mode, following_mode)
        ):
            raise ValueError("transition look expects short/long mode bits")
        quarter = int(blocksizes[current_mode]) // 4
        lower = int(center) - quarter
        upper = int(center) + quarter
        if current_mode:
            lower -= int(blocksizes[previous_mode]) // 4
            upper += int(blocksizes[following_mode]) // 4
        else:
            short_quarter = int(blocksizes[0]) // 4
            lower -= short_quarter
            upper += short_quarter

        # Check the exact selected position before scanning queue slots.
        # Profile positions are non-negative; int() deliberately retains
        # truncation toward zero for a generic early negative boundary.
        if lower <= self.selected < upper:
            return True
        first = int(lower / self.hop)
        limit = int(upper / self.hop)
        for slot in range(first, limit):
            if 0 <= slot < self.capacity and self.queue[slot]:
                return True
        return False

    def transition_code(
        self,
        *,
        center: int,
        previous_mode: int,
        current_mode: int,
        following_mode: int,
        blocksizes: Sequence[int],
    ) -> int:
        """Return the psychoacoustic variant code for one scheduled frame.

        The low bit selects the profile variant and the high bit represents
        the current block mode.  Long blocks combine adjacent long modes;
        short blocks invert the transient-window result.
        """
        if current_mode:
            variant = int(bool(previous_mode) and bool(following_mode))
        else:
            variant = int(
                not self.has_transient_in_window(
                    center=center,
                    previous_mode=previous_mode,
                    current_mode=current_mode,
                    following_mode=following_mode,
                    blocksizes=blocksizes,
                )
            )
        return 2 * int(current_mode) + variant
