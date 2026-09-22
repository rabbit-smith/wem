"""Per-frame values of the pure-Python oracle, as a JSON-line stream on stdout.

This program is the oracle side of the live per-frame parity suites.  The Rust
suite (``crates/wem-core/tests/frame_pipeline_parity.rs``) spawns it and
compares the native kernel's own per-frame results against the records it
prints; ``tests/parity/test_frame_pipeline_parity.py`` imports
:func:`frame_records` for the fields reachable through the shipped binding.

Nothing here is a recorded expectation: every value is produced by the oracle
at the moment a test asks for it, from one committed input
(``tests/fixtures/input.wav``), by the same pipeline the whole-file parity
suite drives (``wwise_wem_reference.python_engine``): condition the PCM, select
the mode/window plan, then per frame analyze and pack.

Stream shape — one JSON document per line, header first, then one document per
audio frame in encoder order:

* header: source path and digest, profile label, the resolved setup packet,
  geometry, frame count, and which stages/frames the frame documents carry;
* frame: the scheduling fields (``mode``, ``transition``, ``window_center``),
  the float stage rows, the floor posts, the quantized residue rows, and the
  packed audio packet.

Row lists are channel-major.  Every numeric row is one string of 8-digit
hexadecimal words: the IEEE-754 float32 bit pattern for the float stages, the
two's-complement int32 pattern for posts and residue.  Words, not digests and
not decimal reprs — a consumer compares value against value and can name the
first differing channel and bin.  ``null`` marks a floor-zero channel's posts;
``stages`` is ``null`` for a frame whose stage rows were projected out.

``--stages`` and ``--frames`` exist for consumers that do not need the whole
stream (the per-frame x per-stage comparison is the same channel with more
stage names, and projecting frames keeps that stream bounded).
"""

from __future__ import annotations

import argparse
import hashlib
import json
import os
import struct
import sys
from pathlib import Path
from typing import Any, Collection, Iterator, Sequence

from wwise_wem import WwiseProfile, WwiseVersion
from wwise_wem.adapters.wav import read_pcm_wav
from wwise_wem_reference.profiles.artifact import resolve_selection
from wwise_wem_reference.analysis.session import AnalysisSession
from wwise_wem_reference.profiles.assembly import assemble_encoder_profile_resources
from wwise_wem_reference.vorbis.packet_encoder import pack_analysis_frame

ROOT = Path(__file__).resolve().parents[2]
FIXTURE = ROOT / "tests" / "fixtures" / "input.wav"

SCHEMA = "oracle-frame-values.v1"

#: The float analysis stages, in pipeline order (the names the per-frame
#: comparison reports a divergence under).
STAGES: tuple[str, ...] = (
    "window",
    "coefficients",
    "raw_mdct",
    "fft",
    "remap",
    "seed",
    "post",
    "side",
)


def _float_words(row: Sequence[float]) -> str:
    """One float row as concatenated float32 bit-pattern words."""
    words = struct.unpack(f"<{len(row)}I", struct.pack(f"<{len(row)}f", *row))
    return "".join(f"{word:08x}" for word in words)


def _int_words(row: Sequence[int]) -> str:
    """One integer row as concatenated two's-complement int32 words."""
    return "".join(f"{int(value) & 0xFFFFFFFF:08x}" for value in row)


def _stage_rows(analysis: Any, window: Any) -> dict[str, list[str]]:
    """The float stage rows of one analyzed frame, channel-major."""
    return {
        "window": [_float_words(row) for row in window.samples],
        "coefficients": [_float_words(row) for row in analysis.coefficients],
        "raw_mdct": [_float_words(row) for row in analysis.raw_mdct],
        "fft": [_float_words(row) for row in analysis.fft],
        "remap": [_float_words(row) for row in analysis.remap],
        "seed": [_float_words(row) for row in analysis.seed],
        "post": [_float_words(row) for row in analysis.post],
        "side": [_float_words(row) for row in analysis.side],
    }


def frame_records(
    wav: Path = FIXTURE,
    *,
    stages: Collection[str] = STAGES,
    frames: Collection[int] | None = None,
) -> Iterator[dict[str, Any]]:
    """Yield the header record, then one record per audio frame of ``wav``.

    ``stages`` names the float stages carried per frame (empty to project them
    out); ``frames`` limits which frames carry them (``None`` means every
    frame).  The scheduling fields, floor posts, residue rows, and packet are
    always emitted, so a projected stream still compares every frame.
    """
    pcm = read_pcm_wav(wav)
    profile = resolve_selection(
        WwiseProfile(WwiseVersion.DEFAULT, pcm.channel_count, pcm.sample_rate)
    )
    resources = assemble_encoder_profile_resources(
        profile, setup_packet=profile.setup_packet
    )
    session = AnalysisSession(
        pcm.channel_count,
        sample_rate=pcm.sample_rate,
        blocksizes=profile.block_sizes,
        resources=resources.analysis,
    )
    conditioned = session.condition_pcm(pcm.channels)
    modes, windows = session.selected_windows(conditioned)

    frames_with_stages = (
        list(range(len(modes))) if frames is None else sorted(set(frames))
    )
    yield {
        "kind": "header",
        "schema": SCHEMA,
        "wav": _display_path(wav),
        "input_sha256": hashlib.sha256(Path(wav).read_bytes()).hexdigest(),
        "profile": profile.label(),
        "channels": pcm.channel_count,
        "sample_rate": pcm.sample_rate,
        "pcm_frames": pcm.frame_count,
        "audio_packets": len(modes),
        "stages": list(stages),
        "stage_frames": frames_with_stages,
        # The Wwise setup packet the resolved profile carries: the container's
        # first packet, and the half of the profile identity a value consumer
        # cannot derive from the per-frame rows.
        "setup_packet": bytes(resources.setup_packet).hex(),
    }

    emitted = 0
    for index, window in enumerate(windows):
        analysis = session.analyze_window(window)
        encoded = pack_analysis_frame(
            resources.setup, resources.codebooks, analysis, channels=pcm.channel_count
        )
        yield {
            "kind": "frame",
            "index": index,
            "mode": int(window.current),
            "transition": [int(window.previous), int(window.current), int(window.following)],
            "window_center": int(window.center),
            "stages": (
                {
                    name: rows
                    for name, rows in _stage_rows(analysis, window).items()
                    if name in stages
                }
                if stages and (frames is None or index in frames_with_stages)
                else None
            ),
            "posts": [
                None if row is None else _int_words(row) for row in encoded.posts
            ],
            "residue_q": [_int_words(row) for row in encoded.quantized_residue],
            "packet": bytes(encoded.packet).hex(),
        }
        emitted += 1
    if emitted != len(modes):
        raise AssertionError(
            f"window iterator yielded {emitted} frames, mode selection {len(modes)}"
        )


def _display_path(path: Path) -> str:
    """``path`` relative to the repository root when it lives under it."""
    resolved = Path(path).resolve()
    try:
        return resolved.relative_to(ROOT).as_posix()
    except ValueError:
        return str(resolved)


def _stage_selection(spec: str) -> tuple[str, ...]:
    if spec == "all":
        return STAGES
    if spec == "none":
        return ()
    names = tuple(part.strip() for part in spec.split(",") if part.strip())
    unknown = sorted(set(names) - set(STAGES))
    if unknown:
        raise SystemExit(f"unknown stages {unknown}; known stages: {list(STAGES)}")
    return names


def _frame_selection(spec: str) -> list[int] | None:
    if spec == "all":
        return None
    try:
        return sorted({int(part) for part in spec.split(",") if part.strip()})
    except ValueError as error:
        raise SystemExit(f"--frames takes 'all' or a comma-separated index list: {error}")


def main(argv: Sequence[str] | None = None) -> int:
    parser = argparse.ArgumentParser(description="Per-frame oracle values (JSON lines)")
    parser.add_argument("--wav", type=Path, default=FIXTURE)
    parser.add_argument(
        "--stages",
        default="all",
        help="'all', 'none', or a comma-separated subset of " + ",".join(STAGES),
    )
    parser.add_argument(
        "--frames",
        default="all",
        help="frames whose stage rows are emitted: 'all' or a comma-separated index list",
    )
    args = parser.parse_args(argv)
    try:
        for record in frame_records(
            args.wav,
            stages=_stage_selection(args.stages),
            frames=_frame_selection(args.frames),
        ):
            sys.stdout.write(json.dumps(record, separators=(",", ":")) + "\n")
    except BrokenPipeError:
        # The consumer stopped reading, which it only does when it has already
        # found a difference; leaving the pipe quietly keeps that report clean.
        os.dup2(os.open(os.devnull, os.O_WRONLY), sys.stdout.fileno())
        return 0
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
