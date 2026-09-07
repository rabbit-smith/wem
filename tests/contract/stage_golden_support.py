"""Generate and compare the checked stage-golden pipeline assets.

The stage golden assets freeze the per-frame values at every pipeline seam as
a byte-exact reference for the Rust kernel port:

- ``index.json`` records SHA-256 hashes for every stage of every frame,
  reusing the frame-contract row packing (little-endian f32 words for float
  surfaces, little-endian i32 words for integer rows), plus the per-segment
  hashes of the final WEM container (fmt, setup, data).
- ``frames/`` holds raw little-endian binary dumps for representative frames
  only: the first 6 frames, the last 4 frames, and both sides of every
  short/long mode transition.  Float stages dump as f32le words, floor posts
  as u16le words (``0xffff`` marks an unused channel), residue integers as
  i32le words, and packet bytes verbatim.

Capture uses the production path only: observing wrappers installed at
runtime on ``Encoder.encode_pcm``, ``AnalysisSession.analyze_window``, and
the reference engine module's own references to ``pack_analysis_frame``
and ``build_vorbis_wem`` record the values the real pipeline computes, then
are removed.  No production module is modified on disk.  The facade engine
is pinned to the pure-Python reference implementation for the capture, so
the oracle pipeline is what gets observed regardless of whether the native
kernel happens to be importable in the environment.
"""
from __future__ import annotations

import hashlib
import json
import os
import struct
from contextlib import contextmanager
from pathlib import Path
from typing import Any, Iterator

import wwise_wem._engine as _facade_engine
import wwise_wem_reference.python_engine as reference_engine_module
from wwise_wem_reference.analysis.session import AnalysisSession
from wwise_wem import encode_wav
from wwise_wem.application.encoder import Encoder
from wwise_wem_reference.container.wem import load_wem_parts_bytes


SCHEMA = "wwise-wem.stage-golden.v1"
FIRST_REPRESENTATIVE_FRAMES = 6
LAST_REPRESENTATIVE_FRAMES = 4

# Hashed stages per frame, in pipeline order.  The names match the
# frame-contract field conventions where they already exist.
FLOAT_STAGES = (
    "window",
    "coefficients",
    "raw_mdct",
    "fft",
    "remap",
    "seed",
    "post",
    "side",
)
INT_STAGES = ("floor_posts", "residue_q")
U8_STAGES = ("packet",)
DUMP_STAGES = FLOAT_STAGES + INT_STAGES + U8_STAGES

FRAME_FIELDS = (
    "index",
    "mode",
    "previous",
    "current",
    "following",
    "window_center",
    "bins",
    *tuple(f"{name}_sha256" for name in FLOAT_STAGES),
    "floor_posts_sha256",
    "residue_q_after_sha256",
    "packet_size",
    "packet_sha256",
)

CONTAINER_FIELDS = (
    "wem_sha256",
    "wem_size",
    "fmt_sha256",
    "fmt_size",
    "setup_sha256",
    "setup_size",
    "data_sha256",
    "data_size",
)

PCM_FIELDS = ("sample_rate", "channels", "pcm_frames", "pcm_sha256")

HEADER_FIELDS = (
    "schema",
    "profile",
    "channels",
    "sample_rate",
    "pcm_frames",
    "input_sha256",
    "audio_packets",
)


def _hash_float_rows(rows) -> str:
    digest = hashlib.sha256()
    for row in rows:
        for value in row:
            digest.update(struct.pack("<f", float(value)))
    return digest.hexdigest()


def _hash_int_rows(rows) -> str:
    digest = hashlib.sha256()
    for row in rows:
        if row is None:
            digest.update(b"\xff")
            continue
        digest.update(b"\x00")
        for value in row:
            digest.update(struct.pack("<i", int(value)))
    return digest.hexdigest()


def pack_float_rows(rows) -> bytes:
    """Packed f32le words, channel-major, matching ``_hash_float_rows``."""
    return b"".join(
        struct.pack("<f", float(value)) for row in rows for value in row
    )


def pack_posts(posts) -> bytes:
    """Packed u16le floor posts; ``0xffff`` marks an unused channel."""
    out = bytearray()
    for row in posts:
        if row is None:
            out += struct.pack("<H", 0xFFFF)
            continue
        for value in row:
            out += struct.pack("<H", int(value))
    return bytes(out)


def pack_int_rows(rows) -> bytes:
    """Packed i32le residue words, channel-major."""
    return b"".join(
        struct.pack("<i", int(value)) for row in rows for value in row
    )


def _dump_bytes(stage: str, raw) -> bytes:
    if stage in FLOAT_STAGES:
        return pack_float_rows(raw)
    if stage == "floor_posts":
        return pack_posts(raw)
    if stage == "residue_q":
        return pack_int_rows(raw)
    return bytes(raw)


def _dump_extension(stage: str) -> str:
    if stage in FLOAT_STAGES:
        return "f32le.bin"
    if stage == "floor_posts":
        return "u16le.bin"
    if stage == "residue_q":
        return "i32le.bin"
    return "u8.bin"


def representative_frame_indices(modes) -> tuple[int, ...]:
    """First/last representative frames plus both sides of each transition."""
    count = len(modes)
    keep = set(range(min(FIRST_REPRESENTATIVE_FRAMES, count)))
    keep.update(range(max(0, count - LAST_REPRESENTATIVE_FRAMES), count))
    for index in transition_frame_indices(modes):
        keep.update({index - 1, index})
    return tuple(sorted(keep))


def transition_frame_indices(modes) -> tuple[int, ...]:
    """Frame indices where the emitted mode differs from the previous one."""
    return tuple(
        index
        for index in range(1, len(modes))
        if modes[index] != modes[index - 1]
    )


class _StageCapture:
    """Observe one production encode and keep only what the assets need."""

    def __init__(self) -> None:
        self.pcm_geometry: dict[str, int] | None = None
        self.pcm_sha256: str | None = None
        self.profile_name: str | None = None
        self.frame_rows: list[dict[str, Any]] = []
        self.raw_frames: dict[int, dict[str, Any]] = {}
        self.setup_packet: bytes | None = None
        self.seek_table: bytes | None = None
        self.wem_bytes: bytes | None = None
        self._keep: set[int] = set()
        self._recent_raw: list[int] = []
        self._pending: dict[str, Any] | None = None

    # -- observing wrappers -------------------------------------------------

    def on_encode_pcm(self, encoder: Encoder, pcm) -> None:
        self.profile_name = encoder.profile.name
        self.pcm_geometry = {
            "sample_rate": pcm.sample_rate,
            "channels": pcm.channel_count,
            "pcm_frames": pcm.frame_count,
        }
        self.pcm_sha256 = _hash_float_rows(pcm.channels)

    def on_analyze_window(self, window, analysis) -> None:
        index = window.index
        if index != len(self.frame_rows):
            raise RuntimeError("analysis frame indices are not contiguous")
        rows = {
            "window": window.samples,
            "coefficients": analysis.coefficients,
            "raw_mdct": analysis.raw_mdct,
            "fft": analysis.fft,
            "remap": analysis.remap,
            "seed": analysis.seed,
            "post": analysis.post,
            "side": analysis.side,
        }
        previous_mode = (
            self.frame_rows[-1]["current"] if self.frame_rows else None
        )
        is_transition = (
            previous_mode is not None and window.current != previous_mode
        )
        if is_transition:
            self._keep.update({index - 1, index})
        self._pending = {"index": index, "rows": rows, "keep": is_transition}
        self.frame_rows.append(
            {
                "index": index,
                "mode": window.current,
                "previous": window.previous,
                "current": window.current,
                "following": window.following,
                "window_center": window.center,
                "bins": len(analysis.coefficients[0]),
                "window_sha256": _hash_float_rows(rows["window"]),
                "coefficients_sha256": _hash_float_rows(
                    rows["coefficients"]
                ),
                "raw_mdct_sha256": _hash_float_rows(rows["raw_mdct"]),
                "fft_sha256": _hash_float_rows(rows["fft"]),
                "remap_sha256": _hash_float_rows(rows["remap"]),
                "seed_sha256": _hash_float_rows(rows["seed"]),
                "post_sha256": _hash_float_rows(rows["post"]),
                "side_sha256": _hash_float_rows(rows["side"]),
            }
        )
        self._prune_raw(index)

    def on_pack_analysis_frame(self, encoded) -> None:
        pending = self._pending
        if pending is None:
            raise RuntimeError("pack_analysis_frame ran without analysis")
        index = pending["index"]
        frame = self.frame_rows[index]
        frame["floor_posts_sha256"] = _hash_int_rows(encoded.posts)
        frame["residue_q_after_sha256"] = _hash_int_rows(
            encoded.quantized_residue
        )
        frame["packet_size"] = len(encoded.packet)
        frame["packet_sha256"] = hashlib.sha256(encoded.packet).hexdigest()
        self._pending = None
        # The current frame is the newest raw row, so it always stays for
        # now; _prune_raw trims it once later frames prove it unneeded
        # (the trailing last-4 representatives are kept at build time).
        self.raw_frames[index] = {
            **pending["rows"],
            "floor_posts": encoded.posts,
            "residue_q": encoded.quantized_residue,
            "packet": encoded.packet,
        }
        self._recent_raw = (self._recent_raw + [index])[
            -LAST_REPRESENTATIVE_FRAMES:
        ]
        self._prune_raw(index)

    def on_build_vorbis_wem(
        self,
        packets: list,
        seek_table: bytes,
        wem_bytes: bytes,
    ) -> None:
        if not packets:
            raise RuntimeError("build_vorbis_wem received no packets")
        self.setup_packet = bytes(packets[0])
        self.seek_table = bytes(seek_table)
        self.wem_bytes = bytes(wem_bytes)

    # -- housekeeping -------------------------------------------------------

    def _prune_raw(self, index: int) -> None:
        for stored in list(self.raw_frames):
            if stored in self._keep:
                continue
            if stored < FIRST_REPRESENTATIVE_FRAMES:
                continue
            if stored in self._recent_raw:
                continue
            if stored == index:
                continue
            del self.raw_frames[stored]


@contextmanager
def stage_capture() -> Iterator[_StageCapture]:
    """Run the production encoder with observing wrappers installed."""
    capture = _StageCapture()
    encoder_class = Encoder
    session_class = AnalysisSession
    original_encode = encoder_class.encode_pcm
    original_analyze = session_class.analyze_window
    original_pack = reference_engine_module.pack_analysis_frame
    original_build = reference_engine_module.build_vorbis_wem

    def wrapped_encode_pcm(self, pcm):
        capture.on_encode_pcm(self, pcm)
        return original_encode(self, pcm)

    def wrapped_analyze_window(self, window, *, short_variant=None):
        result = original_analyze(self, window, short_variant=short_variant)
        capture.on_analyze_window(window, result)
        return result

    def wrapped_pack_analysis_frame(setup, books, analysis, *, channels):
        result = original_pack(
            setup, books, analysis, channels=channels
        )
        capture.on_pack_analysis_frame(result)
        return result

    def wrapped_build_vorbis_wem(
        fmt_fields,
        packets,
        *,
        seek_table=b"",
        endian="le",
        extra_chunks=None,
        recompute_sizes=True,
        fmt_raw=None,
    ):
        wem_bytes = original_build(
            fmt_fields,
            packets,
            seek_table=seek_table,
            endian=endian,
            extra_chunks=extra_chunks,
            recompute_sizes=recompute_sizes,
            fmt_raw=fmt_raw,
        )
        capture.on_build_vorbis_wem(packets, seek_table, wem_bytes)
        return wem_bytes

    encoder_class.encode_pcm = wrapped_encode_pcm
    session_class.analyze_window = wrapped_analyze_window
    reference_engine_module.pack_analysis_frame = wrapped_pack_analysis_frame
    reference_engine_module.build_vorbis_wem = wrapped_build_vorbis_wem
    try:
        yield capture
    finally:
        encoder_class.encode_pcm = original_encode
        session_class.analyze_window = original_analyze
        reference_engine_module.pack_analysis_frame = original_pack
        reference_engine_module.build_vorbis_wem = original_build


def build_stage_golden(
    wav: Path, *, profile: str | None = None
) -> tuple[dict[str, Any], dict[str, bytes]]:
    """Run the real encoder and return (stage index, raw dump blobs)."""
    wav = Path(wav)
    previous_engine = os.environ.get(_facade_engine.ENGINE_ENV_VAR)
    os.environ[_facade_engine.ENGINE_ENV_VAR] = "python"
    try:
        with stage_capture() as capture:
            result = encode_wav(wav, profile=profile)
    finally:
        if previous_engine is None:
            os.environ.pop(_facade_engine.ENGINE_ENV_VAR, None)
        else:
            os.environ[_facade_engine.ENGINE_ENV_VAR] = previous_engine

    frames = capture.frame_rows
    if len(frames) != result.stats.audio_packets:
        raise RuntimeError("stage capture missed analysis frames")
    for field in (
        "pcm_geometry",
        "pcm_sha256",
        "profile_name",
        "setup_packet",
        "wem_bytes",
    ):
        if getattr(capture, field) is None:
            raise RuntimeError(f"stage capture is incomplete: {field}")

    modes = [frame["mode"] for frame in frames]
    transitions = transition_frame_indices(modes)
    representatives = representative_frame_indices(modes)

    raw_frames = {
        index: raw
        for index, raw in capture.raw_frames.items()
        if index in representatives
    }
    missing = set(representatives) - set(raw_frames)
    if missing:
        raise RuntimeError(f"missing raw rows for frames {sorted(missing)}")

    parts = load_wem_parts_bytes(capture.wem_bytes)
    container = {
        "wem_sha256": hashlib.sha256(capture.wem_bytes).hexdigest(),
        "wem_size": len(capture.wem_bytes),
        "fmt_sha256": hashlib.sha256(parts["fmt_raw"]).hexdigest(),
        "fmt_size": len(parts["fmt_raw"]),
        "setup_sha256": hashlib.sha256(capture.setup_packet).hexdigest(),
        "setup_size": len(capture.setup_packet),
        "data_sha256": hashlib.sha256(parts["data_raw"]).hexdigest(),
        "data_size": len(parts["data_raw"]),
    }
    pcm_block = {
        **capture.pcm_geometry,
        "pcm_sha256": capture.pcm_sha256,
    }

    dumps: dict[str, bytes] = {}
    for index in representatives:
        raw = raw_frames[index]
        for stage in DUMP_STAGES:
            blob = _dump_bytes(stage, raw[stage])
            dumps[f"f{index:03d}.{stage}"] = blob

    dump_index = {
        key: {
            "path": f"frames/f{index:03d}.{stage}.{_dump_extension(stage)}",
            "sha256": hashlib.sha256(blob).hexdigest(),
            "size": len(blob),
        }
        for (index, stage), (key, blob) in (
            (
                (int(key.split(".")[0][1:]), key.split(".")[1]),
                (key, blob),
            )
            for key, blob in dumps.items()
        )
    }

    index_doc = {
        "schema": SCHEMA,
        "profile": capture.profile_name,
        "channels": capture.pcm_geometry["channels"],
        "sample_rate": capture.pcm_geometry["sample_rate"],
        "pcm_frames": capture.pcm_geometry["pcm_frames"],
        "input_sha256": hashlib.sha256(wav.read_bytes()).hexdigest(),
        "audio_packets": len(frames),
        "pcm": pcm_block,
        "container": container,
        "transition_frames": list(transitions),
        "representative_frames": list(representatives),
        "frames": frames,
        "dumps": dump_index,
    }
    return index_doc, dumps


def write_stage_golden(
    index_doc: dict[str, Any],
    dumps: dict[str, bytes],
    out_dir: Path,
) -> dict[str, int]:
    """Write index.json and the raw dumps; remove stale dumps idempotently."""
    out_dir = Path(out_dir)
    frames_dir = out_dir / "frames"
    frames_dir.mkdir(parents=True, exist_ok=True)
    expected_names = {
        Path(entry["path"]).name for entry in index_doc["dumps"].values()
    }
    removed = 0
    for path in sorted(frames_dir.glob("*.bin")):
        if path.name not in expected_names:
            path.unlink()
            removed += 1
    for key, blob in dumps.items():
        (out_dir / index_doc["dumps"][key]["path"]).write_bytes(blob)
    (out_dir / "index.json").write_text(
        json.dumps(index_doc, indent=2, sort_keys=True) + "\n"
    )
    return {
        "frames": len(index_doc["frames"]),
        "representative_frames": len(index_doc["representative_frames"]),
        "dumps": len(dumps),
        "removed_stale_dumps": removed,
        "index_bytes": (out_dir / "index.json").stat().st_size,
        "dump_bytes": sum(len(blob) for blob in dumps.values()),
    }


def first_stage_golden_difference(
    expected: dict[str, Any], actual: dict[str, Any]
) -> dict[str, Any] | None:
    """Return a compact first-difference report, or ``None`` when identical."""
    for field in HEADER_FIELDS:
        if expected.get(field) != actual.get(field):
            return {
                "frame": None,
                "stage": field,
                "expected": expected.get(field),
                "actual": actual.get(field),
            }
    for field in PCM_FIELDS:
        if expected.get("pcm", {}).get(field) != actual.get("pcm", {}).get(
            field
        ):
            return {
                "frame": None,
                "stage": f"pcm.{field}",
                "expected": expected.get("pcm", {}).get(field),
                "actual": actual.get("pcm", {}).get(field),
            }
    for field in CONTAINER_FIELDS:
        if expected.get("container", {}).get(
            field
        ) != actual.get("container", {}).get(field):
            return {
                "frame": None,
                "stage": f"container.{field}",
                "expected": expected.get("container", {}).get(field),
                "actual": actual.get("container", {}).get(field),
            }

    expected_frames = expected.get("frames", [])
    actual_frames = actual.get("frames", [])
    common = min(len(expected_frames), len(actual_frames))
    for index in range(common):
        expected_frame = expected_frames[index]
        actual_frame = actual_frames[index]
        for field in FRAME_FIELDS:
            if expected_frame.get(field) != actual_frame.get(field):
                return {
                    "frame": index,
                    "stage": field,
                    "expected": expected_frame.get(field),
                    "actual": actual_frame.get(field),
                }
    if len(expected_frames) != len(actual_frames):
        return {
            "frame": common,
            "stage": "frame_count",
            "expected": len(expected_frames),
            "actual": len(actual_frames),
        }

    for field in ("transition_frames", "representative_frames"):
        if expected.get(field) != actual.get(field):
            return {
                "frame": None,
                "stage": field,
                "expected": expected.get(field),
                "actual": actual.get(field),
            }

    expected_dumps = expected.get("dumps", {})
    actual_dumps = actual.get("dumps", {})
    for key in sorted(set(expected_dumps) | set(actual_dumps)):
        expected_entry = expected_dumps.get(key, {})
        actual_entry = actual_dumps.get(key, {})
        for field in ("path", "sha256", "size"):
            if expected_entry.get(field) != actual_entry.get(field):
                return {
                    "frame": None,
                    "stage": f"dumps.{key}.{field}",
                    "expected": expected_entry.get(field),
                    "actual": actual_entry.get(field),
                }
    return None
