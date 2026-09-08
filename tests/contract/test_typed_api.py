"""Contract tests for the typed, import-light package façade."""

from __future__ import annotations

import json
import os
import subprocess
import sys
import unittest
from pathlib import Path
from unittest.mock import patch

import wwise_wem
from wwise_wem import EncodeResult, EncodeStats, PcmBuffer, encode_wav
from wwise_wem.profiles.registry import load_wem_profile


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
            patch("wwise_wem.adapters.wav.read_pcm16", return_value=pcm) as read,
            patch(
                "wwise_wem.profiles.registry.load_wem_profile",
                return_value=profile,
            ) as resolve,
            patch("wwise_wem.application.encoder.Encoder") as encoder_type,
        ):
            encoder_type.return_value.encode_pcm.return_value = expected
            result = encode_wav(
                Path("input.wav"), profile="wwise2013-6ch-44100"
            )

        read.assert_called_once_with(Path("input.wav"))
        resolve.assert_called_once_with("wwise2013-6ch-44100", quality=None)
        encoder_type.assert_called_once_with(profile)
        encoder_type.return_value.encode_pcm.assert_called_once_with(pcm)
        self.assertIs(result, expected)

    def test_encode_wav_does_not_import_the_legacy_encoder_module(self):
        code = """
import json
import sys
from pathlib import Path
from unittest.mock import patch
from wwise_wem import EncodeResult, EncodeStats, PcmBuffer, encode_wav
from wwise_wem.profiles.registry import load_wem_profile
pcm = PcmBuffer(44100, ((0.0,),) * 6)
stats = EncodeStats(1, 6, 1, 1, 0, 4, 'profile:test')
with patch('wwise_wem.adapters.wav.read_pcm16', return_value=pcm), \
     patch('wwise_wem.profiles.registry.load_wem_profile', return_value=load_wem_profile('wwise2013-6ch-44100')), \
     patch('wwise_wem.application.encoder.Encoder') as encoder:
    encoder.return_value.encode_pcm.return_value = EncodeResult(b'RIFF', stats)
    encode_wav(Path('input.wav'), profile='wwise2013-6ch-44100')
print(json.dumps('wwise_wem.application.compat' in sys.modules))
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

    def test_read_pcm16_wav_is_a_lazy_root_export(self):
        from wwise_wem.application.compat import read_pcm16_wav as canonical

        self.assertIs(wwise_wem.read_pcm16_wav, canonical)
        self.assertTrue(callable(wwise_wem.read_pcm16_wav))

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
            self.assertIn(name, wwise_wem.__all__)
            self.assertTrue(hasattr(wwise_wem, name), name)

    def test_fresh_package_import_does_not_load_heavy_codec_modules(self):
        code = """
import json
import sys
import wwise_wem
names = [
    'wwise_wem.application.compat',
    'wwise_wem.application.encoder',
    'wwise_wem.adapters.wav',
    'wwise_wem_reference.analysis.dsp.transform',
    'wwise_wem_reference.analysis.psychoacoustics.pipeline',
    'wwise_wem_reference.vorbis.floor',
    'wwise_wem_reference.vorbis.residue',
    'wwise_wem_reference.analysis.session',
]
print(json.dumps([name for name in names if name in sys.modules]))
"""
        env = dict(os.environ)
        env["PYTHONPATH"] = str(ROOT / "src") + os.pathsep + str(ROOT / "reference")
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
