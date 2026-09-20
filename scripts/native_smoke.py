#!/usr/bin/env python3
"""P3-1 smoke test for the PyO3 abi3 binding `wwise_wem._core`.

The in-package extension is the facade's single execution path; this
smoke proves, from the Python side:
  1. the streaming path (StreamSession: start -> 5 uneven chunks -> finish)
     reproduces tests/fixtures/reference.wem byte-for-byte;
  2. the one-shot path (Encoder.encode_pcm, list-of-lists and memoryview
     forms) reproduces the same reference byte-for-byte;
  3. the error surface maps wem-core rejections to WemEncoderError codes.

Usage:
  .venv/bin/python scripts/native_smoke.py

Exit code 0 on success, 1 on any failure.
"""

from __future__ import annotations

import hashlib
import struct
import sys
from pathlib import Path

REPO = Path(__file__).resolve().parents[1]
FIXTURES = REPO / "tests" / "fixtures"
PROFILE_NAME = "wwise2013-6ch-44100"
SETUP_SHA256 = (
    "3ef56cbd6e6a66a5474005db05912624487faa555fb2cdfed130f606b322e4e3"
)
EXPECTED_WEM_SHA256 = (
    "17851d26c6210b85e498ae0452d2562d7b9e2c3e9e795c459656b9c9d8d35247"
)


def read_pcm16_interleaved(path: Path) -> tuple[int, int, bytes]:
    """Minimal RIFF/PCM-16 reader: (sample_rate, channels, interleaved bytes)."""
    raw = path.read_bytes()
    if raw[:4] != b"RIFF" or raw[8:12] != b"WAVE":
        raise ValueError(f"{path} is not a RIFF/WAVE file")
    pos = 12
    sample_rate = channels = None
    data: bytes | None = None
    while pos + 8 <= len(raw):
        chunk_id = raw[pos : pos + 4]
        size = int.from_bytes(raw[pos + 4 : pos + 8], "little")
        body = raw[pos + 8 : pos + 8 + size]
        if chunk_id == b"fmt ":
            channels = int.from_bytes(body[2:4], "little")
            sample_rate = int.from_bytes(body[4:8], "little")
        elif chunk_id == b"data":
            data = body
        pos += 8 + size + (size & 1)  # RIFF chunks are word-aligned
    if data is None or sample_rate is None or channels is None:
        raise ValueError(f"{path} is not a PCM-16 WAV with a data chunk")
    return sample_rate, channels, data


def sha256_hex(data: bytes) -> str:
    return hashlib.sha256(data).hexdigest()


def main() -> int:
    import wwise_wem._core as native

    print(f"module: {native.__file__}")

    # --- inputs -----------------------------------------------------------
    sample_rate, channels, interleaved = read_pcm16_interleaved(
        FIXTURES / "input.wav"
    )
    total_frames = len(interleaved) // (2 * channels)
    reference = (FIXTURES / "reference.wem").read_bytes()
    ref_sha = sha256_hex(reference)
    print(
        f"input.wav: {channels}ch / {sample_rate}Hz / {total_frames} frames "
        f"({len(interleaved)} PCM bytes)"
    )
    print(f"reference.wem: {len(reference)} bytes sha256={ref_sha}")
    if ref_sha != EXPECTED_WEM_SHA256:
        print("FAIL: reference.wem fixture drifted")
        return 1

    failures: list[str] = []

    def check(label: str, condition: bool) -> None:
        print(f"{'PASS' if condition else 'FAIL'}: {label}")
        if not condition:
            failures.append(label)

    # --- 1) streaming path: 5 uneven chunks -------------------------------
    cuts = [0, 17000, 40500, 70600, 85600, total_frames]
    step = 2 * channels
    session = native.StreamSession()
    session.start(SETUP_SHA256, name=PROFILE_NAME)
    packets = []
    chunk_sizes = []
    for i in range(len(cuts) - 1):
        lo, hi = cuts[i] * step, cuts[i + 1] * step
        chunk_sizes.append(hi - lo)
        packets.extend(session.push(interleaved[lo:hi]))
    complete = session.finish()
    wem_bytes = bytes(complete.bytes)
    print(f"chunk pattern (bytes): {chunk_sizes}")
    print(f"packets emitted before/at finish: {len(packets)} (seq 0..{len(packets)-1})")
    print(f"complete.sha256: {complete.sha256}")
    check("stream complete.sha256 matches reference", complete.sha256 == ref_sha)
    check("stream bytes equal reference.wem", wem_bytes == reference)
    check("complete.total_len matches", complete.total_len == len(reference))
    seqs = [p.seq for p in packets]
    check(
        "packet seq is 0..n-1 in emission order",
        seqs == list(range(len(packets))),
    )
    # The emitted packet stream is a strict prefix of the WEM's packet
    # sequence (the kernel withholds the end-of-stream tail until finish),
    # and seq 0 is always the setup packet.
    setup = bytes(packets[0].data) if packets else b""
    check(
        "seq 0 packet is embedded in the reference container",
        setup != b"" and setup in reference,
    )

    # --- 2) one-shot path: list of lists ----------------------------------
    values = struct.unpack(f"<{total_frames * channels}h", interleaved)
    rows = [
        [values[f * channels + c] for f in range(total_frames)]
        for c in range(channels)
    ]
    result = native.Encoder(PROFILE_NAME).encode_pcm(sample_rate, rows)
    print(f"encode_pcm(lists).sha256(): {result.sha256()}")
    check("encode_pcm(lists) sha256 matches reference", result.sha256() == ref_sha)
    check("encode_pcm(lists) bytes equal reference.wem", bytes(result.data) == reference)
    check("stats.audio_packets", result.audio_packets == 205)
    check("stats.bytes_out", result.bytes_out == len(reference))
    check("stats.pcm_frames", result.pcm_frames == total_frames)
    check("stats.channels", result.channels == channels)
    check("stats.metadata_source", result.metadata_source == f"profile:{PROFILE_NAME}")

    # --- 3) one-shot path: 2-D signed-16 memoryview ------------------------
    cm_bytes = b"".join(
        struct.pack("<h", v) for v in (values[f * channels + c] for c in range(channels) for f in range(total_frames))
    )
    mv = memoryview(cm_bytes).cast("h", [channels, total_frames])
    result_mv = native.Encoder(PROFILE_NAME).encode_pcm(sample_rate, mv)
    print(f"encode_pcm(memoryview).sha256(): {result_mv.sha256()}")
    check("encode_pcm(memoryview) sha256 matches reference", result_mv.sha256() == ref_sha)
    check("encode_pcm(memoryview) bytes equal reference.wem", bytes(result_mv.data) == reference)

    # --- 4) error mapping --------------------------------------------------
    try:
        native.Encoder("definitely-not-installed")
    except native.WemEncoderError as e:
        check(
            "unknown profile -> PROFILE_NOT_FOUND",
            e.code == "PROFILE_NOT_FOUND" and "definitely-not-installed" in str(e),
        )
    else:
        check("unknown profile -> PROFILE_NOT_FOUND", False)

    s = native.StreamSession()
    try:
        s.push(b"\x00" * 12)
    except native.WemEncoderError as e:
        check("push before start -> STATE_ERROR", e.code == "STATE_ERROR")
    else:
        check("push before start -> STATE_ERROR", False)

    enc = native.Encoder(PROFILE_NAME)
    try:
        enc.encode_pcm(sample_rate, [[0] * 100 for _ in range(channels)])
    except native.WemEncoderError as e:
        check("short stream -> INPUT_TOO_SHORT", e.code == "INPUT_TOO_SHORT")
    else:
        check("short stream -> INPUT_TOO_SHORT", False)

    # --- verdict -----------------------------------------------------------
    if failures:
        print(f"\nFAIL: {len(failures)} check(s) failed: {failures}")
        return 1
    print("\nPASS: all native smoke checks green (byte-exact vs reference.wem)")
    return 0


if __name__ == "__main__":
    sys.exit(main())
