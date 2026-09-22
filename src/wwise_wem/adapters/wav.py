"""Typed WAV input adapters for the public encoder facade."""

from __future__ import annotations

import struct
from dataclasses import dataclass
from pathlib import Path

from ..model import PcmBuffer
from .raw import _normalize_pcm16_bytes, _pcm16_bytes_to_buffer

# The fmt tags this reader resolves: the two the canonical header carries
# directly, and the one that replaced them for files whose representation the
# original 16-byte header could not name.
_WAVE_FORMAT_PCM = 0x0001
_WAVE_FORMAT_IEEE_FLOAT = 0x0003
_WAVE_FORMAT_EXTENSIBLE = 0xFFFE

# The two KSDATAFORMAT sub-format GUIDs, in their stored little-endian byte
# order: the first four bytes are the format tag, the remaining twelve are the
# same suffix in both.  Only these two are read; any other sub-format -- A-law,
# mu-law, ADPCM, a vendor GUID -- is a different audio format, not another
# spelling of one this reader takes.
_KSDATAFORMAT_SUBTYPE_PCM = bytes.fromhex("0100000000001000800000aa00389b71")
_KSDATAFORMAT_SUBTYPE_IEEE_FLOAT = bytes.fromhex("0300000000001000800000aa00389b71")
_KSDATAFORMAT_SUFFIX = bytes.fromhex("00001000800000aa00389b71")

# The 22 bytes a WAVE_FORMAT_EXTENSIBLE fmt chunk carries beyond the canonical
# 16: cbSize, wValidBitsPerSample, dwChannelMask, SubFormat.
_EXTENSIBLE_FMT_SIZE = 40

_REPRESENTATION_REFUSAL = (
    "encoder input WAV must be 16-bit PCM, 24-bit PCM, or 32-bit IEEE-float PCM"
)


@dataclass(frozen=True)
class _ExtensibleFormat:
    """The extension a ``WAVE_FORMAT_EXTENSIBLE`` ``fmt `` chunk carries.

    ``valid_bits`` and ``sub_format`` decide whether the header means one of
    the representations this reader takes.  ``channel_mask`` is parsed and
    deliberately not acted on: the reader's contract is interleaved samples in
    the file's own order, and the geometry a mask could corroborate is checked
    later, at profile selection.
    """

    valid_bits: int
    channel_mask: int
    sub_format: bytes


@dataclass(frozen=True)
class _WavHeader:
    """One parsed RIFF/WAVE header: the first ``fmt `` chunk and the data."""

    format_tag: int
    channels: int
    sample_rate: int
    sampwidth: int
    extensible: _ExtensibleFormat | None
    data: bytes


def _probe_wav(path: Path) -> _WavHeader:
    """Parse the RIFF/WAVE header of ``path``.

    Returns the first ``fmt `` chunk's fields and the first ``data`` chunk's
    bytes; ``sampwidth`` (bytes per sample) is derived from the fmt
    bits-per-sample field.  A canonical 16-byte fmt chunk leaves
    ``extensible`` unset; a ``WAVE_FORMAT_EXTENSIBLE`` one is read at its
    40-byte layout, and a fmt chunk too short to hold that layout is refused
    rather than read past.  Raises ``ValueError`` for non-RIFF containers,
    missing chunks, and truncated chunk headers.  Interleaved chunk padding
    bytes are skipped.
    """
    format_tag: int | None = None
    channels: int | None = None
    sample_rate: int | None = None
    sampwidth: int | None = None
    extensible: _ExtensibleFormat | None = None
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
                if format_tag != _WAVE_FORMAT_EXTENSIBLE:
                    source.seek(payload_end)
                else:
                    if chunk_size < _EXTENSIBLE_FMT_SIZE:
                        raise ValueError(
                            "WAV extensible fmt chunk is malformed or truncated"
                        )
                    extension = source.read(_EXTENSIBLE_FMT_SIZE - 16)
                    if len(extension) != _EXTENSIBLE_FMT_SIZE - 16:
                        raise ValueError(
                            "WAV extensible fmt chunk is malformed or truncated"
                        )
                    extensible = _ExtensibleFormat(
                        valid_bits=struct.unpack_from("<H", extension, 2)[0],
                        channel_mask=struct.unpack_from("<I", extension, 4)[0],
                        sub_format=extension[8:24],
                    )
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
    return _WavHeader(
        format_tag=format_tag,
        channels=channels,
        sample_rate=sample_rate,
        sampwidth=sampwidth,
        extensible=extensible,
        data=data,
    )


def _sub_format_text(sub_format: bytes) -> str:
    """Name a sub-format for a refusal: its tag, or the GUID when it has none.

    A KSDATAFORMAT sub-format is the format tag this reader already knows plus
    a fixed twelve-byte suffix, so the tag is the useful half and is reported
    alone; anything else is a GUID this reader knows nothing about, and is
    reported whole rather than guessed at from its first field.
    """
    if len(sub_format) == 16 and sub_format[4:] == _KSDATAFORMAT_SUFFIX:
        return f"tag {int.from_bytes(sub_format[:4], 'little')}"
    return "-".join(
        (
            sub_format[0:4][::-1].hex(),
            sub_format[4:6][::-1].hex(),
            sub_format[6:8][::-1].hex(),
            sub_format[8:10].hex(),
            sub_format[10:16].hex(),
        )
    )


def _resolve_representation(header: _WavHeader) -> tuple[int, int]:
    """Map a header onto the (format tag, container width) the gate reads.

    A canonical header already carries its own tag.  A
    ``WAVE_FORMAT_EXTENSIBLE`` header moved the tag into the sub-format GUID,
    so the GUID is what decides here: one of the two KSDATAFORMAT sub-formats
    this reader resolves means the same representation its tag would have
    meant directly, and everything else is a format this reader does not take.

    ``wValidBitsPerSample`` is checked rather than assumed.  When it is not the
    container's width the samples are not the full-width representation the
    conversion rules are stated for: fewer valid bits means left-aligned
    samples with the unused low bits zero-padded, and folding that into the
    conversion would be a fourth rule -- one that reads bits no source obliges
    a writer to zero, so the same bytes could mean two different samples.  The
    refusal names the count instead, which is what a caller needs to convert.
    """
    extensible = header.extensible
    if extensible is None:
        return header.format_tag, header.sampwidth
    if extensible.sub_format == _KSDATAFORMAT_SUBTYPE_PCM:
        format_tag = _WAVE_FORMAT_PCM
    elif extensible.sub_format == _KSDATAFORMAT_SUBTYPE_IEEE_FLOAT:
        format_tag = _WAVE_FORMAT_IEEE_FLOAT
    else:
        raise ValueError(
            f"{_REPRESENTATION_REFUSAL}; got WAVE_FORMAT_EXTENSIBLE sub-format "
            f"{_sub_format_text(extensible.sub_format)} at "
            f"{header.sampwidth * 8} bits per sample"
        )
    container_bits = header.sampwidth * 8
    if extensible.valid_bits != container_bits:
        raise ValueError(
            f"{_REPRESENTATION_REFUSAL}; got WAVE_FORMAT_EXTENSIBLE "
            f"wValidBitsPerSample={extensible.valid_bits} in a "
            f"{container_bits}-bit container"
        )
    return format_tag, header.sampwidth


def read_pcm_wav(path: Path) -> PcmBuffer:
    """Read a PCM WAV in any supported format into the signed-16 domain.

    Supported formats (RIFF/WAVE), in either the canonical 16-byte ``fmt ``
    chunk or the 40-byte ``WAVE_FORMAT_EXTENSIBLE`` one that names the same
    representation through its sub-format GUID:

    * format tag 1 (PCM), 16-bit signed little-endian; and
    * format tag 1 (PCM), 24-bit signed little-endian; and
    * format tag 3 (IEEE float), 32-bit float32 little-endian.

    The extensible header is the one ``ffmpeg`` writes whenever a source has
    more than two channels or integer samples wider than 16 bits, so the
    common case reaches the same three representations instead of being
    refused for its header.  ``WAVE_FORMAT_EXTENSIBLE`` is a wider header, not
    a wider set of audio formats: a sub-format GUID other than those two tags
    is refused exactly as its tag would be, and a container whose
    ``wValidBitsPerSample`` is not its own width is refused with the count
    named (see :func:`_resolve_representation`).  ``dwChannelMask`` is read
    and not acted on: samples are taken in the file's interleaved order, and
    nothing here validates that order against the mask.

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
    header = _probe_wav(path)
    if header.channels <= 0 or header.sample_rate <= 0:
        raise ValueError("WAV geometry must be positive (channels, sample rate)")
    format_tag, sampwidth = _resolve_representation(header)
    if not (
        (format_tag == 1 and sampwidth in (2, 3))
        or (format_tag == 3 and sampwidth == 4)
    ):
        if header.extensible is None:
            seen = f"format tag {format_tag}"
        else:
            seen = (
                "WAVE_FORMAT_EXTENSIBLE sub-format "
                f"{_sub_format_text(header.extensible.sub_format)}"
            )
        raise ValueError(
            f"{_REPRESENTATION_REFUSAL}; got {seen} at "
            f"{sampwidth * 8} bits per sample"
        )
    data = header.data
    bytes_per_frame = header.channels * sampwidth
    if len(data) % bytes_per_frame:
        raise ValueError("WAV PCM data does not align to whole frames")
    if not data:
        raise ValueError("encoder input WAV must contain at least one frame")
    payload = _normalize_pcm16_bytes(
        data,
        sample_rate=header.sample_rate,
        channels=header.channels,
        bits_per_sample=sampwidth * 8,
    )
    return header.sample_rate, header.channels, payload


__all__ = ["read_pcm_wav"]
