"""Parity tests for the small, import-light package facade."""

from __future__ import annotations

import json
import os
import subprocess
import sys
import unittest
from pathlib import Path
from unittest.mock import patch

import wwise_wem
from wwise_wem import (
    EncodeResult,
    EncodeStats,
    PcmBuffer,
    RawPcm,
    WwiseProfile,
    WwiseVersion,
    encode,
)


ROOT = Path(__file__).resolve().parents[2]


def _result() -> EncodeResult:
    return EncodeResult(
        b"RIFF",
        EncodeStats(16, 6, 2, 1, 1, 4, "profile:6ch/44100Hz/2013"),
    )


class TypedApiTests(unittest.TestCase):
    def test_path_input_reads_header_and_forwards_the_selection(self) -> None:
        payload = b"\0\0" * 6
        selection = WwiseProfile(WwiseVersion.WWISE2013, 6, 44100)
        expected = _result()
        with (
            patch(
                "wwise_wem.adapters.wav._read_wav_pcm16_bytes",
                return_value=(44100, 6, payload),
            ) as read,
            patch("wwise_wem.application.encoder.Encoder") as encoder_type,
        ):
            encoder_type.return_value.encode_pcm16_interleaved.return_value = expected
            result = encode(Path("input.audio"), profile=selection, quality=5.0)

        read.assert_called_once_with(Path("input.audio"))
        encoder_type.assert_called_once_with(selection, quality=5.0)
        encoder_type.return_value.encode_pcm16_interleaved.assert_called_once_with(
            payload,
            sample_rate=44100,
            channels=6,
        )
        self.assertIs(result, expected)

    def test_automatic_selection_comes_from_the_input_geometry(self) -> None:
        payload = b"\0\0" * 6
        expected = _result()
        with (
            patch(
                "wwise_wem.adapters.wav._read_wav_pcm16_bytes",
                return_value=(44100, 6, payload),
            ),
            patch("wwise_wem.application.encoder.Encoder") as encoder_type,
        ):
            encoder_type.return_value.encode_pcm16_interleaved.return_value = expected
            result = encode(Path("input.audio"))

        encoder_type.assert_called_once_with(
            WwiseProfile(WwiseVersion.DEFAULT, 6, 44100), quality=None
        )
        self.assertIs(result, expected)

    def test_pcm_buffer_bypasses_input_adapters(self) -> None:
        pcm = PcmBuffer(44100, ((0.0,),) * 6)
        expected = _result()
        with (
            patch("wwise_wem.adapters.wav._read_wav_pcm16_bytes") as read_wav,
            patch("wwise_wem.adapters.raw._normalize_pcm16_bytes") as read_raw,
            patch("wwise_wem.application.encoder.Encoder") as encoder_type,
        ):
            encoder_type.return_value.encode_pcm.return_value = expected
            self.assertIs(encode(pcm), expected)
        read_wav.assert_not_called()
        read_raw.assert_not_called()

    def test_raw_pcm_carries_explicit_format_and_geometry(self) -> None:
        source = RawPcm(b"\0\0" * 6, 44100, 6, "s16le")
        with (
            patch(
                "wwise_wem.adapters.raw._normalize_pcm16_bytes",
                return_value=source.data,
            ) as read,
            patch("wwise_wem.application.encoder.Encoder") as encoder_type,
        ):
            encoder_type.return_value.encode_pcm16_interleaved.return_value = _result()
            encode(source)
        read.assert_called_once_with(
            source.data,
            sample_rate=44100,
            channels=6,
            bits_per_sample=16,
        )
        encoder_type.return_value.encode_pcm16_interleaved.assert_called_once_with(
            source.data,
            sample_rate=44100,
            channels=6,
        )

    def test_untyped_bytes_are_rejected(self) -> None:
        with self.assertRaisesRegex(TypeError, "path, PcmBuffer, or RawPcm"):
            encode(b"\0\0")  # type: ignore[arg-type]

    def test_package_root_has_one_small_public_surface(self) -> None:
        self.assertEqual(
            wwise_wem.__all__,
            [
                "EncodeResult",
                "EncodeStats",
                "PcmBuffer",
                "RawPcm",
                "WwiseProfile",
                "WwiseVersion",
                "encode",
            ],
        )

    def test_fresh_package_import_keeps_runtime_modules_lazy(self) -> None:
        code = """
import json
import sys
import wwise_wem
names = [
    'wwise_wem.application.encoder',
    'wwise_wem.adapters.wav',
    'wwise_wem.adapters.raw',
    'wwise_wem_reference.python_engine',
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
