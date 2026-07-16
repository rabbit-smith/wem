"""Typed WAV input adapter for the public encoder facade."""

from __future__ import annotations

import struct
import wave
from pathlib import Path

from ..model import PcmBuffer


def read_pcm16(path: Path) -> PcmBuffer:
    """Read an uncompressed signed-16 PCM WAV into channel-major floats.

    Samples retain the legacy encoder's exact ``value / 32768.0``
    normalization.  Empty WAV files are rejected at this typed boundary
    because :class:`PcmBuffer` requires at least one frame.
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

    if frames == 0:
        raise ValueError("encoder input WAV must contain at least one frame")
    pcm = tuple(
        tuple(
            values[frame * channels + channel] / 32768.0
            for frame in range(frames)
        )
        for channel in range(channels)
    )
    return PcmBuffer(sample_rate=sample_rate, channels=pcm)


def read_wav_geometry(path: Path) -> tuple[int, int]:
    """Return ``(channel_count, sample_rate)`` from a WAV header."""
    with wave.open(str(path), "rb") as source:
        return source.getnchannels(), source.getframerate()


__all__ = ["read_pcm16", "read_wav_geometry"]
