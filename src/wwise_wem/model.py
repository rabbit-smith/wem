"""Immutable value models for encoder inputs and profile metadata.

Also holds the package's public error type, which crosses the same boundary
as these values: a kernel rejection reaches the caller as a value carrying
the kernel's stable error code, not as a rewritten message.
"""

from __future__ import annotations

from dataclasses import dataclass
from typing import Literal, Mapping


def _require_int(value: int, label: str, *, positive: bool = False) -> int:
    if not isinstance(value, int) or isinstance(value, bool):
        raise TypeError(f"{label} must be an integer")
    if positive and value <= 0:
        raise ValueError(f"{label} must be positive")
    if not positive and value < 0:
        raise ValueError(f"{label} must be non-negative")
    return value


@dataclass(frozen=True)
class PcmBuffer:
    sample_rate: int
    channels: tuple[tuple[float, ...], ...]

    def __post_init__(self) -> None:
        _require_int(self.sample_rate, "sample_rate", positive=True)
        normalized = tuple(
            tuple(float(sample) for sample in row) for row in self.channels
        )
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


RawPcmFormat = Literal["s16le", "s24le", "f32le"]


@dataclass(frozen=True)
class RawPcm:
    """Unframed PCM bytes with the geometry needed to decode them safely."""

    data: bytes
    sample_rate: int
    channels: int
    sample_format: RawPcmFormat

    def __post_init__(self) -> None:
        if not isinstance(self.data, (bytes, bytearray, memoryview)):
            raise TypeError("data must be bytes-like")
        _require_int(self.sample_rate, "sample_rate", positive=True)
        _require_int(self.channels, "channels", positive=True)
        if self.sample_format not in ("s16le", "s24le", "f32le"):
            raise ValueError("sample_format must be 's16le', 's24le', or 'f32le'")
        object.__setattr__(self, "data", bytes(self.data))


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


class WwiseWemError(ValueError):
    """One kernel rejection, carrying the kernel's stable error code.

    The kernel's error classes are stable across the cross-language shells,
    and this type is where they reach a Python caller: ``code`` is the
    class (``PROFILE_NOT_FOUND``, ``GEOMETRY_MISMATCH``, ``INPUT_TOO_SHORT``,
    ``FORMAT_UNSUPPORTED``, ``STATE_ERROR``, ``INTERNAL``) and ``message``
    is the kernel's own diagnostic text, which ``str(error)`` returns
    unchanged. Nothing here reinterprets or rewrites the kernel's surface.

    Subclassing :class:`ValueError` keeps ``except ValueError`` working for
    callers written against the message-only error, and the native
    ``WemEncoderError`` that caused it stays reachable as ``__cause__``.
    """

    def __init__(self, code: str, message: str) -> None:
        super().__init__(code, message)
        self.code = code
        self.message = message

    def __str__(self) -> str:
        return self.message
