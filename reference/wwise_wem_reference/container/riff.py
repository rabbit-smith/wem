"""Pure RIFF/RIFX WAVE chunk parsing and construction."""

from __future__ import annotations

import struct


def parse_chunks(raw: bytes) -> tuple[str, list[dict]]:
    """Parse RIFF/RIFX chunks while preserving the legacy permissive walk.

    The declared RIFF extent is clamped to the supplied bytes. A chunk whose
    declared payload extends past that extent is returned with its declared
    ``size`` and the available payload bytes, matching the historical parser.
    """
    if raw[0:4] not in (b"RIFF", b"RIFX"):
        raise ValueError("not RIFF/RIFX")
    le = raw[0:4] == b"RIFF"
    endian = "<" if le else ">"
    rsize = struct.unpack_from(endian + "I", raw, 4)[0]
    end = min(8 + rsize, len(raw))
    pos = 12
    chunks: list[dict] = []
    while pos + 8 <= end:
        cid = raw[pos : pos + 4].decode("ascii", "replace")
        csize = struct.unpack_from(endian + "I", raw, pos + 4)[0]
        payload = raw[pos + 8 : pos + 8 + csize]
        chunks.append({"id": cid, "size": csize, "off": pos, "payload": payload})
        pos += 8 + csize + (csize & 1)
    return ("le" if le else "be"), chunks


def build_riff(
    chunks: list[tuple[bytes, bytes]],
    *,
    endian: str = "le",
    pad_final: bool = False,
) -> bytes:
    """Build a RIFF/RIFX WAVE byte string from ``(fourcc, payload)`` chunks.

    Odd intermediate payloads are word-padded. Wwise 2013 references commonly
    omit the pad after the final chunk, so that remains the default; pass
    ``pad_final=True`` for a strict final pad.
    """
    if endian not in ("le", "be"):
        raise ValueError(endian)
    riff_id = b"RIFF" if endian == "le" else b"RIFX"
    size_fmt = "<I" if endian == "le" else ">I"
    body = bytearray(b"WAVE")
    n = len(chunks)
    for i, (cid, payload) in enumerate(chunks):
        if len(cid) != 4:
            raise ValueError(f"bad chunk id {cid!r}")
        body += cid
        body += struct.pack(size_fmt, len(payload))
        body += payload
        is_last = i == n - 1
        if (len(payload) & 1) and (not is_last or pad_final):
            body += b"\x00"
    out = bytearray(riff_id)
    out += struct.pack(size_fmt, len(body))
    out += body
    return bytes(out)
