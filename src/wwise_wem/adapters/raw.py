"""Raw PCM input adapter: unframed sample bytes plus explicit geometry.

The caller states the geometry (sample rate, channel count, bit depth);
this module validates it and returns a :class:`PcmBuffer` in the signed-16
encoder domain.  24-bit samples are two's-complement little-endian;
32-bit payloads are IEEE-754 float32.  Conversion uses only the pure
helpers from :mod:`wwise_wem.adapters.sample_conversion`, so converted
inputs land exactly on the in-domain floats the encoder expects.

Validation mirrors the existing :class:`PcmBuffer` rejection semantics:
geometry arguments are explicit ``TypeError`` / ``ValueError`` at the
point of validation, byte-size mismatches and empty payloads are
``ValueError``, and the final :class:`PcmBuffer` construction re-checks
channel lengths.
"""

from __future__ import annotations

import struct
from array import array
from sys import byteorder

from ..model import PcmBuffer
from .sample_conversion import (
    float_to_int16,
    int16_to_domain_value,
    sample24_to_int16,
    unpack_sample24,
)

_SUPPORTED_BITS = (16, 24, 32)


def _require_geometry_int(value: int, label: str, *, positive: bool) -> int:
    if not isinstance(value, int) or isinstance(value, bool):
        raise TypeError(f"{label} must be an integer")
    if positive and value <= 0:
        raise ValueError(f"{label} must be positive")
    return value


def read_raw_pcm(
    data: bytes,
    *,
    sample_rate: int,
    channels: int,
    bits_per_sample: int,
) -> PcmBuffer:
    """Build a signed-16-domain :class:`PcmBuffer` from raw PCM bytes.

    ``data`` must be a bytes-like payload whose length is an exact
    multiple of ``channels * bytes_per_sample`` and holds at least one
    frame.  Supported bit depths: 16 (signed integer), 24 (signed
    integer), 32 (IEEE-754 float32).
    """
    payload = _normalize_pcm16_bytes(
        data,
        sample_rate=sample_rate,
        channels=channels,
        bits_per_sample=bits_per_sample,
    )
    return _pcm16_bytes_to_buffer(payload, sample_rate=sample_rate, channels=channels)


def _normalize_pcm16_bytes(
    data: bytes | bytearray | memoryview,
    *,
    sample_rate: int,
    channels: int,
    bits_per_sample: int,
) -> bytes:
    """Validate raw PCM and return interleaved signed-16 little-endian bytes."""
    if not isinstance(data, (bytes, bytearray, memoryview)):
        raise TypeError("data must be bytes-like")
    payload = bytes(data)
    sample_rate = _require_geometry_int(sample_rate, "sample_rate", positive=True)
    channels = _require_geometry_int(channels, "channels", positive=True)
    if not isinstance(bits_per_sample, int) or isinstance(bits_per_sample, bool):
        raise TypeError("bits_per_sample must be an integer")
    if bits_per_sample not in _SUPPORTED_BITS:
        raise ValueError("bits_per_sample must be 16, 24, or 32")
    bytes_per_sample = bits_per_sample // 8
    bytes_per_frame = channels * bytes_per_sample
    if len(payload) % bytes_per_frame:
        raise ValueError(
            f"raw PCM size {len(payload)} is not a multiple of the frame size "
            f"({channels} channels x {bytes_per_sample} bytes)"
        )
    frames = len(payload) // bytes_per_frame
    if frames == 0:
        raise ValueError("raw PCM must contain at least one frame")

    if bits_per_sample == 16:
        return payload
    if bits_per_sample == 24:
        samples = array(
            "h",
            (
                sample24_to_int16(unpack_sample24(payload, index * 3))
                for index in range(frames * channels)
            ),
        )
    else:
        samples = array(
            "h",
            (
                float_to_int16(item[0])
                for item in struct.iter_unpack("<f", payload)
            ),
        )
    if byteorder != "little":
        samples.byteswap()
    return samples.tobytes()


def _pcm16_bytes_to_buffer(
    payload: bytes,
    *,
    sample_rate: int,
    channels: int,
) -> PcmBuffer:
    """Expand validated interleaved signed-16 bytes into a typed PCM buffer."""
    samples = array("h")
    samples.frombytes(payload)
    if byteorder != "little":
        samples.byteswap()
    frames = len(samples) // channels
    pcm = tuple(
        tuple(
            int16_to_domain_value(samples[frame * channels + channel])
            for frame in range(frames)
        )
        for channel in range(channels)
    )
    return PcmBuffer(sample_rate=sample_rate, channels=pcm)


__all__ = ["read_raw_pcm"]
