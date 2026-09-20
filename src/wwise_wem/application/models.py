"""Immutable results produced by application-level encoding."""

from __future__ import annotations

import hashlib
from dataclasses import dataclass


def _require_int(value: int, label: str, *, positive: bool = False) -> int:
    if not isinstance(value, int) or isinstance(value, bool):
        raise TypeError(f"{label} must be an integer")
    if positive and value <= 0:
        raise ValueError(f"{label} must be positive")
    if not positive and value < 0:
        raise ValueError(f"{label} must be non-negative")
    return value


@dataclass(frozen=True)
class EncodeStats:
    pcm_frames: int
    channels: int
    audio_packets: int
    short_packets: int
    long_packets: int
    bytes: int
    metadata_source: str

    def __post_init__(self) -> None:
        _require_int(self.pcm_frames, "pcm_frames")
        _require_int(self.channels, "channels", positive=True)
        _require_int(self.audio_packets, "audio_packets")
        _require_int(self.short_packets, "short_packets")
        _require_int(self.long_packets, "long_packets")
        _require_int(self.bytes, "bytes")
        if self.short_packets + self.long_packets != self.audio_packets:
            raise ValueError("short/long packet counts must equal audio_packets")
        if not isinstance(self.metadata_source, str) or not self.metadata_source:
            raise ValueError("metadata_source must be a non-empty string")

    def to_dict(self) -> dict[str, int | str]:
        """Return the stable, JSON-ready statistics fields."""
        return {
            "pcm_frames": self.pcm_frames,
            "channels": self.channels,
            "audio_packets": self.audio_packets,
            "short_packets": self.short_packets,
            "long_packets": self.long_packets,
            "bytes": self.bytes,
            "metadata_source": self.metadata_source,
        }


@dataclass(frozen=True)
class EncodeResult:
    data: bytes
    stats: EncodeStats

    def __post_init__(self) -> None:
        data = bytes(self.data)
        if not isinstance(self.stats, EncodeStats):
            raise TypeError("stats must be EncodeStats")
        if len(data) != self.stats.bytes:
            raise ValueError("encoded byte count differs from stats")
        object.__setattr__(self, "data", data)

    @property
    def sha256(self) -> str:
        return hashlib.sha256(self.data).hexdigest()
