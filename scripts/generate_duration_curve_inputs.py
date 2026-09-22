#!/usr/bin/env python3
"""Generate the deterministic PCM16 programs the duration curves are measured on.

This is a *measurement input* generator, not a fixture generator: the WAVs land
in a scratch directory under the gitignored ``crates/target/`` tree and nothing
in the test suites reads them.  It generalizes the signal definition of
:mod:`scripts.generate_2ch_long_program` (integer fixed-point phase, triangle
oscillators, a fixed xorshift percussion seed, no platform math-library calls,
no run-time randomness) from one stereo/48 kHz/20 s program to
``(duration, channels, sample_rate)``.

The generalization is *checked against the original*, not asserted: at
``(20 s, 2 ch, 48 kHz)`` the general renderer must reproduce
:func:`scripts.generate_2ch_long_program.render_pcm16le` byte for byte, and the
script refuses to emit anything if it does not.  That check is the reason the
per-channel layout below is written the way it is -- channels 0 and 1 are group
0, and group 0 is the original program.

Geometry: channels are grouped in pairs.  Group ``g`` plays chord
``(act + g) mod 4``, the lead pattern offset by ``g`` half-second steps, a pan
oscillator at ``(g + 1)`` times the original rate, and a percussion hit every
``0.25 s * (g + 1)``; the even channel of a group takes the left pan and the
``+burst`` percussion branch, the odd one the right pan and the ``-(burst // 2)``
branch.  For ``2 ch`` group 0 is the whole program, so every term collapses to
the original expression.

Byte stability: every value is an integer throughout, the packing is explicit
little-endian signed 16-bit, the header comes from the standard library's
canonical ``wave`` writer, and nothing ambient (time, path, host) enters the
payload.  Run it twice and the digests match.

Usage::

    PYTHONDONTWRITEBYTECODE=1 PYTHONPATH=src:reference \\
        python3 scripts/generate_duration_curve_inputs.py

    # explicit matrix (durations in seconds; geometry as <channels>ch<rate>)
    python3 scripts/generate_duration_curve_inputs.py \\
        --durations 10,30,60 --geometries 6ch44100,2ch48000 \\
        --output-dir crates/target/curves/inputs

    # skip the reduction check against scripts/generate_2ch_long_program.py
    python3 scripts/generate_duration_curve_inputs.py --no-legacy-check

Nothing is downloaded and nothing outside ``--output-dir`` is written.
"""
from __future__ import annotations

import argparse
import hashlib
import struct
import sys
import wave
from pathlib import Path

REPO = Path(__file__).resolve().parents[1]
DEFAULT_OUTPUT_DIR = REPO / "crates" / "target" / "curves" / "inputs"
DEFAULT_DURATIONS = (10, 30, 60, 120, 300, 600)
DEFAULT_GEOMETRIES = ("6ch44100", "2ch48000")

#: The rate every frozen increment below is stated at.
BASE_RATE = 48_000
_PHASE_MASK = (1 << 32) - 1
_FULL_SCALE = 32_767
_PAN_FULL_SCALE = 65_534

#: Original percussion geometry at 48 kHz: a hit every 0.25 s, a 640-frame
#: decay envelope, a 12-frame click.
_HIT_FRAMES = 12_000
_ENVELOPE_FRAMES = 640
_CLICK_FRAMES = 12
_PAN_INCREMENT = 178_957  # about 2 Hz across the stereo field

#: Fixed Q0.32 phase increments at 48 kHz, grouped as four major chords.
_CHORDS = (
    (11_704_931, 14_747_289, 17_537_577),  # C3, E3, G3
    (9_842_633, 13_138_341, 16_424_205),  # A2, D3, F3
    (13_138_341, 17_537_577, 22_095_969),  # D3, G3, B3
    (9_842_633, 14_747_289, 19_685_267),  # A2, E3, A3
)
_LEAD = (
    23_409_862,
    29_494_578,
    35_075_155,
    39_370_534,
    44_191_930,
    35_075_155,
    29_494_578,
    26_276_681,
)
#: `_section_levels`' five four-second acts (chord, lead, percussion).
_SECTION_LEVELS = (
    (2_200, 0, 1_000),
    (4_800, 3_800, 2_000),
    (8_000, 5_800, 9_000),
    (1_000, 1_600, 0),
    (8_800, 6_800, 12_000),
)
#: Per-group lead phase offsets, so the pairs do not start in lockstep.  The
#: first entry is the original start phase.
_LEAD_PHASE = (0x4000_0000, 0x2000_0000, 0x6000_0000)
#: Per-group xorshift seeds, so the percussion bursts are independent.  The
#: first entry is the original seed.
_NOISE_SEED = (0x6D2B_79F5, 0x1F3A_5C7D, 0x5E8D_2B41)

_SECTION_FRAMES = 4  # seconds per act


def _scale(increment: int, sample_rate: int) -> int:
    """Scale one Q0.32 increment stated at 48 kHz to ``sample_rate``."""
    return increment * sample_rate // BASE_RATE


def _triangle(phase: int) -> int:
    """Map one unsigned Q0.32 phase to a signed 16-bit triangle wave."""
    position = phase >> 16
    rising = position if position < 32_768 else 65_535 - position
    return (rising << 1) - _FULL_SCALE


def _clamp_i16(value: int) -> int:
    return max(-32_768, min(_FULL_SCALE, value))


def parse_geometry(label: str) -> tuple[int, int]:
    """``"6ch44100"`` -> ``(6, 44100)``."""
    channels_text, _, rate_text = label.partition("ch")
    if not channels_text.isdigit() or not rate_text.isdigit():
        raise SystemExit(f"geometry {label!r} is not <channels>ch<rate>")
    channels = int(channels_text)
    sample_rate = int(rate_text)
    if channels < 1 or sample_rate < 8_000:
        raise SystemExit(f"geometry {label!r} has an unusable channel count or rate")
    return channels, sample_rate


def render_pcm16le(duration_seconds: int, channels: int, sample_rate: int) -> bytes:
    """Return the canonical interleaved PCM16 little-endian payload."""
    frame_count = duration_seconds * sample_rate
    payload = bytearray(frame_count * channels * 2)
    groups = (channels + 1) // 2
    row_bytes = channels * 2

    # Every increment is scaled once; the inner loop stays integer arithmetic.
    scaled_chords = tuple(
        tuple(_scale(increment, sample_rate) for increment in chord)
        for chord in _CHORDS
    )
    scaled_lead = tuple(_scale(increment, sample_rate) for increment in _LEAD)
    pan_increment = _scale(_PAN_INCREMENT, sample_rate)
    hit_frames = sample_rate // 4
    envelope_frames = _ENVELOPE_FRAMES * sample_rate // BASE_RATE
    click_frames = _CLICK_FRAMES * sample_rate // BASE_RATE
    section_frames = _SECTION_FRAMES * sample_rate

    chord_phases = [[0, 0x1555_5555, 0x2AAA_AAAA] for _ in range(groups)]
    lead_phases = [_LEAD_PHASE[g % len(_LEAD_PHASE)] for g in range(groups)]
    pan_phases = [0] * groups
    noise_states = [_NOISE_SEED[g % len(_NOISE_SEED)] for g in range(groups)]

    full_scale = _FULL_SCALE
    pan_scale = _PAN_FULL_SCALE
    mask = _PHASE_MASK
    lead_length = len(_LEAD)
    row = [0] * channels
    offset = 0
    for frame in range(frame_count):
        chord_level, lead_level, percussion_level = _SECTION_LEVELS[
            (frame // section_frames) % len(_SECTION_LEVELS)
        ]
        lead_index = frame // (sample_rate // 2)
        for group in range(groups):
            phases = chord_phases[group]
            increments = scaled_chords[(frame // sample_rate + group) % len(_CHORDS)]
            chord = 0
            for index in range(3):
                phase = (phases[index] + increments[index]) & mask
                phases[index] = phase
                position = phase >> 16
                rising = position if position < 32_768 else 65_535 - position
                chord += (rising << 1) - full_scale
            chord //= 3

            lead_phase = (lead_phases[group] + scaled_lead[(lead_index + group) % lead_length]) & mask
            lead_phases[group] = lead_phase
            position = lead_phase >> 16
            rising = position if position < 32_768 else 65_535 - position
            lead = (rising << 1) - full_scale

            pan_phase = (pan_phases[group] + pan_increment * (group + 1)) & mask
            pan_phases[group] = pan_phase
            position = pan_phase >> 16
            rising = position if position < 32_768 else 65_535 - position
            pan = ((rising << 1) - full_scale) + full_scale
            left_pan = pan_scale - pan
            right_pan = pan

            shared = chord * chord_level // full_scale
            lead_scaled = lead * lead_level

            noise_state = noise_states[group]
            noise_state ^= (noise_state << 13) & mask
            noise_state ^= noise_state >> 17
            noise_state ^= (noise_state << 5) & mask
            noise_state &= mask
            noise_states[group] = noise_state
            burst = 0
            click = 0
            hit_frame = frame % (hit_frames * (group + 1))
            if percussion_level and hit_frame < envelope_frames:
                envelope = envelope_frames - hit_frame
                noise = ((noise_state >> 16) & 0xFFFF) - 32_768
                burst = noise * percussion_level * envelope // (32_768 * envelope_frames)
                click = percussion_level // 2 if hit_frame < click_frames else 0

            for index in range(2):
                channel = 2 * group + index
                if channel >= channels:
                    break
                if index == 0:
                    value = shared + lead_scaled * left_pan // (full_scale * pan_scale)
                    value += burst + click
                else:
                    value = shared + lead_scaled * right_pan // (full_scale * pan_scale)
                    value += -(burst // 2) + click
                row[channel] = _clamp_i16(value)

        struct.pack_into(f"<{channels}h", payload, offset, *row)
        offset += row_bytes

    return bytes(payload)


def _legacy_payload() -> bytes:
    """The original 20 s stereo/48 kHz program, for the reduction check."""
    if str(REPO) not in sys.path:
        sys.path.insert(0, str(REPO))
    from scripts.generate_2ch_long_program import render_pcm16le as legacy

    return legacy()


def check_legacy_reduction() -> None:
    """The general renderer must reproduce the original program exactly."""
    general = render_pcm16le(20, 2, 48_000)
    legacy = _legacy_payload()
    if general != legacy:
        first = next(
            (
                index
                for index, (left, right) in enumerate(zip(general, legacy))
                if left != right
            ),
            min(len(general), len(legacy)),
        )
        raise SystemExit(
            "the generalized renderer no longer reduces to "
            "scripts/generate_2ch_long_program.render_pcm16le at 2ch/48000/20s "
            f"(first differing byte {first}, {len(general)} vs {len(legacy)} bytes)"
        )
    print(
        "reduction check: 2ch/48000/20s == "
        f"scripts.generate_2ch_long_program ({hashlib.sha256(general).hexdigest()})"
    )


def write_wav(path: Path, payload: bytes, channels: int, sample_rate: int) -> str:
    path.parent.mkdir(parents=True, exist_ok=True)
    with wave.open(str(path), "wb") as destination:
        destination.setnchannels(channels)
        destination.setsampwidth(2)
        destination.setframerate(sample_rate)
        destination.writeframes(payload)
    return hashlib.sha256(payload).hexdigest()


def main(argv: list[str] | None = None) -> int:
    parser = argparse.ArgumentParser(
        description=__doc__,
        formatter_class=argparse.RawDescriptionHelpFormatter,
    )
    parser.add_argument(
        "--output-dir",
        type=Path,
        default=DEFAULT_OUTPUT_DIR,
        help=(
            "scratch directory the WAVs are written to "
            f"(default: {DEFAULT_OUTPUT_DIR.relative_to(REPO)}, gitignored)"
        ),
    )
    parser.add_argument(
        "--durations",
        default=",".join(str(value) for value in DEFAULT_DURATIONS),
        help="comma-separated durations in seconds (default: 10,30,60,120,300,600)",
    )
    parser.add_argument(
        "--geometries",
        default=",".join(DEFAULT_GEOMETRIES),
        help="comma-separated <channels>ch<rate> labels (default: 6ch44100,2ch48000)",
    )
    parser.add_argument(
        "--no-legacy-check",
        action="store_true",
        help="skip the byte-for-byte reduction check against the original 2ch program",
    )
    args = parser.parse_args(argv)

    durations = [int(value) for value in args.durations.split(",") if value]
    geometries = [value for value in args.geometries.split(",") if value]
    if not durations or not geometries:
        raise SystemExit("--durations and --geometries must both be non-empty")

    if not args.no_legacy_check:
        check_legacy_reduction()

    failures = 0
    for label in geometries:
        channels, sample_rate = parse_geometry(label)
        for duration in durations:
            payload = render_pcm16le(duration, channels, sample_rate)
            path = args.output_dir / f"{label}-{duration}s.wav"
            digest = write_wav(path, payload, channels, sample_rate)
            print(
                f"{digest}  {duration:>4}s  {channels}ch/{sample_rate}  "
                f"{len(payload):>12} bytes  {path}"
            )
    return 1 if failures else 0


if __name__ == "__main__":
    raise SystemExit(main())
