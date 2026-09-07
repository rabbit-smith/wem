"""Phase-0 contract tests for the intentionally supported public surface.

These tests freeze user-visible behavior, not the current internal module
layout.  Refactors may freely replace the implementation behind this API.
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
    ProfileKey,
    WwiseVorbisProfile,
    encode_wav,
    load_wem_profile,
    resolve_wem_profile,
)


ROOT = Path(__file__).resolve().parents[2]
FIXTURES = Path(__file__).resolve().parents[1] / "fixtures"
INPUT = FIXTURES / "input.wav"
PROFILE = "wwise2013-6ch-44100"


class PublicProfileContractTests(unittest.TestCase):
    def test_automatic_and_named_profile_resolution(self):
        automatic = resolve_wem_profile(6, 44100)
        explicit = load_wem_profile(PROFILE)
        self.assertIsInstance(automatic, WwiseVorbisProfile)
        self.assertEqual(automatic, explicit)
        self.assertEqual(ProfileKey(6, 44100), ProfileKey(automatic.channels, automatic.sample_rate))
        self.assertEqual(automatic.name, PROFILE)
        self.assertEqual(len(automatic.setup_packet()), 201)

    def test_unknown_profile_name_and_geometry_are_errors(self):
        with self.assertRaisesRegex(ValueError, "unknown WEM profile"):
            load_wem_profile("missing-profile")
        with self.assertRaisesRegex(ValueError, "no Wwise 2013.2 profile"):
            resolve_wem_profile(2, 48000)


class PublicEncodeContractTests(unittest.TestCase):
    @classmethod
    def setUpClass(cls):
        cls.directory = tempfile.TemporaryDirectory()
        cls.short_input = Path(cls.directory.name) / "short.wav"
        with wave.open(str(INPUT), "rb") as source:
            params = source.getparams()
            pcm = source.readframes(4096)
        with wave.open(str(cls.short_input), "wb") as target:
            target.setparams(params)
            target.writeframes(pcm)
        cls.automatic_result = encode_wav(cls.short_input)
        cls.explicit_result = encode_wav(cls.short_input, profile=PROFILE)

    @classmethod
    def tearDownClass(cls):
        cls.directory.cleanup()

    def test_automatic_and_explicit_encoding_are_identical(self):
        self.assertEqual(
            self.automatic_result.data, self.explicit_result.data
        )
        self.assertEqual(
            self.automatic_result.stats, self.explicit_result.stats
        )

    def test_encoder_is_a_lazy_canonical_root_export(self):
        import wwise_wem
        from wwise_wem.application.encoder import Encoder as CanonicalEncoder

        self.assertIn("Encoder", wwise_wem.__all__)
        self.assertIn("Encoder", dir(wwise_wem))
        self.assertIs(wwise_wem.Encoder, CanonicalEncoder)

    def test_packet_counts_are_internally_consistent(self):
        stats = self.automatic_result.stats
        self.assertEqual(
            stats.short_packets + stats.long_packets,
            stats.audio_packets,
        )

    def test_unsupported_wav_geometry_is_an_error(self):
        with tempfile.TemporaryDirectory() as directory:
            path = Path(directory) / "stereo.wav"
            with wave.open(str(path), "wb") as target:
                target.setnchannels(2)
                target.setsampwidth(2)
                target.setframerate(48000)
                target.writeframes(b"\0" * (4096 * 2 * 2))
            with self.assertRaisesRegex(ValueError, "no Wwise 2013.2 profile"):
                encode_wav(path)

    def test_non_s16_pcm_wav_is_an_error(self):
        with tempfile.TemporaryDirectory() as directory:
            path = Path(directory) / "pcm8.wav"
            with wave.open(str(path), "wb") as target:
                target.setnchannels(6)
                target.setsampwidth(1)
                target.setframerate(44100)
                target.writeframes(b"\x80" * (32 * 6))
            with self.assertRaisesRegex(ValueError, "signed-16 PCM WAV"):
                encode_wav(path)


class PublicCliContractTests(unittest.TestCase):
    def run_cli(self, *arguments: str) -> subprocess.CompletedProcess[str]:
        return subprocess.run(
            [sys.executable, "-m", "wwise_wem", *arguments],
            cwd=ROOT,
            capture_output=True,
            text=True,
        )

    def test_help_exposes_supported_options(self):
        result = self.run_cli("--help")
        self.assertEqual(result.returncode, 0, result.stderr)
        self.assertIn("usage:", result.stdout)
        for option in (
            "--profile",
            "--wwise-version",
            "--channels",
            "--sample-rate",
            "--output",
            "--expect-sha256",
        ):
            self.assertIn(option, result.stdout)
        self.assertIn(PROFILE, result.stdout)

    def test_output_is_required(self):
        result = self.run_cli(str(INPUT))
        self.assertNotEqual(result.returncode, 0)
        self.assertIn("--output", result.stderr)

    def test_cli_geometry_assertion_fails_before_output(self):
        with tempfile.TemporaryDirectory() as directory:
            output = Path(directory) / "unused.wem"
            result = self.run_cli(
                str(INPUT), "--channels", "2", "--output", str(output)
            )
            self.assertNotEqual(result.returncode, 0)
            self.assertIn("differs from WAV", result.stderr)
            self.assertFalse(output.exists())


class PublicImportBoundaryTests(unittest.TestCase):
    def test_fresh_root_import_keeps_encoder_and_codec_runtime_lazy(self):
        code = """
import json
import sys
import wwise_wem
names = [
    'wwise_wem.application.encoder',
    'wwise_wem.application.compat',
    'wwise_wem_reference.python_engine',
    'wwise_wem_reference.analysis.dsp.transform',
    'wwise_wem_reference.analysis.psychoacoustics.pipeline',
    'wwise_wem_reference.vorbis.floor',
    'wwise_wem_reference.vorbis.residue',
    'wwise_wem_reference.analysis.session',
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


if __name__ == "__main__":
    unittest.main()
