"""Typed WAV input adapters for the public encoder facade."""

from __future__ import annotations

import struct
from pathlib import Path

from ..model import PcmBuffer
from .raw import _normalize_pcm16_bytes, _pcm16_bytes_to_buffer


def _probe_wav(path: Path) -> tuple[int, int, int, int, bytes]:
    """Parse the RIFF/WAVE header of ``path``.

    Returns ``(format_tag, channels, sample_rate, sampwidth, data)`` for
    the first ``fmt `` and the first ``data`` chunk; ``sampwidth`` (bytes
    per sample) is derived from the fmt bits-per-sample field.  Raises
    ``ValueError`` for non-RIFF containers, missing chunks, and truncated
    chunk headers.  Interleaved chunk padding bytes are skipped.
    """
    format_tag: int | None = None
    channels: int | None = None
    sample_rate: int | None = None
    sampwidth: int | None = None
    data: bytes | None = None
    with path.open("rb") as source:
        source.seek(0, 2)
        file_size = source.tell()
        source.seek(0)
        header = source.read(12)
        if len(header) != 12 or header[:4] != b"RIFF" or header[8:] != b"WAVE":
            raise ValueError("encoder input must be a RIFF/WAVE file")
        riff_end = 8 + struct.unpack_from("<I", header, 4)[0]
        if riff_end < 12 or riff_end > file_size:
            raise ValueError("WAV declared RIFF payload is truncated")
        while format_tag is None or data is None:
            remaining = riff_end - source.tell()
            if remaining == 0:
                break
            if remaining < 8:
                raise ValueError("WAV chunk header is truncated")
            chunk_header = source.read(8)
            if len(chunk_header) != 8:
                raise ValueError("WAV chunk header is truncated")
            chunk_id, chunk_size = struct.unpack("<4sI", chunk_header)
            payload_start = source.tell()
            payload_end = payload_start + chunk_size
            if payload_end > riff_end:
                label = chunk_id.decode("ascii", "replace").strip() or "unknown"
                raise ValueError(f"WAV {label} chunk is truncated")
            if chunk_id == b"fmt " and format_tag is None:
                if chunk_size < 16:
                    raise ValueError("WAV fmt chunk is malformed or truncated")
                prefix = source.read(16)
                if len(prefix) != 16:
                    raise ValueError("WAV fmt chunk is malformed or truncated")
                unpacked = struct.unpack("<HHIIHH", prefix)
                format_tag = int(unpacked[0])
                channels = int(unpacked[1])
                sample_rate = int(unpacked[2])
                bits_per_sample = int(unpacked[5])
                if bits_per_sample % 8:
                    raise ValueError("WAV bits per sample is not a whole byte count")
                sampwidth = bits_per_sample // 8
                source.seek(payload_end)
            elif chunk_id == b"data" and data is None:
                data = source.read(chunk_size)
                if len(data) != chunk_size:
                    raise ValueError("WAV data chunk is truncated")
            else:
                source.seek(payload_end)
            if chunk_size & 1 and source.tell() < riff_end:
                source.seek(1, 1)
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

    24-bit and float32 files are converted at this adapter boundary by the
    deterministic rules in :mod:`wwise_wem.adapters.sample_conversion`:
    by the time the :class:`PcmBuffer` exists, every sample is an exact
    ``value / 32768.0`` in-domain float, so the encoder sees only the
    signed-16 domain.  Converted inputs are deterministic; they are not
    promised bit-exact against Wwise imports.
    """
    sample_rate, channels, payload = _read_wav_pcm16_bytes(path)
    return _pcm16_bytes_to_buffer(
        payload,
        sample_rate=sample_rate,
        channels=channels,
    )


def _read_wav_pcm16_bytes(path: Path) -> tuple[int, int, bytes]:
    """Read a supported WAV as interleaved signed-16 little-endian bytes."""
    format_tag, channels, sample_rate, sampwidth, data = _probe_wav(path)
    if channels <= 0 or sample_rate <= 0:
        raise ValueError("WAV geometry must be positive (channels, sample rate)")
    if not (
        (format_tag == 1 and sampwidth in (2, 3))
        or (format_tag == 3 and sampwidth == 4)
    ):
        raise ValueError(
            "encoder input WAV must be 16-bit PCM, 24-bit PCM, or "
            "32-bit IEEE-float PCM"
        )
    bytes_per_frame = channels * sampwidth
    if len(data) % bytes_per_frame:
        raise ValueError("WAV PCM data does not align to whole frames")
    if not data:
        raise ValueError("encoder input WAV must contain at least one frame")
    payload = _normalize_pcm16_bytes(
        data,
        sample_rate=sample_rate,
        channels=channels,
        bits_per_sample=sampwidth * 8,
    )
    return sample_rate, channels, payload


__all__ = ["read_pcm_wav"]
