"""Pure byte codecs for Wwise size-prefixed packet streams."""

from __future__ import annotations

import struct

from .fmt import WWISE_VORBIS_FORMAT_TAG


def extract_packets(data: bytes, seek_table_size: int = 0, *, endian: str = "le") -> dict:
    """Split a data payload into its seek table and sized packets."""
    if seek_table_size < 0 or seek_table_size > len(data):
        raise ValueError(f"bad seek_table_size={seek_table_size}")
    if seek_table_size & 3:
        # Historical Wwise inputs are aligned, but preserve permissive parsing.
        pass
    seek = data[:seek_table_size]
    sizes: list[int] = []
    packets: list[bytes] = []
    position = seek_table_size
    u16_fmt = "<H" if endian == "le" else ">H"
    while position + 2 <= len(data):
        size = struct.unpack_from(u16_fmt, data, position)[0]
        if position + 2 + size > len(data):
            return {
                "ok": False,
                "seek_table": seek,
                "packets": packets,
                "sizes": sizes,
                "packet_count": len(sizes),
                "error_at": position,
                "error_size": size,
                "end": position,
                "data_size": len(data),
            }
        payload = data[position + 2 : position + 2 + size]
        sizes.append(size)
        packets.append(payload)
        position += 2 + size
    setup = sizes[0] if sizes else None
    return {
        "ok": position == len(data),
        "seek_table": seek,
        "packets": packets,
        "sizes": sizes,
        "packet_count": len(sizes),
        "end": position,
        "data_size": len(data),
        "setup_packet_size": setup,
        "first_audio_offset": (seek_table_size + 2 + setup if setup is not None else None),
        "sizes_head": sizes[:8],
    }


def walk_packets(
    data: bytes,
    seek_table_size: int,
    *,
    endian: str = "le",
) -> dict:
    """Return packet framing metadata without exposing copied payload lists."""
    extracted = extract_packets(data, seek_table_size, endian=endian)
    return {
        key: value
        for key, value in extracted.items()
        if key not in ("seek_table", "packets", "sizes")
    }


def build_packet_stream(
    packets: list[bytes],
    seek_table: bytes = b"",
    *,
    endian: str = "le",
) -> bytes:
    """Build ``seek_table + Σ(u16 size + payload)`` bytes."""
    u16_fmt = "<H" if endian == "le" else ">H"
    out = bytearray(seek_table)
    for packet in packets:
        if len(packet) > 0xFFFF:
            raise ValueError(f"packet too large: {len(packet)}")
        out += struct.pack(u16_fmt, len(packet))
        out += packet
    return bytes(out)


def _optional_int(value: int | str | float | None) -> int | None:
    """Coerce a fmt field to int, or None when absent/not numeric.

    Callers use a None result to keep the carried field untouched rather than
    writing a bogus derived value.
    """
    if value is None:
        return None
    try:
        return int(value)
    except (TypeError, ValueError):
        return None


def recompute_vorbis_fmt_sizes(
    fmt_fields: dict,
    packets: list[bytes],
    seek_table: bytes = b"",
) -> dict:
    """Return fmt fields with packet-derived sizes and offsets updated.

    ``nAvgBytesPerSec`` is derived rather than carried: Wwise writes
    ``floor(data_payload_bytes * nSamplesPerSec / dwTotalPCMFrames)``. Verified
    against six reference WEMs from the paired 2013.2 build (the 6ch/44.1k
    fixture gives 108677 B / 139398 frames / 44100 Hz -> 34381, and a 2ch/48k
    sample with an exact quotient of 18600.5 pins the operator to truncation).
    """
    fields = dict(fmt_fields)
    data_size = len(seek_table) + sum(2 + len(packet) for packet in packets)
    setup = packets[0] if packets else b""
    first = (2 + len(setup)) if packets else 0
    first_in_data = len(seek_table) + first
    fields["dwSeekTableSize"] = len(seek_table)
    fields["dwDataPayloadSize"] = data_size
    fields["dwFirstAudioPacketOffset"] = first_in_data
    fields["dwVorbisDataOffset"] = first_in_data
    # ``uMaxPacketSize`` is the largest *audio* packet: the paired build writes 1
    # for an all-silent 2ch/48k stream whose setup packet is 215 bytes.  Counting
    # the setup packet only changes the field when it is the largest one.
    if len(packets) > 1:
        fields["uMaxPacketSize"] = max(len(packet) for packet in packets[1:])
    total_frames = _optional_int(fields.get("dwTotalPCMFrames"))
    sample_rate = _optional_int(fields.get("nSamplesPerSec"))
    if total_frames and sample_rate:
        fields["nAvgBytesPerSec"] = data_size * sample_rate // total_frames
    fields["wFormatTag"] = fields.get("wFormatTag", WWISE_VORBIS_FORMAT_TAG)
    return fields
