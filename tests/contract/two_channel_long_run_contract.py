"""Cross-implementation stability contract for long 2ch/48 kHz input.

This intentionally lacks a ``test_`` prefix: the pure-Python oracle makes it a
heavy explicit contract, following ``tests/AGENTS.md``.  The expected WEM is a
native/oracle agreement and is not represented as a real Wwise build.
"""

from __future__ import annotations

import hashlib
import itertools
import tempfile
import unittest
import wave
from pathlib import Path

import wwise_wem as W
from scripts.generate_2ch_long_program import (
    CHANNELS,
    FRAME_COUNT,
    SAMPLE_RATE,
    render_pcm16le,
)
from tests.contract.wem_byte_contract import assert_wem_equal
from wwise_wem import _core
from wwise_wem.profiles.registry import load_wem_profile
from wwise_wem.adapters.raw import read_raw_pcm
from wwise_wem_reference.container.wem import load_wem_parts_bytes
from wwise_wem_reference.python_engine import ContainerPlan, encode_pcm_python


PROFILE = "wwise2013-2ch-48000"
PCM_SHA256 = "89b916a151ad1d42d313efdc2ae970a2671fee9b5c5b979aeb9982f7b1f85554"
WEM_SHA256 = "bac1214584a57a1e9ae013cdee2d17671a2a63bda0f78ec8d064e3b41db17290"
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


class TwoChannelLongRunContract(unittest.TestCase):
    def test_twenty_second_music_program_is_stable_across_all_engines(self) -> None:
        raw = render_pcm16le()
        self.assertEqual(len(raw), FRAME_COUNT * CHANNELS * 2)
        self.assertEqual(hashlib.sha256(raw).hexdigest(), PCM_SHA256)

        profile = load_wem_profile(PROFILE)
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
        self.assertEqual(hashlib.sha256(native).hexdigest(), WEM_SHA256)
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
                stream = _core.StreamSession()
                stream.start(profile.setup_sha256, profile.name)
                for chunk in _iter_chunks(raw, pattern):
                    stream.push(chunk)
                streamed = bytes(stream.finish().bytes)
                assert_wem_equal(self, native, streamed, label)


if __name__ == "__main__":
    unittest.main()
