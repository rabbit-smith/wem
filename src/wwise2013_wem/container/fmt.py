"""Pure byte codecs for Wwise Vorbis and PCM ``fmt `` payloads."""

from __future__ import annotations

import struct


WWISE_VORBIS_FORMAT_TAG = 0xFFFF
WWISE_PCM_EXT_FORMAT_TAG = 0xFFFE
WWISE_PCM_FORMAT_TAG = 0x0001
WWISE_VORBIS_FMT_SIZE = 66
WWISE_PCM_EXT_FMT_SIZE = 24
WWISE_SPEAKER_5POINT1 = 0x3F


def _u16(data: bytes, offset: int) -> int:
    return struct.unpack_from("<H", data, offset)[0]


def _u32(data: bytes, offset: int) -> int:
    return struct.unpack_from("<I", data, offset)[0]


def _hex_or_int(value) -> int:
    if isinstance(value, int):
        return value
    if isinstance(value, str):
        return int(value, 0)
    raise TypeError(f"expected int/str, got {type(value)}")


def parse_vorbis_fmt(payload: bytes) -> dict:
    if len(payload) < WWISE_VORBIS_FMT_SIZE:
        raise ValueError(
            f"vorbis fmt expected {WWISE_VORBIS_FMT_SIZE} bytes, got {len(payload)}"
        )
    return {
        "wFormatTag": hex(_u16(payload, 0)),
        "nChannels": _u16(payload, 2),
        "nSamplesPerSec": _u32(payload, 4),
        "nAvgBytesPerSec": _u32(payload, 8),
        "nBlockAlign": _u16(payload, 0x0C),
        "wBitsPerSample": _u16(payload, 0x0E),
        "cbSize": _u16(payload, 0x10),
        "wReserved0": _u16(payload, 0x12),
        "dwChannelMask": hex(_u32(payload, 0x14)),
        "dwTotalPCMFrames": _u32(payload, 0x18),
        "dwFirstAudioPacketOffset": _u32(payload, 0x1C),
        "dwDataPayloadSize": _u32(payload, 0x20),
        "dwUnknown_0x24": hex(_u32(payload, 0x24)),
        "dwSeekTableSize": _u32(payload, 0x28),
        "dwVorbisDataOffset": _u32(payload, 0x2C),
        "uMaxPacketSize": _u16(payload, 0x30),
        "uUnknown_0x32": _u16(payload, 0x32),
        "dwUnknown_0x34": _u32(payload, 0x34),
        "dwUnknown_0x38": _u32(payload, 0x38),
        "dwUnknown_0x3C": hex(_u32(payload, 0x3C)),
        "uBlocksize0Pow": payload[0x40],
        "uBlocksize1Pow": payload[0x41],
        "blocksize0": 1 << payload[0x40],
        "blocksize1": 1 << payload[0x41],
    }


def pack_vorbis_fmt(fields: dict) -> bytes:
    """Build a 66-byte Wwise Vorbis fmt payload."""
    if fields.get("raw_hex"):
        raw = bytes.fromhex(fields["raw_hex"])
        if len(raw) != WWISE_VORBIS_FMT_SIZE:
            raise ValueError(f"raw_hex fmt len {len(raw)} != 66")
        return raw

    tag = _hex_or_int(fields.get("wFormatTag", WWISE_VORBIS_FORMAT_TAG))
    buf = bytearray(WWISE_VORBIS_FMT_SIZE)
    struct.pack_into("<H", buf, 0x00, tag)
    struct.pack_into("<H", buf, 0x02, int(fields["nChannels"]))
    struct.pack_into("<I", buf, 0x04, int(fields["nSamplesPerSec"]))
    struct.pack_into("<I", buf, 0x08, int(fields["nAvgBytesPerSec"]))
    struct.pack_into("<H", buf, 0x0C, int(fields.get("nBlockAlign", 0)))
    struct.pack_into("<H", buf, 0x0E, int(fields.get("wBitsPerSample", 0)))
    struct.pack_into("<H", buf, 0x10, int(fields.get("cbSize", 48)))
    struct.pack_into(
        "<H", buf, 0x12, int(fields.get("wReserved0", fields.get("u16_0x12", 0)))
    )
    struct.pack_into(
        "<I",
        buf,
        0x14,
        _hex_or_int(fields.get("dwChannelMask", fields.get("u32_0x14", 0))),
    )
    struct.pack_into("<I", buf, 0x18, int(fields["dwTotalPCMFrames"]))
    first = int(
        fields.get(
            "dwFirstAudioPacketOffset",
            fields.get("dwFirstAudioOffset", fields.get("dwVorbisDataOffset", 0)),
        )
    )
    struct.pack_into("<I", buf, 0x1C, first)
    struct.pack_into("<I", buf, 0x20, int(fields["dwDataPayloadSize"]))
    struct.pack_into(
        "<I",
        buf,
        0x24,
        _hex_or_int(fields.get("dwUnknown_0x24", fields.get("u32_0x24", 0))),
    )
    struct.pack_into("<I", buf, 0x28, int(fields.get("dwSeekTableSize", 0)))
    struct.pack_into("<I", buf, 0x2C, int(fields.get("dwVorbisDataOffset", first)))
    struct.pack_into("<H", buf, 0x30, int(fields.get("uMaxPacketSize", 0)))
    struct.pack_into(
        "<H",
        buf,
        0x32,
        int(fields.get("uUnknown_0x32", fields.get("u16_0x32", 0))),
    )
    struct.pack_into(
        "<I",
        buf,
        0x34,
        int(fields.get("dwUnknown_0x34", fields.get("u32_0x34", 0))),
    )
    struct.pack_into(
        "<I",
        buf,
        0x38,
        int(fields.get("dwUnknown_0x38", fields.get("u32_0x38", 0))),
    )
    struct.pack_into(
        "<I",
        buf,
        0x3C,
        _hex_or_int(fields.get("dwUnknown_0x3C", fields.get("u32_0x3C", 0))),
    )
    buf[0x40] = int(fields.get("uBlocksize0Pow", 8)) & 0xFF
    buf[0x41] = int(fields.get("uBlocksize1Pow", 11)) & 0xFF
    return bytes(buf)


def parse_pcm_ext_fmt(payload: bytes) -> dict:
    fields = {
        "wFormatTag": hex(_u16(payload, 0)),
        "nChannels": _u16(payload, 2),
        "nSamplesPerSec": _u32(payload, 4),
        "nAvgBytesPerSec": _u32(payload, 8),
        "nBlockAlign": _u16(payload, 0x0C),
        "wBitsPerSample": _u16(payload, 0x0E),
        "cbSize": _u16(payload, 0x10) if len(payload) >= 18 else 0,
    }
    if len(payload) >= WWISE_PCM_EXT_FMT_SIZE:
        fields["wSamples"] = _u16(payload, 0x12)
        fields["dwChannelMask"] = hex(_u32(payload, 0x14))
    return fields


def pack_pcm_ext_fmt(fields: dict) -> bytes:
    """Build a PCM extensible fmt payload or the legacy 16-byte PCM form."""
    if fields.get("raw_hex"):
        return bytes.fromhex(fields["raw_hex"])

    tag = _hex_or_int(fields.get("wFormatTag", WWISE_PCM_EXT_FORMAT_TAG))
    channels = int(fields["nChannels"])
    sample_rate = int(fields["nSamplesPerSec"])
    bits = int(fields.get("wBitsPerSample", 16))
    align = int(fields.get("nBlockAlign", channels * (bits // 8)))
    average = int(fields.get("nAvgBytesPerSec", sample_rate * align))
    cb_size = int(fields.get("cbSize", 6 if tag == WWISE_PCM_EXT_FORMAT_TAG else 0))

    if tag == WWISE_PCM_FORMAT_TAG and cb_size == 0 and "dwChannelMask" not in fields:
        return struct.pack(
            "<HHIIHHH",
            tag,
            channels,
            sample_rate,
            average,
            align,
            bits,
            0,
        )

    buf = bytearray(WWISE_PCM_EXT_FMT_SIZE)
    struct.pack_into("<H", buf, 0x00, tag)
    struct.pack_into("<H", buf, 0x02, channels)
    struct.pack_into("<I", buf, 0x04, sample_rate)
    struct.pack_into("<I", buf, 0x08, average)
    struct.pack_into("<H", buf, 0x0C, align)
    struct.pack_into("<H", buf, 0x0E, bits)
    struct.pack_into("<H", buf, 0x10, cb_size)
    struct.pack_into("<H", buf, 0x12, int(fields.get("wSamples", 0)))
    struct.pack_into(
        "<I",
        buf,
        0x14,
        _hex_or_int(fields.get("dwChannelMask", WWISE_SPEAKER_5POINT1)),
    )
    return bytes(buf)
