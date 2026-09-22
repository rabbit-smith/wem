#!/usr/bin/env python3
"""P3-1 smoke test for the PyO3 abi3 binding `wwise_wem._core`.

The in-package extension is the facade's single execution path; this
smoke proves, from the Python side:
  1. the streaming path (StreamSession.for_selection -> 5 uneven chunks ->
     finish) reproduces tests/fixtures/reference.wem byte-for-byte;
  2. the one-shot path (Encoder.encode_pcm, list-of-lists and memoryview
     forms) reproduces the same reference byte-for-byte;
  3. the error surface maps wem-core rejections to WemEncoderError codes.

Usage:
  PYTHONPATH=src .venv/bin/python scripts/native_smoke.py

Exit code 0 on success, 1 on any failure.
"""

from __future__ import annotations

import struct
import sys
from pathlib import Path

from wwise_wem import WwiseProfile, WwiseVersion

REPO = Path(__file__).resolve().parents[1]
FIXTURES = REPO / "tests" / "fixtures"
SELECTION = WwiseProfile(WwiseVersion.WWISE2013, 6, 44100)
UNINSTALLED_SELECTION = WwiseProfile(WwiseVersion.WWISE2013, 2, 44100)
PROFILE_METADATA_SOURCE = "profile:6ch/44100Hz/2013"


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


def first_wem_packet(raw: bytes) -> bytes:
    """The first packet of a Wwise WEM: one u16 length-prefixed packet.

    The container's data chunk is `[u16 size][packet]...`; the seq 0 packet is
    therefore readable straight out of the file without a full packet walk.
    """
    if raw[:4] != b"RIFF" or raw[8:12] != b"WAVE":
        raise ValueError("not a RIFF/WAVE container")
    pos = 12
    while pos + 8 <= len(raw):
        chunk_id = raw[pos : pos + 4]
        size = int.from_bytes(raw[pos + 4 : pos + 8], "little")
        if chunk_id == b"data":
            start = pos + 8
            packet_size = int.from_bytes(raw[start : start + 2], "little")
            return raw[start + 2 : start + 2 + packet_size]
        pos += 8 + size + (size & 1)
    raise ValueError("container has no data chunk")


def main() -> int:
    import wwise_wem._core as native

    print(f"module: {native.__file__}")

    # --- inputs -----------------------------------------------------------
    sample_rate, channels, interleaved = read_pcm16_interleaved(
        FIXTURES / "input.wav"
    )
    total_frames = len(interleaved) // (2 * channels)
    reference = (FIXTURES / "reference.wem").read_bytes()
    print(
        f"input.wav: {channels}ch / {sample_rate}Hz / {total_frames} frames "
        f"({len(interleaved)} PCM bytes)"
    )
    print(f"reference.wem: {len(reference)} bytes")

    failures: list[str] = []

    def check(label: str, condition: bool) -> None:
        print(f"{'PASS' if condition else 'FAIL'}: {label}")
        if not condition:
            failures.append(label)

    # --- 1) streaming path: 5 uneven chunks -------------------------------
    cuts = [0, 17000, 40500, 70600, 85600, total_frames]
    step = 2 * channels
    session = native.StreamSession.for_selection(SELECTION)
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
    # The setup packet is a recorded artifact twice over: the compiled
    # profile carries it, and the reference container carries it as its first
    # length-prefixed packet. Compare bytes against both — no digest stands in
    # for either.
    from wwise_wem_reference.profiles.artifact import resolve_selection

    check(
        "seq 0 packet reproduces the selected profile's setup resource",
        setup == resolve_selection(SELECTION).setup_packet,
    )
    check(
        "seq 0 packet is the reference container's first packet",
        setup != b"" and setup == first_wem_packet(reference),
    )

    # --- 2) one-shot path: list of lists ----------------------------------
    values = struct.unpack(f"<{total_frames * channels}h", interleaved)
    rows = [
        [values[f * channels + c] for f in range(total_frames)]
        for c in range(channels)
    ]
    result = native.Encoder(SELECTION).encode_pcm(sample_rate, rows)
    print(f"encode_pcm(lists).sha256(): {result.sha256()}")
    check("encode_pcm(lists) bytes equal reference.wem", bytes(result.data) == reference)
    check("stats.audio_packets", result.audio_packets == 205)
    check("stats.bytes_out", result.bytes_out == len(reference))
    check("stats.pcm_frames", result.pcm_frames == total_frames)
    check("stats.channels", result.channels == channels)
    check(
        "stats.metadata_source",
        result.metadata_source == PROFILE_METADATA_SOURCE,
    )

    # --- 3) one-shot path: 2-D signed-16 memoryview ------------------------
    cm_bytes = b"".join(
        struct.pack("<h", v) for v in (values[f * channels + c] for c in range(channels) for f in range(total_frames))
    )
    mv = memoryview(cm_bytes).cast("h", [channels, total_frames])
    result_mv = native.Encoder(SELECTION).encode_pcm(sample_rate, mv)
    print(f"encode_pcm(memoryview).sha256(): {result_mv.sha256()}")
    check("encode_pcm(memoryview) bytes equal reference.wem", bytes(result_mv.data) == reference)

    # --- 4) error mapping --------------------------------------------------
    try:
        native.Encoder(UNINSTALLED_SELECTION)
    except native.WemEncoderError as e:
        check(
            "unsatisfiable selection -> PROFILE_NOT_FOUND",
            e.code == "PROFILE_NOT_FOUND" and "2ch/44100Hz" in str(e),
        )
    else:
        check("unsatisfiable selection -> PROFILE_NOT_FOUND", False)

    s = native.StreamSession()
    try:
        s.push(b"\x00" * 12)
    except native.WemEncoderError as e:
        check("push before open -> STATE_ERROR", e.code == "STATE_ERROR")
    else:
        check("push before open -> STATE_ERROR", False)

    enc = native.Encoder(SELECTION)
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
