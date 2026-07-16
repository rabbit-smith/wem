"""Immutable value models for the encoder's public adapter boundary."""

from __future__ import annotations

import hashlib
from dataclasses import dataclass, field
from types import MappingProxyType
from typing import Any, Mapping


def _require_int(value: int, label: str, *, positive: bool = False) -> int:
    if not isinstance(value, int) or isinstance(value, bool):
        raise TypeError(f"{label} must be an integer")
    if positive and value <= 0:
        raise ValueError(f"{label} must be positive")
    if not positive and value < 0:
        raise ValueError(f"{label} must be non-negative")
    return value


def _freeze(value: Any) -> Any:
    """Copy common containers into recursively read-only equivalents."""
    if isinstance(value, Mapping):
        return MappingProxyType({key: _freeze(item) for key, item in value.items()})
    if isinstance(value, (list, tuple)):
        return tuple(_freeze(item) for item in value)
    if isinstance(value, (set, frozenset)):
        return frozenset(_freeze(item) for item in value)
    if isinstance(value, bytearray):
        return bytes(value)
    return value


@dataclass(frozen=True)
class PcmBuffer:
    sample_rate: int
    channels: tuple[tuple[float, ...], ...]

    def __post_init__(self) -> None:
        _require_int(self.sample_rate, "sample_rate", positive=True)
        normalized = tuple(tuple(float(sample) for sample in row) for row in self.channels)
        if not normalized:
            raise ValueError("PCM buffer needs at least one channel")
        if not normalized[0]:
            raise ValueError("PCM buffer needs at least one frame")
        frame_count = len(normalized[0])
        if any(len(row) != frame_count for row in normalized):
            raise ValueError("PCM channels must have equal frame counts")
        object.__setattr__(self, "channels", normalized)

    @property
    def channel_count(self) -> int:
        return len(self.channels)

    @property
    def frame_count(self) -> int:
        return len(self.channels[0])


@dataclass(frozen=True)
class ContainerMetadata:
    """Typed representation of the fixed 66-byte Wwise Vorbis fmt fields."""

    wFormatTag: int
    nChannels: int
    nSamplesPerSec: int
    nAvgBytesPerSec: int
    nBlockAlign: int
    wBitsPerSample: int
    cbSize: int
    wReserved0: int
    dwChannelMask: int
    dwTotalPCMFrames: int
    dwFirstAudioPacketOffset: int
    dwDataPayloadSize: int
    dwUnknown_0x24: int
    dwSeekTableSize: int
    dwVorbisDataOffset: int
    uMaxPacketSize: int
    uUnknown_0x32: int
    dwUnknown_0x34: int
    dwUnknown_0x38: int
    dwUnknown_0x3C: int
    uBlocksize0Pow: int
    uBlocksize1Pow: int

    def __post_init__(self) -> None:
        _require_int(self.nChannels, "nChannels", positive=True)
        _require_int(self.nSamplesPerSec, "nSamplesPerSec", positive=True)
        for name, value in self.to_fmt_dict(frame_count=self.dwTotalPCMFrames).items():
            _require_int(value, name)

    @classmethod
    def from_fmt_dict(cls, values: Mapping[str, int]) -> "ContainerMetadata":
        return cls(**{name: values[name] for name in cls.__dataclass_fields__})

    def to_fmt_dict(self, *, frame_count: int = 0) -> dict[str, int]:
        _require_int(frame_count, "frame_count")
        return {
            "wFormatTag": self.wFormatTag,
            "nChannels": self.nChannels,
            "nSamplesPerSec": self.nSamplesPerSec,
            "nAvgBytesPerSec": self.nAvgBytesPerSec,
            "nBlockAlign": self.nBlockAlign,
            "wBitsPerSample": self.wBitsPerSample,
            "cbSize": self.cbSize,
            "wReserved0": self.wReserved0,
            "dwChannelMask": self.dwChannelMask,
            "dwTotalPCMFrames": frame_count,
            "dwFirstAudioPacketOffset": self.dwFirstAudioPacketOffset,
            "dwDataPayloadSize": self.dwDataPayloadSize,
            "dwUnknown_0x24": self.dwUnknown_0x24,
            "dwSeekTableSize": self.dwSeekTableSize,
            "dwVorbisDataOffset": self.dwVorbisDataOffset,
            "uMaxPacketSize": self.uMaxPacketSize,
            "uUnknown_0x32": self.uUnknown_0x32,
            "dwUnknown_0x34": self.dwUnknown_0x34,
            "dwUnknown_0x38": self.dwUnknown_0x38,
            "dwUnknown_0x3C": self.dwUnknown_0x3C,
            "uBlocksize0Pow": self.uBlocksize0Pow,
            "uBlocksize1Pow": self.uBlocksize1Pow,
        }


@dataclass(frozen=True)
class SetupConfig:
    raw_packet: bytes
    channels: int
    book_ids: tuple[int, ...]
    parsed: Mapping[str, Any] = field(default_factory=dict, repr=False, compare=False)

    def __post_init__(self) -> None:
        _require_int(self.channels, "channels", positive=True)
        packet = bytes(self.raw_packet)
        if not packet:
            raise ValueError("setup packet must not be empty")
        ids = tuple(_require_int(value, "book_id") for value in self.book_ids)
        if not ids:
            raise ValueError("setup config needs at least one book id")
        if not isinstance(self.parsed, Mapping):
            raise TypeError("parsed setup must be a mapping")
        object.__setattr__(self, "raw_packet", packet)
        object.__setattr__(self, "book_ids", ids)
        object.__setattr__(self, "parsed", _freeze(dict(self.parsed)))


@dataclass(frozen=True)
class PacketResult:
    """One encoded packet and the window-mode triple that produced it."""

    packet: bytes
    mode: int
    previous_mode: int
    following_mode: int

    def __post_init__(self) -> None:
        packet = bytes(self.packet)
        if not packet:
            raise ValueError("encoded packet must not be empty")
        for name in ("mode", "previous_mode", "following_mode"):
            value = getattr(self, name)
            if not isinstance(value, int) or isinstance(value, bool) or value not in (0, 1):
                raise ValueError(f"{name} must be short/long mode 0 or 1")
        object.__setattr__(self, "packet", packet)

    @property
    def size(self) -> int:
        return len(self.packet)

    @property
    def sha256(self) -> str:
        return hashlib.sha256(self.packet).hexdigest()
