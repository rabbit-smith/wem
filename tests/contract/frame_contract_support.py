"""Generate and compare the checked per-frame encoder contract.

The manifest deliberately stores hashes rather than analysis buffers.  It is
small enough to review and commit while still identifying the first encoder
stage that diverges on any of the 205 acceptance frames.
"""
from __future__ import annotations

import argparse
import hashlib
import json
import struct
from pathlib import Path
from typing import Any, Iterable, Sequence

from wwise_wem.vorbis.packet_encoder import pack_analysis_frame
from wwise_wem.profiles.assembly import assemble_encoder_profile_resources
from wwise_wem.profiles.bundle import load_profile_bundle
from wwise_wem.application.compat import read_pcm16_wav
from wwise_wem.analysis.session import AnalysisSession
from wwise_wem.profiles.registry import load_wem_profile, resolve_wem_profile


SCHEMA = "wwise-wem.frame-contract.v1"
FRAME_FIELDS = (
    "mode",
    "transition",
    "window_center",
    "window_sha256",
    "coefficients_sha256",
    "raw_mdct_sha256",
    "fft_sha256",
    "remap_sha256",
    "seed_sha256",
    "post_sha256",
    "side_sha256",
    "floor_posts_sha256",
    "residue_q_after_sha256",
    "packet_size",
    "packet_sha256",
)


def _hash_float_rows(rows: Iterable[Iterable[float]]) -> str:
    digest = hashlib.sha256()
    for row in rows:
        for value in row:
            digest.update(struct.pack("<f", float(value)))
    return digest.hexdigest()


def _hash_int_rows(rows: Iterable[Sequence[int] | None]) -> str:
    digest = hashlib.sha256()
    for row in rows:
        if row is None:
            digest.update(b"\xff")
            continue
        digest.update(b"\x00")
        for value in row:
            digest.update(struct.pack("<i", int(value)))
    return digest.hexdigest()


def _numeric_word(value: float | int, word_kind: str) -> int:
    if word_kind == "float32":
        return struct.unpack("<I", struct.pack("<f", float(value)))[0]
    if word_kind == "int32":
        return int(value) & 0xFFFFFFFF
    raise ValueError("word_kind must be 'float32' or 'int32'")


def first_nested_word_difference(
    expected: Sequence[Sequence[Sequence[float | int]]],
    actual: Sequence[Sequence[Sequence[float | int]]],
    *,
    word_kind: str = "float32",
) -> dict[str, Any] | None:
    """Locate the first differing frame/channel/bin numeric word.

    Comparing packed words, rather than Python numeric equality, preserves
    float32 rounding and signed-zero differences from dual-run buffers.
    """
    _numeric_word(0, word_kind)  # validate even for two empty inputs
    frame_count = max(len(expected), len(actual))
    for frame in range(frame_count):
        expected_frame = expected[frame] if frame < len(expected) else ()
        actual_frame = actual[frame] if frame < len(actual) else ()
        channel_count = max(len(expected_frame), len(actual_frame))
        for channel in range(channel_count):
            expected_channel = (
                expected_frame[channel] if channel < len(expected_frame) else ()
            )
            actual_channel = (
                actual_frame[channel] if channel < len(actual_frame) else ()
            )
            bin_count = max(len(expected_channel), len(actual_channel))
            for bin_index in range(bin_count):
                expected_value = (
                    expected_channel[bin_index]
                    if bin_index < len(expected_channel)
                    else None
                )
                actual_value = (
                    actual_channel[bin_index]
                    if bin_index < len(actual_channel)
                    else None
                )
                expected_word = (
                    _numeric_word(expected_value, word_kind)
                    if expected_value is not None
                    else None
                )
                actual_word = (
                    _numeric_word(actual_value, word_kind)
                    if actual_value is not None
                    else None
                )
                if expected_word != actual_word:
                    return {
                        "frame": frame,
                        "channel": channel,
                        "bin": bin_index,
                        "expected": expected_value,
                        "actual": actual_value,
                        "expected_word": (
                            f"0x{expected_word:08x}" if expected_word is not None else None
                        ),
                        "actual_word": (
                            f"0x{actual_word:08x}" if actual_word is not None else None
                        ),
                    }
    return None


def build_frame_contract(wav: Path, *, profile: str | None = None) -> dict[str, Any]:
    """Run the real encoder and return its compact per-frame contract."""
    sample_rate, pcm_frames, pcm = read_pcm16_wav(Path(wav))
    channels = len(pcm)
    selected = (
        load_wem_profile(profile)
        if profile is not None
        else resolve_wem_profile(channels, sample_rate)
    )
    setup_packet = selected.setup_packet()
    resources = assemble_encoder_profile_resources(
        load_profile_bundle(profile=selected.name, verify_all=False),
        setup_packet=setup_packet,
    )
    setup = resources.setup
    books = resources.codebooks
    stream = AnalysisSession(
        channels,
        sample_rate=sample_rate,
        blocksizes=selected.block_sizes,
        resources=resources.analysis,
    )
    modes, windows = stream.selected_windows(pcm)

    frames: list[dict[str, Any]] = []
    packet_digest = hashlib.sha256()
    for index, window in enumerate(windows):
        analysis = stream.analyze_window(window)
        encoded = pack_analysis_frame(
            setup, books, analysis, channels=channels
        )
        packet_digest.update(encoded.packet)
        frames.append(
            {
                "index": index,
                "mode": window.current,
                "transition": [window.previous, window.current, window.following],
                "window_center": window.center,
                "window_sha256": _hash_float_rows(window.samples),
                "coefficients_sha256": _hash_float_rows(analysis.coefficients),
                "raw_mdct_sha256": _hash_float_rows(analysis.raw_mdct),
                "fft_sha256": _hash_float_rows(analysis.fft),
                "remap_sha256": _hash_float_rows(analysis.remap),
                "seed_sha256": _hash_float_rows(analysis.seed),
                "post_sha256": _hash_float_rows(analysis.post),
                "side_sha256": _hash_float_rows(analysis.side),
                "floor_posts_sha256": _hash_int_rows(encoded.posts),
                "residue_q_after_sha256": _hash_int_rows(
                    encoded.quantized_residue
                ),
                "packet_size": len(encoded.packet),
                "packet_sha256": hashlib.sha256(encoded.packet).hexdigest(),
            }
        )
    if len(frames) != len(modes):
        raise AssertionError("frame-contract window and mode counts diverged")
    return {
        "schema": SCHEMA,
        "profile": selected.name,
        "channels": channels,
        "sample_rate": sample_rate,
        "pcm_frames": pcm_frames,
        "input_sha256": hashlib.sha256(Path(wav).read_bytes()).hexdigest(),
        "audio_packets": len(frames),
        "packet_stream_sha256": packet_digest.hexdigest(),
        "frames": frames,
    }


def first_frame_contract_difference(
    expected: dict[str, Any], actual: dict[str, Any]
) -> dict[str, Any] | None:
    """Return a compact first-difference report, or ``None`` when identical."""
    for field in (
        "schema",
        "profile",
        "channels",
        "sample_rate",
        "pcm_frames",
        "input_sha256",
        "audio_packets",
    ):
        if expected.get(field) != actual.get(field):
            return {
                "frame": None,
                "field": field,
                "expected": expected.get(field),
                "actual": actual.get(field),
            }

    expected_frames = expected.get("frames", [])
    actual_frames = actual.get("frames", [])
    common = min(len(expected_frames), len(actual_frames))
    for index in range(common):
        expected_frame = expected_frames[index]
        actual_frame = actual_frames[index]
        if expected_frame.get("index") != actual_frame.get("index"):
            return {
                "frame": index,
                "field": "index",
                "expected": expected_frame.get("index"),
                "actual": actual_frame.get("index"),
            }
        for field in FRAME_FIELDS:
            if expected_frame.get(field) != actual_frame.get(field):
                return {
                    "frame": index,
                    "field": field,
                    "expected": expected_frame.get(field),
                    "actual": actual_frame.get(field),
                }
    if len(expected_frames) != len(actual_frames):
        return {
            "frame": common,
            "field": "frame_count",
            "expected": len(expected_frames),
            "actual": len(actual_frames),
        }
    if expected.get("packet_stream_sha256") != actual.get("packet_stream_sha256"):
        return {
            "frame": None,
            "field": "packet_stream_sha256",
            "expected": expected.get("packet_stream_sha256"),
            "actual": actual.get("packet_stream_sha256"),
        }
    return None


def main() -> None:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("wav", type=Path)
    parser.add_argument("--profile")
    parser.add_argument("--output", type=Path, required=True)
    parser.add_argument("--compare", type=Path)
    args = parser.parse_args()
    frame_contract = build_frame_contract(args.wav, profile=args.profile)
    if args.compare:
        expected = json.loads(args.compare.read_text())
        difference = first_frame_contract_difference(expected, frame_contract)
        if difference is not None:
            raise AssertionError(f"frame contract differs: {difference}")
    args.output.parent.mkdir(parents=True, exist_ok=True)
    args.output.write_text(json.dumps(frame_contract, indent=2) + "\n")
    print(
        "frame contract OK",
        {"frames": frame_contract["audio_packets"], "output": str(args.output)},
    )


if __name__ == "__main__":
    main()
