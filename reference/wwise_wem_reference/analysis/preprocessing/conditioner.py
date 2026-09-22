"""Stateful profile-selected PCM conditioning."""

from __future__ import annotations

from typing import Sequence

from ..._f32 import _f32
from ..config import InputConditionerConfig


class InputConditioner:
    """Apply the paired build's DC filter and signed-16 storage boundary."""

    def __init__(self, channels: int, config: InputConditionerConfig) -> None:
        if channels <= 0:
            raise ValueError("input conditioner needs at least one channel")
        self._channels = int(channels)
        self._coefficient = config.dc_filter_coefficient
        self.reset()

    def reset(self) -> None:
        self._previous_input = [0.0] * self._channels
        self._previous_output = [0.0] * self._channels

    def process(self, pcm_by_channel: Sequence[Sequence[float]]) -> tuple[tuple[float, ...], ...]:
        if len(pcm_by_channel) != self._channels:
            raise ValueError("input conditioner channel count differs")
        frames = len(pcm_by_channel[0]) if pcm_by_channel else 0
        if any(len(row) != frames for row in pcm_by_channel):
            raise ValueError("input conditioner PCM channels must have equal lengths")

        conditioned: list[tuple[float, ...]] = []
        for channel, row in enumerate(pcm_by_channel):
            previous_input = self._previous_input[channel]
            previous_output = self._previous_output[channel]
            output: list[float] = []
            for sample in row:
                current = _f32(sample)
                filtered = _f32((current - previous_input) + self._coefficient * previous_output)
                stored = round(_f32(filtered * 32767.0))
                if stored > 32767:
                    stored = 32767
                elif stored < -32768:
                    stored = -32768
                output.append(stored / 32768.0)
                previous_input = current
                previous_output = filtered
            self._previous_input[channel] = previous_input
            self._previous_output[channel] = previous_output
            conditioned.append(tuple(output))
        return tuple(conditioned)
