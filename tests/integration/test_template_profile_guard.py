from __future__ import annotations

import tempfile
import unittest
import wave
from pathlib import Path

from wwise2013_wem import encode_wav, load_wem_profile
from wwise2013_wem.container.wem import build_vorbis_wem


FIXTURES = Path(__file__).resolve().parents[1] / "fixtures"
PROFILE_NAME = "wwise2013-6ch-44100"


def _write_wav(path: Path, *, channels: int = 6, sample_rate: int = 44100, frames: int = 1) -> None:
    with wave.open(str(path), "wb") as target:
        target.setnchannels(channels)
        target.setsampwidth(2)
        target.setframerate(sample_rate)
        target.writeframes(b"\x00\x00" * channels * frames)


def _write_template(path: Path, *, setup: bytes, channels: int = 6, sample_rate: int = 44100) -> None:
    profile = load_wem_profile(PROFILE_NAME)
    fmt = dict(profile.fmt)
    fmt["nChannels"] = channels
    fmt["nSamplesPerSec"] = sample_rate
    path.write_bytes(
        build_vorbis_wem(
            fmt,
            [setup],
            seek_table=profile.seek_table,
            endian=profile.endian,
            extra_chunks=list(profile.extra_chunks),
            recompute_sizes=True,
        )
    )


class TemplateProfileGuardTests(unittest.TestCase):
    def test_matching_profile_setup_template_encodes(self):
        profile = load_wem_profile(PROFILE_NAME)
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            template = root / "template.wem"
            wav = root / "input.wav"
            _write_template(template, setup=profile.setup_packet())
            with wave.open(str(FIXTURES / "input.wav"), "rb") as source:
                params = source.getparams()
                pcm = source.readframes(4096)
            with wave.open(str(wav), "wb") as target:
                target.setparams(params)
                target.writeframes(pcm)

            result = encode_wav(wav, template)

        self.assertTrue(result.data.startswith(b"RIFF"))
        self.assertGreater(len(result.data), len(profile.setup_packet()))
        self.assertEqual(result.stats.channels, 6)
        self.assertEqual(result.stats.pcm_frames, 4096)
        self.assertTrue(result.stats.metadata_source.startswith("template:"))

    def test_every_changed_setup_byte_is_rejected_before_analysis(self):
        profile = load_wem_profile(PROFILE_NAME)
        original = profile.setup_packet()
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            template = root / "template.wem"
            wav = root / "one-frame.wav"
            _write_wav(wav)

            for offset in range(len(original)):
                changed = bytearray(original)
                changed[offset] ^= 0x01
                _write_template(template, setup=bytes(changed))
                with self.subTest(offset=offset):
                    with self.assertRaises(ValueError) as caught:
                        encode_wav(wav, template)
                    message = str(caught.exception).lower()
                    self.assertIn("setup", message)
                    self.assertIn("sha", message)
                    self.assertNotIn("mode selection", message)

    def test_unknown_setup_sha_reports_geometry_and_installed_profile(self):
        profile = load_wem_profile(PROFILE_NAME)
        changed = bytearray(profile.setup_packet())
        changed[len(changed) // 2] ^= 0x80
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            template = root / "unknown-setup.wem"
            wav = root / "one-frame.wav"
            _write_template(template, setup=bytes(changed))
            _write_wav(wav)

            with self.assertRaises(ValueError) as caught:
                encode_wav(wav, template)

        message = str(caught.exception)
        self.assertIn("setup", message.lower())
        self.assertIn("sha", message.lower())
        self.assertIn("6ch/44100Hz", message)
        self.assertIn(PROFILE_NAME, message)

    def test_template_channel_or_rate_mismatch_is_rejected(self):
        profile = load_wem_profile(PROFILE_NAME)
        cases = ((2, 44100), (6, 48000))
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            wav = root / "one-frame.wav"
            template = root / "mismatch.wem"
            _write_wav(wav)
            for channels, sample_rate in cases:
                _write_template(
                    template,
                    setup=profile.setup_packet(),
                    channels=channels,
                    sample_rate=sample_rate,
                )
                with self.subTest(channels=channels, sample_rate=sample_rate):
                    with self.assertRaisesRegex(
                        ValueError,
                        "channel count/sample rate differs|profile.*geometry|geometry.*profile",
                    ):
                        encode_wav(wav, template)


if __name__ == "__main__":
    unittest.main()
