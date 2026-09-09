"""Public contract tests for the extended PCM input entries.

Locks the supported root exports and their user-visible error surface for
the extended input forms (24-bit PCM, 32-bit float32 WAV, raw PCM), and the
explicit short-input rejection on every entry point.
"""

from __future__ import annotations

import json
import os
import subprocess
import sys
import tempfile
import unittest
import wave
from pathlib import Path

from wwise_wem import (
    PcmBuffer,
    encode_pcm_wav,
    encode_raw_pcm,
    encode_wav,
    read_pcm_wav,
    read_raw_pcm,
)

ROOT = Path(__file__).resolve().parents[2]
FIXTURES = ROOT / "tests" / "fixtures"
INPUT = FIXTURES / "input.wav"
PROFILE = "wwise2013-6ch-44100"


class ExtendedInputExportTests(unittest.TestCase):
    def test_new_entries_are_supported_root_exports(self):
        import wwise_wem

        for name in (
            "encode_pcm_wav",
            "encode_raw_pcm",
            "read_pcm_wav",
            "read_raw_pcm",
        ):
            with self.subTest(name=name):
                self.assertIn(name, wwise_wem.__all__)
                self.assertTrue(callable(getattr(wwise_wem, name)))

    def test_fresh_root_import_stays_lazy_for_the_new_adapters(self):
        code = """
import json
import sys
import wwise_wem
names = [
    'wwise_wem.application.encoder',
    'wwise_wem.adapters.wav',
    'wwise_wem.adapters.raw',
    'wwise_wem.adapters.sample_conversion',
]
print(json.dumps([name for name in names if name in sys.modules]))
"""
        environment = dict(os.environ)
        environment["PYTHONPATH"] = str(ROOT / "src") + os.pathsep + str(ROOT / "reference")
        completed = subprocess.run(
            [sys.executable, "-c", code],
            cwd=ROOT,
            env=environment,
            capture_output=True,
            text=True,
            check=True,
        )
        self.assertEqual(json.loads(completed.stdout), [])

    def test_read_entries_return_pcm_buffers(self):
        pcm_wav = read_pcm_wav(INPUT)
        self.assertIsInstance(pcm_wav, PcmBuffer)
        self.assertEqual(pcm_wav.channel_count, 6)
        self.assertEqual(pcm_wav.sample_rate, 44100)
        raw = b"\x00\x00" * 3
        pcm_raw = read_raw_pcm(raw, sample_rate=44100, channels=1, bits_per_sample=16)
        self.assertEqual(pcm_raw.frame_count, 3)
        self.assertEqual(pcm_raw.channels[0], (0.0, 0.0, 0.0))


class ExtendedInputEncodeContractTests(unittest.TestCase):
    @classmethod
    def setUpClass(cls):
        cls.directory = tempfile.TemporaryDirectory()
        with wave.open(str(INPUT), "rb") as source:
            params = source.getparams()
            pcm = source.readframes(2048)
        cls.short_input = Path(cls.directory.name) / "short.wav"
        with wave.open(str(cls.short_input), "wb") as target:
            target.setparams(params)
            target.writeframes(pcm)

    @classmethod
    def tearDownClass(cls):
        cls.directory.cleanup()

    def test_signed16_path_is_byte_identical_on_the_new_entry(self):
        legacy = encode_wav(INPUT)
        extended = encode_pcm_wav(INPUT)
        self.assertEqual(legacy.data, extended.data)
        self.assertEqual(legacy.sha256, extended.sha256)

    def test_short_input_rejection_matches_the_encoder_contract(self):
        for action in (
            lambda: encode_wav(self.short_input),
            lambda: encode_pcm_wav(self.short_input),
        ):
            with self.assertRaisesRegex(ValueError, "at least 4096 frames"):
                action()

    def test_raw_entry_geometry_errors_are_explicit(self):
        with self.assertRaisesRegex(ValueError, "sample_rate must be positive"):
            encode_raw_pcm(
                b"\x00\x00", sample_rate=0, channels=1, bits_per_sample=16
            )
        with self.assertRaises(TypeError):
            encode_raw_pcm(
                b"\x00\x00",
                sample_rate=44100,
                channels=True,
                bits_per_sample=16,
            )
        with self.assertRaisesRegex(ValueError, "bits_per_sample must be"):
            encode_raw_pcm(
                b"\x00\x00",
                sample_rate=44100,
                channels=1,
                bits_per_sample=8,
            )
        with self.assertRaisesRegex(ValueError, "not a multiple of the frame size"):
            encode_raw_pcm(
                b"\x00" * 5,
                sample_rate=44100,
                channels=2,
                bits_per_sample=16,
            )

    def test_raw_entry_short_input_keeps_the_explicit_rejection(self):
        # 6 channels x 4000 frames: aligned, non-empty, still too short.
        data = b"\x00\x00" * (6 * 4000)
        with self.assertRaisesRegex(ValueError, "at least 4096 frames"):
            encode_raw_pcm(
                data, sample_rate=44100, channels=6, bits_per_sample=16
            )

    def test_unsupported_geometry_resolves_to_the_profile_error(self):
        # 4ch/48000 is not a registered profile (the 2ch profile is 48000).
        data = b"\x00\x00" * (4 * 4096)
        with self.assertRaisesRegex(ValueError, "no Wwise 2013.2 profile"):
            encode_raw_pcm(
                data, sample_rate=48000, channels=4, bits_per_sample=16
            )


if __name__ == "__main__":
    unittest.main()
