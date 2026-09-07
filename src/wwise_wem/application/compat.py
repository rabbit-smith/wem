#!/usr/bin/env python3
"""Compatibility helpers for the historical PCM input domain."""

from __future__ import annotations

import struct
import wave
from pathlib import Path


def read_pcm16_wav(path: Path) -> tuple[int, int, list[list[float]]]:
    """Return the historical ``(rate, frames, channel_rows)`` tuple.

    Unlike the typed :class:`PcmBuffer` adapter, this compatibility function
    deliberately preserves empty WAV files as one empty row per channel.
    """
    with wave.open(str(path), "rb") as source:
        channels = source.getnchannels()
        sample_rate = source.getframerate()
        frames = source.getnframes()
        if source.getsampwidth() != 2 or source.getcomptype() != "NONE":
            raise ValueError("encoder input must be uncompressed signed-16 PCM WAV")
        values = struct.unpack(
            f"<{frames * channels}h", source.readframes(frames)
        )
    pcm = [
        [values[frame * channels + channel] / 32768.0 for frame in range(frames)]
        for channel in range(channels)
    ]
    return sample_rate, frames, pcm
