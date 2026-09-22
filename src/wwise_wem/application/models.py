"""Immutable results produced by application-level encoding."""

from __future__ import annotations

from dataclasses import dataclass
from pathlib import Path


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
    """What the encoder observed, and nothing the caller can recompose.

    ``pcm_frames`` is here because a streaming caller may never have counted
    the frames it pushed; the packet counts are here because they require
    parsing the assembled container. The container's byte length is not — it
    is ``len(result.data)`` — and neither is a label naming the selected
    profile, which is the ``WwiseProfile`` the caller itself passed.
    """

    pcm_frames: int
    channels: int
    audio_packets: int
    short_packets: int
    long_packets: int

    def __post_init__(self) -> None:
        _require_int(self.pcm_frames, "pcm_frames")
        _require_int(self.channels, "channels", positive=True)
        _require_int(self.audio_packets, "audio_packets")
        _require_int(self.short_packets, "short_packets")
        _require_int(self.long_packets, "long_packets")
        if self.short_packets + self.long_packets != self.audio_packets:
            raise ValueError("short/long packet counts must equal audio_packets")

    def to_dict(self) -> dict[str, int]:
        """Return the stable, JSON-ready statistics fields."""
        return {
            "pcm_frames": self.pcm_frames,
            "channels": self.channels,
            "audio_packets": self.audio_packets,
            "short_packets": self.short_packets,
            "long_packets": self.long_packets,
        }


@dataclass(frozen=True)
class EncodeResult:
    """The completed container and the statistics observed while producing it."""

    data: bytes
    stats: EncodeStats

    def __post_init__(self) -> None:
        data = bytes(self.data)
        if not isinstance(self.stats, EncodeStats):
            raise TypeError("stats must be EncodeStats")
        object.__setattr__(self, "data", data)

    def __len__(self) -> int:
        """Byte length of the completed container."""
        return len(self.data)

    def write_to(self, path: str | Path) -> None:
        """Write the completed container to ``path``.

        The counterpart of reading ``data`` and calling ``write_bytes``, so a
        caller does not have to reach into the result to persist it.
        """
        Path(path).write_bytes(self.data)
