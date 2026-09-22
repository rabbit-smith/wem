#!/usr/bin/env python3
"""Parse Wwise Vorbis *audio* packets (post-setup).

Wwise 2013.2 vs raw Ogg Vorbis I:
  - No packet-type bit (always audio); first field is mode number
  - mode uses ilog(nmodes-1) bits
  - long packet headers do not serialize Vorbis-I prev/next-window fields
  - floor1 / residue follow standard Vorbis I body layout
  - a 1-byte 0x01 packet is a complete long-mode all-zero-floor packet

"""
from __future__ import annotations

import struct
from typing import Any

from .bitio import BitReader
from .codebook import Codebook
from .setup import ilog
from .residue import consume_residue

# floor1 multiplier → quant range (Vorbis I)
FLOOR1_RANGES = {1: 256, 2: 128, 3: 86, 4: 64}


def extract_packets(data: bytes, seek_table_size: int = 0) -> list[bytes]:
    packets: list[bytes] = []
    p = seek_table_size
    while p + 2 <= len(data):
        sz = struct.unpack_from("<H", data, p)[0]
        if p + 2 + sz > len(data):
            raise ValueError(f"truncated packet at {p}: size={sz}")
        packets.append(data[p + 2 : p + 2 + sz])
        p += 2 + sz
    if p != len(data):
        raise ValueError(f"trailing bytes after packets: {len(data) - p}")
    return packets


def parse_audio_header(br: BitReader, setup: dict) -> dict:
    nmodes = setup["nmodes"]
    mode_bits = ilog(nmodes - 1) if nmodes > 1 else 0
    start = br.tell_bits()
    mode_num = br.read(mode_bits) if mode_bits else 0
    if mode_num >= nmodes:
        raise ValueError(f"mode {mode_num} out of range 0..{nmodes - 1}")
    mode = setup["modes"][mode_num]
    # The scheduler carries previous/current/following block geometry to the
    # PCM window path. This packet revision serializes only ``mode_num``;
    # interpreting the next two floor flags as Vorbis-I window bits
    # misaligns all long packets.
    prev_window = next_window = None
    mapping = setup["maps"][mode["mapping"]]
    return {
        "mode": mode_num,
        "blockflag": mode["blockflag"],
        "mapping": mode["mapping"],
        "prev_window": prev_window,
        "next_window": next_window,
        "bit_start": start,
        "bit_end": br.tell_bits(),
        "submaps": mapping["submaps"],
        "floor_ids": list(mapping["floors"]),
        "residue_ids": list(mapping["residues"]),
        "chmux": list(mapping["chmux"]),
    }


def decode_floor1_body(
    br: BitReader, floor: dict, books: list[Codebook]
) -> list[int]:
    """Decode one channel's floor1 Y posts (nonzero already true)."""
    rng = FLOOR1_RANGES[floor["multiplier"]]
    ybits = ilog(rng - 1)
    nvals = 2 + len(floor["x_list"])
    Y = [0] * nvals
    Y[0] = br.read(ybits)
    Y[1] = br.read(ybits)
    ppos = 2
    for p in range(floor["partitions"]):
        cl = floor["partition_classes"][p]
        cdim = floor["class_dims"][cl]
        cbits = floor["class_subs"][cl]
        csub = (1 << cbits) - 1
        if cbits:
            cval = books[floor["class_masterbooks"][cl]].decode(br)
        else:
            cval = 0
        for _j in range(cdim):
            book = floor["subclass_books"][cl][cval & csub]
            cval >>= cbits
            if book >= 0:
                Y[ppos] = books[book].decode(br)
            else:
                Y[ppos] = 0
            ppos += 1
    if ppos != nvals:
        raise ValueError(f"floor1 Y count {ppos} != {nvals}")
    return Y


def decode_floors_for_packet(
    br: BitReader,
    setup: dict,
    books: list[Codebook],
    channels: int,
    mapping: dict,
) -> dict:
    """Per-channel floor nonzero + optional floor1 body."""
    nonzero: list[int] = []
    curves: list[list[int] | None] = []
    for ch in range(channels):
        if br.bits_left() < 1:
            return {
                "complete": False,
                "nonzero": nonzero,
                "curves": curves,
                "error": f"EOF at floor nonzero ch={ch}",
                "bit_pos": br.tell_bits(),
            }
        sub = mapping["chmux"][ch] if mapping["submaps"] > 1 else 0
        floor = setup["floors"][mapping["floors"][sub]]
        nz = br.read(1)
        nonzero.append(nz)
        if nz:
            try:
                curves.append(decode_floor1_body(br, floor, books))
            except (EOFError, ValueError) as e:
                return {
                    "complete": False,
                    "nonzero": nonzero,
                    "curves": curves,
                    "error": f"floor body ch={ch}: {e}",
                    "bit_pos": br.tell_bits(),
                }
        else:
            curves.append(None)
    return {
        "complete": True,
        "nonzero": nonzero,
        "curves": curves,
        "bit_pos": br.tell_bits(),
    }


def parse_audio_packet(
    payload: bytes,
    setup: dict,
    books: list[Codebook],
    channels: int,
) -> dict:
    br = BitReader(payload)
    try:
        hdr = parse_audio_header(br, setup)
    except (EOFError, ValueError) as e:
        return {
            "packet_size": len(payload),
            "complete": False,
            "error": f"header: {e}",
            "stub": len(payload) <= 1,
        }
    mapping = setup["maps"][hdr["mapping"]]
    floors = decode_floors_for_packet(br, setup, books, channels, mapping)
    out: dict[str, Any] = {
        "packet_size": len(payload),
        "header": hdr,
        "floors": {
            "nonzero": floors.get("nonzero"),
            "n_curves": sum(1 for c in floors.get("curves") or [] if c is not None),
            "bit_pos": floors.get("bit_pos"),
            "complete": floors.get("complete"),
            "error": floors.get("error"),
            # keep first channel curve head for debug (avoid huge JSON)
            "y0_head": (
                floors["curves"][0][:6]
                if floors.get("curves") and floors["curves"][0]
                else None
            ),
        },
        "bits_after_floor": br.tell_bits(),
        "bits_left": br.bits_left(),
        "stub": len(payload) <= 1 and not floors.get("complete"),
        "complete": bool(floors.get("complete")),
    }
    if not floors.get("complete"):
        out["error"] = floors.get("error")
        return out

    # residue (eop-tolerant)
    ch_used = [bool(z) for z in floors.get("nonzero") or []]
    n_spec = (2048 if hdr["blockflag"] else 256) // 2
    # one submap in the reference WEM; multi-submap: run each residue id once
    residues_out = []
    for rid in hdr["residue_ids"]:
        residues_out.append(
            consume_residue(
                br,
                setup["residues"][rid],
                books,
                ch_used,
                n_spectrum=n_spec,
            )
        )
    out["residues"] = residues_out
    out["bits_after_residue"] = br.tell_bits()
    out["bits_left"] = br.bits_left()
    return out
