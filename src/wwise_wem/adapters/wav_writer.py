"""Signed-16 PCM WAV output for the decode direction.

The mirror of :mod:`wwise_wem.adapters.wav`, and the one place this package
turns decoded samples into a container: the decode command line writes a WAV
the way the encode command line writes a WEM, so a caller is handed a file
rather than a header it would have to build itself.

The sample rule is
:func:`wwise_wem.adapters.sample_conversion.float_to_int16` — the package's own
normative float-to-signed-16 mapping, which the Rust command line applies too,
so both front ends write the same bytes for the same decode. Samples are
interleaved and little-endian; the channel count and sample rate come from the
decode header announcement.
"""

from __future__ import annotations

import struct
from array import array
from collections.abc import Iterable
from sys import byteorder

from .sample_conversion import float_to_int16

__all__ = ["pcm16_wav_bytes"]

# WAVE_FORMAT_PCM: uncompressed signed-integer samples, the layout the encode
# side's reader accepts, so a decoded file can be handed straight back to it.
_WAVE_FORMAT_PCM = 1
_BITS_PER_SAMPLE = 16
_BYTES_PER_SAMPLE = _BITS_PER_SAMPLE // 8

# The RIFF/WAVE header before the sample payload: "RIFF" + size + "WAVE", the
# 16-byte `fmt ` chunk, and "data" + size.
_HEADER_BYTES = 44

# The largest `data` payload a RIFF size field can describe: that field counts
# 36 bytes of framing plus the payload.
_MAX_DATA_BYTES = 0xFFFFFFFF - (_HEADER_BYTES - 8)


def _require_positive(value: int, label: str) -> int:
    if not isinstance(value, int) or isinstance(value, bool):
        raise TypeError(f"{label} must be an integer")
    if value <= 0:
        raise ValueError(f"{label} must be positive")
    return value


def pcm16_wav_bytes(
    channels: int,
    sample_rate: int,
    blocks: Iterable[Iterable[float]],
) -> bytes:
    """Build one uncompressed signed-16 PCM WAV from decoded f32 blocks.

    ``blocks`` is what a decode result yields: interleaved f32 samples at
    ``±1.0`` full scale, in whole frames at ``channels``. Each sample is mapped
    by :func:`wwise_wem.adapters.sample_conversion.float_to_int16` — scale by
    ``32768.0``, round to nearest with ties away from zero, saturate — so a
    decoded sample outside ``±1.0`` is saturated rather than wrapped, and a
    non-finite sample is a ``ValueError`` rather than a value invented here.

    ``channels`` and ``sample_rate`` are the decode header's announcement. The
    payload is counted from the samples handed in, so the header cannot
    describe data the file does not hold; ``ValueError`` for a block that is
    not a whole number of frames, and for a stream past the RIFF size field's
    own limit.
    """
    channels = _require_positive(channels, "channels")
    sample_rate = _require_positive(sample_rate, "sample_rate")
    if channels > 0xFFFF:
        raise ValueError("channels must fit the WAV header's 16-bit field")
    frame_bytes = channels * _BYTES_PER_SAMPLE
    byte_rate = sample_rate * frame_bytes
    if byte_rate > 0xFFFFFFFF:
        raise ValueError(
            "the sample rate and channel count exceed the WAV header's byte-rate field"
        )

    payload = bytearray()
    for block in blocks:
        samples = tuple(block)
        if len(samples) % channels:
            raise ValueError(
                f"a decoded block of {len(samples)} samples is not a whole number of "
                f"{channels}-channel frames"
            )
        values = array("h", (float_to_int16(sample) for sample in samples))
        if byteorder != "little":
            values.byteswap()
        payload += values.tobytes()
        if len(payload) > _MAX_DATA_BYTES:
            # Checked as the stream arrives, not after it is all in memory: a
            # RIFF size field cannot describe this payload, and saying so is
            # cheaper than holding it.
            raise ValueError(
                f"the decoded PCM is past the {_MAX_DATA_BYTES}-byte limit a RIFF "
                "container can describe"
            )

    return struct.pack(
        "<4sI4s4sIHHIIHH4sI",
        b"RIFF",
        _HEADER_BYTES - 8 + len(payload),
        b"WAVE",
        b"fmt ",
        16,
        _WAVE_FORMAT_PCM,
        channels,
        sample_rate,
        byte_rate,
        frame_bytes,
        _BITS_PER_SAMPLE,
        b"data",
        len(payload),
    ) + bytes(payload)
