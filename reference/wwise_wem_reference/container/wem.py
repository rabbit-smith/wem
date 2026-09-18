"""Pure byte composition for complete Wwise WEM containers."""

from __future__ import annotations

import struct

from .fmt import (
    WWISE_PCM_EXT_FORMAT_TAG,
    WWISE_PCM_FORMAT_TAG,
    WWISE_VORBIS_FMT_SIZE,
    WWISE_VORBIS_FORMAT_TAG,
    _u16,
    pack_pcm_ext_fmt,
    pack_vorbis_fmt,
    parse_pcm_ext_fmt,
    parse_vorbis_fmt,
)
from .packets import build_packet_stream, extract_packets, recompute_vorbis_fmt_sizes, walk_packets
from .riff import build_riff, parse_chunks


def parse_wem_bytes(
    raw: bytes, *, file: str = "<bytes>", path: str = "<bytes>"
) -> dict:
    """Parse WEM bytes into the legacy summary schema without filesystem I/O."""
    endian, chunks = parse_chunks(raw)
    info: dict = {
        "file": file,
        "path": path,
        "size": len(raw),
        "endian": endian,
        "chunks": [
            {"id": chunk["id"], "size": chunk["size"], "off": hex(chunk["off"])}
            for chunk in chunks
        ],
    }
    fmt_chunk = next((chunk for chunk in chunks if chunk["id"] == "fmt "), None)
    data_chunk = next((chunk for chunk in chunks if chunk["id"] == "data"), None)
    if not fmt_chunk:
        info["error"] = "missing fmt"
        return info

    payload = fmt_chunk["payload"]
    tag = _u16(payload, 0)
    info["fmt_chunk_size"] = fmt_chunk["size"]

    if tag == WWISE_VORBIS_FORMAT_TAG:
        info["kind"] = "wwise_vorbis"
        info["fmt"] = parse_vorbis_fmt(payload)
        if data_chunk:
            packet_info = walk_packets(
                data_chunk["payload"],
                info["fmt"]["dwSeekTableSize"],
                endian=endian,
            )
            info["data"] = {
                "size": data_chunk["size"],
                **packet_info,
                "checks": {
                    "data_size_field": info["fmt"]["dwDataPayloadSize"]
                    == data_chunk["size"],
                    "vorbis_offset_vs_setup": packet_info.get("first_audio_offset")
                    == info["fmt"]["dwVorbisDataOffset"],
                    "first_audio_field_eq_vorbis_off": info["fmt"][
                        "dwFirstAudioPacketOffset"
                    ]
                    == info["fmt"]["dwVorbisDataOffset"],
                    "packets_consume_data": packet_info.get("ok"),
                },
            }
    elif tag == WWISE_PCM_EXT_FORMAT_TAG:
        info["kind"] = "pcm_extensible"
        info["fmt"] = parse_pcm_ext_fmt(payload)
        if data_chunk:
            align = info["fmt"]["nBlockAlign"] or 1
            frames = data_chunk["size"] // align
            sample_rate = info["fmt"]["nSamplesPerSec"] or 1
            info["data"] = {
                "size": data_chunk["size"],
                "frames": frames,
                "duration_sec": round(frames / sample_rate, 6),
            }
    elif tag == WWISE_PCM_FORMAT_TAG:
        info["kind"] = "pcm_plain"
        info["fmt"] = parse_pcm_ext_fmt(payload)
        if data_chunk:
            align = info["fmt"]["nBlockAlign"] or 1
            frames = data_chunk["size"] // align
            sample_rate = info["fmt"]["nSamplesPerSec"] or 1
            info["data"] = {
                "size": data_chunk["size"],
                "frames": frames,
                "duration_sec": round(frames / sample_rate, 6),
            }
    else:
        info["kind"] = f"unknown_{hex(tag)}"
        info["fmt"] = parse_pcm_ext_fmt(payload)

    info["has_junk"] = any(chunk["id"] == "JUNK" for chunk in chunks)
    info["has_smpl"] = any(chunk["id"] == "smpl" for chunk in chunks)
    return info


def build_vorbis_wem(
    fmt_fields: dict,
    packets: list[bytes],
    *,
    seek_table: bytes = b"",
    endian: str = "le",
    extra_chunks: list[tuple[bytes, bytes]] | None = None,
    recompute_sizes: bool = True,
    fmt_raw: bytes | None = None,
) -> bytes:
    """Build complete Wwise Vorbis WEM bytes."""
    fields = dict(fmt_fields)
    if recompute_sizes and fmt_raw is None:
        fields = recompute_vorbis_fmt_sizes(fields, packets, seek_table)
    fmt_payload = fmt_raw if fmt_raw is not None else pack_vorbis_fmt(fields)
    if len(fmt_payload) != WWISE_VORBIS_FMT_SIZE:
        raise ValueError(f"vorbis fmt must be 66 bytes, got {len(fmt_payload)}")
    data_payload = build_packet_stream(packets, seek_table, endian=endian)
    if recompute_sizes and fmt_raw is not None:
        fmt_buf = bytearray(fmt_payload)
        struct.pack_into("<I", fmt_buf, 0x20, len(data_payload))
        struct.pack_into("<I", fmt_buf, 0x28, len(seek_table))
        setup_offset = len(seek_table) + (2 + len(packets[0]) if packets else 0)
        struct.pack_into("<I", fmt_buf, 0x1C, setup_offset)
        struct.pack_into("<I", fmt_buf, 0x2C, setup_offset)
        # largest *audio* packet: the paired build writes 1 for an all-silent
        # stream whose setup packet is 215 bytes, so the setup is not counted
        if len(packets) > 1:
            struct.pack_into("<H", fmt_buf, 0x30, max(len(packet) for packet in packets[1:]))
        fmt_payload = bytes(fmt_buf)
    chunks: list[tuple[bytes, bytes]] = [(b"fmt ", fmt_payload)]
    if extra_chunks:
        chunks.extend(extra_chunks)
    chunks.append((b"data", data_payload))
    return build_riff(chunks, endian=endian)


def build_pcm_wem(
    fmt_fields: dict,
    pcm_data: bytes,
    *,
    endian: str = "le",
    junk: bytes | None = b"\x00\x00\x00\x00",
    extra_chunks: list[tuple[bytes, bytes]] | None = None,
    fmt_raw: bytes | None = None,
) -> bytes:
    """Build complete PCM WEM bytes."""
    fmt_payload = fmt_raw if fmt_raw is not None else pack_pcm_ext_fmt(fmt_fields)
    chunks: list[tuple[bytes, bytes]] = [(b"fmt ", fmt_payload)]
    if junk is not None:
        chunks.append((b"JUNK", junk))
    if extra_chunks:
        chunks.extend(extra_chunks)
    chunks.append((b"data", pcm_data))
    return build_riff(chunks, endian=endian)


def load_wem_parts_bytes(
    raw: bytes, *, file: str = "<bytes>", path: str = "<bytes>"
) -> dict:
    """Load structural WEM parts from bytes without filesystem I/O."""
    endian, chunks = parse_chunks(raw)
    fmt_chunk = next((chunk for chunk in chunks if chunk["id"] == "fmt "), None)
    data_chunk = next((chunk for chunk in chunks if chunk["id"] == "data"), None)
    if not fmt_chunk or not data_chunk:
        raise ValueError(f"{path}: missing fmt or data")

    fmt_payload = fmt_chunk["payload"]
    tag = _u16(fmt_payload, 0)
    extras_before_data: list[tuple[bytes, bytes]] = []
    extras_after_data: list[tuple[bytes, bytes]] = []
    seen_fmt = False
    seen_data = False
    for chunk in chunks:
        if chunk["id"] == "fmt ":
            seen_fmt = True
            continue
        if chunk["id"] == "data":
            seen_data = True
            continue
        item = (chunk["id"].encode("ascii"), chunk["payload"])
        if seen_fmt and not seen_data:
            extras_before_data.append(item)
        else:
            extras_after_data.append(item)

    out: dict = {
        "path": path,
        "file": file,
        "endian": endian,
        "raw": raw,
        "fmt_raw": fmt_payload,
        "data_raw": data_chunk["payload"],
        "extras_before_data": extras_before_data,
        "extras_after_data": extras_after_data,
        "chunk_ids": [chunk["id"] for chunk in chunks],
    }

    if tag == WWISE_VORBIS_FORMAT_TAG:
        fmt = parse_vorbis_fmt(fmt_payload)
        extracted = extract_packets(
            data_chunk["payload"], fmt["dwSeekTableSize"], endian=endian
        )
        if not extracted["ok"]:
            raise ValueError(f"{path}: packet walk failed: {extracted}")
        out["kind"] = "wwise_vorbis"
        out["fmt"] = fmt
        out["seek_table"] = extracted["seek_table"]
        out["packets"] = extracted["packets"]
        out["sizes"] = extracted["sizes"]
    elif tag in (WWISE_PCM_EXT_FORMAT_TAG, WWISE_PCM_FORMAT_TAG):
        out["kind"] = (
            "pcm_extensible" if tag == WWISE_PCM_EXT_FORMAT_TAG else "pcm_plain"
        )
        out["fmt"] = parse_pcm_ext_fmt(fmt_payload)
        out["pcm_data"] = data_chunk["payload"]
    else:
        out["kind"] = f"unknown_{hex(tag)}"
        out["fmt"] = parse_pcm_ext_fmt(fmt_payload)
        out["pcm_data"] = data_chunk["payload"]
    return out
