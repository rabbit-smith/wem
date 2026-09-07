"""Immutable results produced by application-level encoding."""

from __future__ import annotations

import hashlib
from dataclasses import dataclass
from typing import Mapping, Sequence


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
    engine: str = "python"

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
        if not isinstance(self.engine, str) or not self.engine:
            raise ValueError("engine must be a non-empty string")

    @classmethod
    def from_legacy_dict(cls, values: Mapping[str, int | str]) -> "EncodeStats":
        return cls(
            pcm_frames=int(values["pcm_frames"]),
            channels=int(values["channels"]),
            audio_packets=int(values["audio_packets"]),
            short_packets=int(values["short_packets"]),
            long_packets=int(values["long_packets"]),
            bytes=int(values["bytes"]),
            metadata_source=str(values["metadata_source"]),
            engine=str(values.get("engine", "python")),
        )

    def to_legacy_dict(self) -> dict[str, int | str]:
        return {
            "pcm_frames": self.pcm_frames,
            "channels": self.channels,
            "audio_packets": self.audio_packets,
            "short_packets": self.short_packets,
            "long_packets": self.long_packets,
            "bytes": self.bytes,
            "metadata_source": self.metadata_source,
            "engine": self.engine,
        }

    from_legacy = from_legacy_dict
    to_legacy = to_legacy_dict


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

    @classmethod
    def from_legacy_tuple(
        cls, value: Sequence[bytes | Mapping[str, int | str]]
    ) -> "EncodeResult":
        if len(value) != 2:
            raise ValueError("legacy encode result must contain data and stats")
        data, stats = value
        if not isinstance(data, (bytes, bytearray)) or not isinstance(stats, Mapping):
            raise TypeError("legacy encode result must be (bytes, stats mapping)")
        return cls(bytes(data), EncodeStats.from_legacy_dict(stats))

    def to_legacy_tuple(self) -> tuple[bytes, dict[str, int | str]]:
        return self.data, self.stats.to_legacy_dict()

    from_legacy = from_legacy_tuple
    to_legacy = to_legacy_tuple
