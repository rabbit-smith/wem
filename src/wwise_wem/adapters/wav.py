"""Typed WAV input adapters for the public encoder facade."""

from __future__ import annotations

import struct
import wave
from pathlib import Path

from ..model import PcmBuffer
from .sample_conversion import float_to_int16, int16_to_domain_value, sample24_to_int16, unpack_sample24


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


def _probe_wav(path: Path) -> tuple[int, int, int, int, bytes]:
    """Parse the RIFF/WAVE header of ``path``.

    Returns ``(format_tag, channels, sample_rate, sampwidth, data)`` for
    the first ``fmt `` and the first ``data`` chunk; ``sampwidth`` (bytes
    per sample) is derived from the fmt bits-per-sample field.  Raises
    ``ValueError`` for non-RIFF containers, missing chunks, and truncated
    chunk headers.  Interleaved chunk padding bytes are skipped.
    """
    blob = path.read_bytes()
    if len(blob) < 12 or blob[:4] != b"RIFF" or blob[8:12] != b"WAVE":
        raise ValueError("encoder input must be a RIFF/WAVE file")
    offset = 12
    format_tag: int | None = None
    channels: int | None = None
    sample_rate: int | None = None
    sampwidth: int | None = None
    data: bytes | None = None
    while offset + 8 <= len(blob):
        chunk_id, chunk_size = struct.unpack_from("<4sI", blob, offset)
        offset += 8
        if chunk_id == b"fmt " and format_tag is None:
            if offset + chunk_size > len(blob) or chunk_size < 16:
                raise ValueError("WAV fmt chunk is malformed or truncated")
            unpacked = struct.unpack_from("<HHIIHH", blob, offset)
            format_tag = int(unpacked[0])
            channels = int(unpacked[1])
            sample_rate = int(unpacked[2])
            bits_per_sample = int(unpacked[5])
            if bits_per_sample % 8:
                raise ValueError("WAV bits per sample is not a whole byte count")
            sampwidth = bits_per_sample // 8
        elif chunk_id == b"data" and data is None:
            if offset + chunk_size > len(blob):
                raise ValueError("WAV data chunk is truncated")
            data = blob[offset : offset + chunk_size]
        offset += chunk_size + (chunk_size & 1)
    if (
        format_tag is None
        or channels is None
        or sample_rate is None
        or sampwidth is None
    ):
        raise ValueError("WAV header is missing the fmt chunk")
    if data is None:
        raise ValueError("WAV header is missing the data chunk")
    return format_tag, channels, sample_rate, sampwidth, data


def read_pcm_wav(path: Path) -> PcmBuffer:
    """Read a PCM WAV in any supported format into the signed-16 domain.

    Supported formats (RIFF/WAVE):

    * format tag 1 (PCM), 16-bit signed little-endian; and
    * format tag 1 (PCM), 24-bit signed little-endian; and
    * format tag 3 (IEEE float), 32-bit float32 little-endian.

    16-bit files go through the legacy :func:`read_pcm16` path unchanged.
    24-bit and float32 files are converted at this adapter boundary by the
    deterministic rules in :mod:`wwise_wem.adapters.sample_conversion`:
    by the time the :class:`PcmBuffer` exists, every sample is an exact
    ``value / 32768.0`` in-domain float, so both engines see only the
    signed-16 domain.  Converted inputs are deterministic and dual-engine
    consistent; they are not promised bit-exact against Wwise imports.
    """
    format_tag, channels, sample_rate, sampwidth, data = _probe_wav(path)
    if format_tag == 1 and sampwidth == 2:
        return read_pcm16(path)

    if channels <= 0 or sample_rate <= 0:
        raise ValueError("WAV geometry must be positive (channels, sample rate)")
    if not (
        (format_tag == 1 and sampwidth == 3)
        or (format_tag == 3 and sampwidth == 4)
    ):
        raise ValueError(
            "encoder input WAV must be 16-bit PCM, 24-bit PCM, or "
            "32-bit IEEE-float PCM"
        )
    bytes_per_frame = channels * sampwidth
    if len(data) % bytes_per_frame:
        raise ValueError("WAV PCM data does not align to whole frames")
    frames = len(data) // bytes_per_frame
    if frames == 0:
        raise ValueError("encoder input WAV must contain at least one frame")

    if sampwidth == 3:
        pcm = tuple(
            tuple(
                int16_to_domain_value(
                    sample24_to_int16(
                        unpack_sample24(
                            data, frame * bytes_per_frame + channel * 3
                        )
                    )
                )
                for frame in range(frames)
            )
            for channel in range(channels)
        )
    else:  # 32-bit IEEE float
        values = struct.unpack(f"<{frames * channels}f", data)
        pcm = tuple(
            tuple(
                int16_to_domain_value(
                    float_to_int16(values[frame * channels + channel])
                )
                for frame in range(frames)
            )
            for channel in range(channels)
        )
    return PcmBuffer(sample_rate=sample_rate, channels=pcm)


__all__ = ["read_pcm16", "read_pcm_wav", "read_wav_geometry"]
