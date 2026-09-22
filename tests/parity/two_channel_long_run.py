"""Cross-implementation stability check for long 2ch/48 kHz input.

This intentionally lacks a ``test_`` prefix: the pure-Python oracle makes it a
heavy explicit run, following ``tests/AGENTS.md``.  What it checks is the
*agreement* of the engines on one long program: every check below compares
produced bytes against another producer (the Python oracle, the public WAV
path, and the streaming lifecycle), and the 2ch/48 kHz configuration's
absolute byte pin is the committed real-build corpora (``tests/data/2ch-reference``,
``tests/data/2ch-stress``).  The program's own identity is its generator's
parameters plus the recorded PCM words below -- there is no committed PCM
artifact for it (the generator deliberately ships no multi-megabyte WAV; see
:mod:`scripts.generate_2ch_long_program`), and a digest over the generated
bytes would only repeat what those words already state.
"""

from __future__ import annotations

import itertools
import struct
import tempfile
import unittest
import wave
from pathlib import Path

import wwise_wem as W
from scripts.generate_2ch_long_program import (
    CHANNELS,
    DURATION_SECONDS,
    FRAME_COUNT,
    SAMPLE_RATE,
    render_pcm16le,
)
from tests.parity.wem_byte_compare import assert_wem_equal
from wwise_wem import WwiseProfile, WwiseVersion, _core
from wwise_wem_reference.profiles.artifact import resolve_selection
from wwise_wem.adapters.raw import read_raw_pcm
from wwise_wem_reference.container.wem import load_wem_parts_bytes
from wwise_wem_reference.python_engine import ContainerPlan, encode_pcm_python


SELECTION = WwiseProfile(WwiseVersion.WWISE2013, CHANNELS, SAMPLE_RATE)
# The generated program's identity: the generator's parameters (fixed-point
# phase increments and a fixed xorshift seed, so the signal has no platform
# math and no run-time randomness) and one recorded interleaved frame pair at
# each act boundary, as the little-endian signed 16-bit words it emits.
GENERATOR_PARAMETERS = (48_000, 2, 20, 960_000)
RECORDED_PCM_WORDS = {
    0: (-1432, -689),
    4 * SAMPLE_RATE - 1: (1181, 1181),
    8 * SAMPLE_RATE - 1: (-722, -1942),
    12 * SAMPLE_RATE - 1: (-1709, 1297),
    16 * SAMPLE_RATE - 1: (-1164, -136),
    FRAME_COUNT - 1: (3804, 2465),
}
WEM_BYTES = 300_592
AUDIO_PACKETS = 1_685
SHORT_PACKETS = 853
LONG_PACKETS = 832


def _iter_chunks(payload: bytes, frame_pattern: tuple[int, ...]):
    offset = 0
    sizes = itertools.cycle(frame_pattern)
    while offset < len(payload):
        end = min(len(payload), offset + next(sizes) * CHANNELS * 2)
        yield payload[offset:end]
        offset = end


class TwoChannelLongRunTests(unittest.TestCase):
    def test_twenty_second_music_program_is_stable_across_all_engines(self) -> None:
        raw = render_pcm16le()
        self.assertEqual(
            (SAMPLE_RATE, CHANNELS, DURATION_SECONDS, FRAME_COUNT), GENERATOR_PARAMETERS
        )
        self.assertEqual(len(raw), FRAME_COUNT * CHANNELS * 2)
        for frame, expected in RECORDED_PCM_WORDS.items():
            self.assertEqual(
                struct.unpack_from("<hh", raw, frame * CHANNELS * 2), expected
            )

        selection = SELECTION
        profile = resolve_selection(selection)
        pcm = read_raw_pcm(
            raw,
            sample_rate=SAMPLE_RATE,
            channels=CHANNELS,
            bits_per_sample=16,
        )
        native_result = W.encode(W.RawPcm(raw, SAMPLE_RATE, CHANNELS, "s16le"))
        native = bytes(native_result.data)
        oracle = bytes(
            encode_pcm_python(
                profile=profile,
                container=ContainerPlan.from_profile(profile),
                pcm=pcm,
            ).data
        )

        self.assertEqual(len(native), WEM_BYTES)
        self.assertEqual(native_result.stats.audio_packets, AUDIO_PACKETS)
        self.assertEqual(native_result.stats.short_packets, SHORT_PACKETS)
        self.assertEqual(native_result.stats.long_packets, LONG_PACKETS)
        audio_packets = load_wem_parts_bytes(native)["packets"][1:]
        modes = tuple(packet[0] & 1 for packet in audio_packets)
        self.assertEqual(len(audio_packets), AUDIO_PACKETS)
        self.assertEqual(modes.count(0), SHORT_PACKETS)
        self.assertEqual(modes.count(1), LONG_PACKETS)
        assert_wem_equal(self, native, oracle, "Python oracle")

        with tempfile.TemporaryDirectory() as directory:
            wav_path = Path(directory) / "long-program.bin"
            with wave.open(str(wav_path), "wb") as target:
                target.setnchannels(CHANNELS)
                target.setsampwidth(2)
                target.setframerate(SAMPLE_RATE)
                target.writeframes(raw)
            from_path = W.encode(wav_path)
        assert_wem_equal(self, native, from_path.data, "public WAV path")

        for label, pattern in (
            ("single chunk", (FRAME_COUNT,)),
            ("prime chunks", (997,)),
            ("irregular chunks", (1, 7, 31, 257, 4_093, 8_191)),
        ):
            with self.subTest(streaming=label):
                stream = _core.StreamSession.for_selection(selection)
                for chunk in _iter_chunks(raw, pattern):
                    stream.push(chunk)
                streamed = bytes(stream.finish().bytes)
                assert_wem_equal(self, native, streamed, label)


if __name__ == "__main__":
    unittest.main()
