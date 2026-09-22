#!/usr/bin/env python3
"""Generate a deterministic 20-second stereo PCM16 music-like program.

The long-run check imports :func:`render_pcm16le` directly, so the test does
not need a committed multi-megabyte WAV.  Oscillators use unsigned fixed-point
phase and triangle waves; the percussion noise uses a fixed xorshift seed.
There are no platform math-library calls in the signal definition.
"""

from __future__ import annotations

import argparse
import struct
import wave
from pathlib import Path


SAMPLE_RATE = 48_000
CHANNELS = 2
DURATION_SECONDS = 20
FRAME_COUNT = SAMPLE_RATE * DURATION_SECONDS

_PHASE_MASK = (1 << 32) - 1
_FULL_SCALE = 32_767

# Fixed Q0.32 phase increments at 48 kHz, grouped as four major chords.
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


def _triangle(phase: int) -> int:
    """Map one unsigned Q0.32 phase to a signed 16-bit triangle wave."""
    position = phase >> 16
    rising = position if position < 32_768 else 65_535 - position
    return (rising << 1) - _FULL_SCALE


def _clamp_i16(value: int) -> int:
    return max(-32_768, min(_FULL_SCALE, value))


def _section_levels(frame: int) -> tuple[int, int, int]:
    """Return chord, lead, and percussion levels for five four-second acts."""
    section = frame // (4 * SAMPLE_RATE)
    return (
        (2_200, 0, 1_000),
        (4_800, 3_800, 2_000),
        (8_000, 5_800, 9_000),
        (1_000, 1_600, 0),
        (8_800, 6_800, 12_000),
    )[section]


def render_pcm16le() -> bytes:
    """Return the canonical interleaved stereo PCM16 little-endian payload."""
    payload = bytearray(FRAME_COUNT * CHANNELS * 2)
    chord_phases = [0, 0x1555_5555, 0x2AAA_AAAA]
    lead_phase = 0x4000_0000
    pan_phase = 0
    pan_increment = 178_957  # about 2 Hz across the stereo field
    noise_state = 0x6D2B_79F5

    for frame in range(FRAME_COUNT):
        chord_increment = _CHORDS[(frame // SAMPLE_RATE) % len(_CHORDS)]
        chord = 0
        for index, increment in enumerate(chord_increment):
            chord_phases[index] = (chord_phases[index] + increment) & _PHASE_MASK
            chord += _triangle(chord_phases[index])
        chord //= len(chord_phases)

        lead_increment = _LEAD[(frame // (SAMPLE_RATE // 2)) % len(_LEAD)]
        lead_phase = (lead_phase + lead_increment) & _PHASE_MASK
        lead = _triangle(lead_phase)

        pan_phase = (pan_phase + pan_increment) & _PHASE_MASK
        pan = _triangle(pan_phase) + _FULL_SCALE
        left_pan = 65_534 - pan
        right_pan = pan

        chord_level, lead_level, percussion_level = _section_levels(frame)
        shared = chord * chord_level // _FULL_SCALE
        left = shared + lead * lead_level * left_pan // (_FULL_SCALE * 65_534)
        right = shared + lead * lead_level * right_pan // (_FULL_SCALE * 65_534)

        noise_state ^= (noise_state << 13) & _PHASE_MASK
        noise_state ^= noise_state >> 17
        noise_state ^= (noise_state << 5) & _PHASE_MASK
        noise_state &= _PHASE_MASK
        hit_frame = frame % 12_000
        if percussion_level and hit_frame < 640:
            envelope = 640 - hit_frame
            noise = ((noise_state >> 16) & 0xFFFF) - 32_768
            burst = noise * percussion_level * envelope // (32_768 * 640)
            click = percussion_level // 2 if hit_frame < 12 else 0
            left += burst + click
            right += -(burst // 2) + click

        struct.pack_into("<hh", payload, frame * 4, _clamp_i16(left), _clamp_i16(right))

    return bytes(payload)


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("output", type=Path, help="destination WAV (never needed by tests)")
    args = parser.parse_args()
    pcm = render_pcm16le()
    args.output.parent.mkdir(parents=True, exist_ok=True)
    with wave.open(str(args.output), "wb") as destination:
        destination.setnchannels(CHANNELS)
        destination.setsampwidth(2)
        destination.setframerate(SAMPLE_RATE)
        destination.writeframes(pcm)
    print(f"{len(pcm)} PCM bytes across {len(pcm) // (2 * CHANNELS)} frames  {args.output}")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
