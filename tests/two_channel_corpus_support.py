"""Shared deterministic input generation for the 2ch reference corpora."""

from __future__ import annotations

import json
import math
import struct
from pathlib import Path
from typing import TYPE_CHECKING, Sequence

if TYPE_CHECKING:
    from wwise_wem import WwiseProfile


RATE = 48_000
FRAMES = 48_000
REFERENCE_CASES = ("silence", "dc_opposed", "tone_low", "tone_high", "impulse", "stereo_noise")
STRESS_CASES = ("channel_bursts", "tail_change")


def _sample(case: str, frame: int, channel: int, states: list[int]) -> int:
    if case == "silence":
        return 0
    if case == "dc_opposed":
        return 8192 if channel == 0 else -4096
    if case == "tone_low":
        phase = 0.0 if channel == 0 else math.pi / 3.0
        amplitude = 12_000.0 if channel == 0 else 9_000.0
        return round(amplitude * math.sin(2.0 * math.pi * 100.0 * frame / RATE + phase))
    if case == "tone_high":
        frequency = 10_000.0 if channel == 0 else 12_000.0
        return round(10_000.0 * math.sin(2.0 * math.pi * frequency * frame / RATE))
    if case == "impulse":
        if frame == 8000 and channel == 0:
            return 30_000
        if frame == 8003 and channel == 1:
            return -28_000
        if frame == 24_000:
            return -24_000 if channel == 0 else 22_000
        return 0
    if case == "stereo_noise":
        states[channel] = (states[channel] * 1_103_515_245 + 12_345) % 2_147_483_648
        return ((states[channel] >> 8) & 0xFFFF) - 32_768
    if case == "channel_bursts":
        active = (frame // 2048) % 2
        if channel != active:
            return 0
        return 16_000 if frame % 96 < 48 else -16_000
    if case == "tail_change":
        if frame < 45_952:
            return round(4_000.0 * math.sin(2.0 * math.pi * 300.0 * frame / RATE))
        tail_index = frame - 45_952
        frequency = 3_000.0 + 2.0 * tail_index
        sign = 1.0 if channel == 0 else -1.0
        return round(sign * 14_000.0 * math.sin(2.0 * math.pi * frequency * tail_index / RATE))
    raise ValueError(f"unknown 2ch reference case: {case}")


def render_wav(case: str) -> bytes:
    """Return one canonical PCM16 RIFF/WAVE input."""
    if case not in REFERENCE_CASES + STRESS_CASES:
        raise ValueError(f"unknown 2ch reference case: {case}")
    states = [1, 104_729]
    samples = (
        _sample(case, frame, channel, states) for frame in range(FRAMES) for channel in range(2)
    )
    payload = b"".join(struct.pack("<h", value) for value in samples)
    header = struct.pack(
        "<4sI4s4sIHHIIHH4sI",
        b"RIFF",
        36 + len(payload),
        b"WAVE",
        b"fmt ",
        16,
        1,
        2,
        RATE,
        RATE * 4,
        4,
        16,
        b"data",
        len(payload),
    )
    return header + payload


def verify_or_write_inputs(
    cases: Sequence[str], destination: Path, manifest_path: Path, *, check: bool
) -> None:
    """Check or write the generated inputs against the manifest's file names.

    The comparison is the committed input's own bytes: a rendered input that
    differs from the file the manifest names is reported before anything is
    written, and ``check`` additionally reports a file that is missing.
    """
    manifest = json.loads(manifest_path.read_text(encoding="utf-8"))
    entries = {Path(entry["input"]).stem: entry for entry in manifest["cases"]}
    for case in cases:
        data = render_wav(case)
        entry = entries.get(case)
        if entry is None:
            raise ValueError(f"manifest has no input entry for {case}")
        path = destination / entry["input"]
        if path.is_file():
            if path.read_bytes() != data:
                raise ValueError(f"committed input differs from generator: {path}")
        elif check:
            raise ValueError(f"committed input is missing: {path}")
        if not check:
            path.parent.mkdir(parents=True, exist_ok=True)
            path.write_bytes(data)


def corpus_selection(profile_key: str) -> WwiseProfile:
    """The structured selection one corpus manifest pins.

    A manifest stores the installed profile's identity key; the selection is
    derived from that key's generation and geometry, so the corpora never
    pick a profile by label.
    """
    from wwise_wem import WwiseProfile, WwiseVersion
    from wwise_wem_reference.profiles.artifact import compiled_profiles

    matches = [
        profile
        for profile in compiled_profiles()
        if profile.key.describe() == profile_key or profile.label() == profile_key
    ]
    if len(matches) != 1:
        raise ValueError(f"no single installed profile for {profile_key!r}")
    key = matches[0].key
    return WwiseProfile(
        WwiseVersion.from_generation(key.generation),
        key.channels,
        key.sample_rate,
    )
