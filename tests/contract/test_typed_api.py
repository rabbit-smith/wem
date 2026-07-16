"""Contract tests for the typed, import-light package façade."""

from __future__ import annotations

import json
import os
import subprocess
import sys
import unittest
from pathlib import Path
from unittest.mock import patch

import wwise2013_wem
from wwise2013_wem import EncodeResult, EncodeStats, PcmBuffer, encode_wav
from wwise2013_wem.profiles.registry import load_wem_profile


ROOT = Path(__file__).resolve().parents[2]
LEGACY_STATS = {
    "pcm_frames": 16,
    "channels": 6,
    "audio_packets": 2,
    "short_packets": 1,
    "long_packets": 1,
    "bytes": 4,
    "metadata_source": "profile:wwise2013-6ch-44100",
}


class TypedApiTests(unittest.TestCase):
    def test_encode_wav_reads_pcm_resolves_profile_and_calls_encoder(self):
        pcm = PcmBuffer(44100, ((0.0,),) * 6)
        profile = load_wem_profile("wwise2013-6ch-44100")
        expected = EncodeResult(b"RIFF", EncodeStats.from_legacy_dict(LEGACY_STATS))
        with (
            patch("wwise2013_wem.adapters.wav.read_pcm16", return_value=pcm) as read,
            patch(
                "wwise2013_wem.profiles.registry.load_wem_profile",
                return_value=profile,
            ) as resolve,
            patch("wwise2013_wem.application.encoder.Encoder") as encoder_type,
        ):
            encoder_type.return_value.encode_pcm.return_value = expected
            result = encode_wav(
                Path("input.wav"), profile="wwise2013-6ch-44100"
            )

        read.assert_called_once_with(Path("input.wav"))
        resolve.assert_called_once_with("wwise2013-6ch-44100")
        encoder_type.assert_called_once_with(profile, _container=None)
        encoder_type.return_value.encode_pcm.assert_called_once_with(pcm)
        self.assertIs(result, expected)

    def test_encode_wav_does_not_import_the_legacy_encoder_module(self):
        code = """
import json
import sys
from pathlib import Path
from unittest.mock import patch
from wwise2013_wem import EncodeResult, EncodeStats, PcmBuffer, encode_wav
from wwise2013_wem.profiles.registry import load_wem_profile
pcm = PcmBuffer(44100, ((0.0,),) * 6)
stats = EncodeStats(1, 6, 1, 1, 0, 4, 'profile:test')
with patch('wwise2013_wem.adapters.wav.read_pcm16', return_value=pcm), \
     patch('wwise2013_wem.profiles.registry.load_wem_profile', return_value=load_wem_profile('wwise2013-6ch-44100')), \
     patch('wwise2013_wem.application.encoder.Encoder') as encoder:
    encoder.return_value.encode_pcm.return_value = EncodeResult(b'RIFF', stats)
    encode_wav(Path('input.wav'), profile='wwise2013-6ch-44100')
print(json.dumps('wwise2013_wem.application.compat' in sys.modules))
"""
        env = dict(os.environ)
        env["PYTHONPATH"] = str(ROOT / "src")
        completed = subprocess.run(
            [sys.executable, "-c", code],
            cwd=ROOT,
            env=env,
            capture_output=True,
            text=True,
            check=True,
        )
        self.assertFalse(json.loads(completed.stdout))

    def test_legacy_root_exports_remain_callable_and_tuple_shaped(self):
        with patch(
            "wwise2013_wem.application.compat.encode_wav_to_wem",
            return_value=(b"RIFF", LEGACY_STATS),
        ):
            legacy_encode = wwise2013_wem.encode_wav_to_wem
            data, stats = legacy_encode(Path("input.wav"))
        self.assertEqual(data, b"RIFF")
        self.assertEqual(stats, LEGACY_STATS)
        self.assertTrue(callable(wwise2013_wem.read_pcm16_wav))

    def test_package_root_exports_models_and_profile_api(self):
        for name in (
            "PcmBuffer",
            "ContainerMetadata",
            "SetupConfig",
            "EncodeStats",
            "EncodeResult",
            "PacketResult",
            "EncoderProfile",
            "ProfileKey",
            "ProfileRegistry",
            "WwiseVorbisProfile",
            "load_wem_profile",
            "resolve_wem_profile",
            "encode_wav",
        ):
            self.assertIn(name, wwise2013_wem.__all__)
            self.assertTrue(hasattr(wwise2013_wem, name), name)

    def test_fresh_package_import_does_not_load_heavy_codec_modules(self):
        code = """
import json
import sys
import wwise2013_wem
names = [
    'wwise2013_wem.application.compat',
    'wwise2013_wem.application.encoder',
    'wwise2013_wem.adapters.wav',
    'wwise2013_wem.analysis.dsp.transform',
    'wwise2013_wem.analysis.psychoacoustics.pipeline',
    'wwise2013_wem.vorbis.floor',
    'wwise2013_wem.vorbis.residue',
    'wwise2013_wem.analysis.session',
]
print(json.dumps([name for name in names if name in sys.modules]))
"""
        env = dict(os.environ)
        env["PYTHONPATH"] = str(ROOT / "src")
        completed = subprocess.run(
            [sys.executable, "-c", code],
            cwd=ROOT,
            env=env,
            capture_output=True,
            text=True,
            check=True,
        )
        self.assertEqual(json.loads(completed.stdout), [])


if __name__ == "__main__":
    unittest.main()
