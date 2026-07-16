from __future__ import annotations

import struct
import tempfile
import unittest
import wave
from pathlib import Path
from unittest.mock import patch

from wwise_wem.application.compat import (
    encode_wav_to_wem,
    read_pcm16_wav,
    read_wav_geometry,
)
from wwise_wem.application.models import EncodeResult, EncodeStats


def _result() -> EncodeResult:
    return EncodeResult(
        b"WEM!",
        EncodeStats(16, 6, 2, 1, 1, 4, "profile:wwise2013-6ch-44100"),
    )


class LegacyFacadeTests(unittest.TestCase):
    def test_read_pcm16_wav_preserves_zero_frame_rows(self) -> None:
        with tempfile.TemporaryDirectory() as directory:
            path = Path(directory) / "empty.wav"
            with wave.open(str(path), "wb") as target:
                target.setnchannels(6)
                target.setsampwidth(2)
                target.setframerate(44100)
                target.writeframes(b"")
            self.assertEqual(
                read_pcm16_wav(path),
                (44100, 0, [[] for _ in range(6)]),
            )

    def test_read_pcm16_wav_preserves_signed_normalization(self) -> None:
        values = (-32768, -1, 0, 1, 32767, 16384)
        with tempfile.TemporaryDirectory() as directory:
            path = Path(directory) / "edges.wav"
            with wave.open(str(path), "wb") as target:
                target.setnchannels(6)
                target.setsampwidth(2)
                target.setframerate(44100)
                target.writeframes(struct.pack("<6h", *values))
            self.assertEqual(
                read_pcm16_wav(path),
                (
                    44100,
                    1,
                    [[-1.0], [-1 / 32768], [0.0], [1 / 32768], [32767 / 32768], [0.5]],
                ),
            )

    def test_encode_wav_to_wem_only_adapts_typed_result(self) -> None:
        typed = _result()
        with patch("wwise_wem.api.encode_wav", return_value=typed) as encode:
            legacy = encode_wav_to_wem(
                Path("input.wav"),
                Path("template.wem"),
                profile=None,
            )
        encode.assert_called_once_with(
            Path("input.wav"), Path("template.wem"), profile=None
        )
        self.assertEqual(legacy, typed.to_legacy_tuple())
        legacy[1]["bytes"] = 99
        self.assertEqual(typed.stats.bytes, 4)

    def test_geometry_is_a_thin_delegate(self) -> None:
        with patch(
            "wwise_wem.adapters.wav.read_wav_geometry",
            return_value=(6, 44100),
        ) as geometry:
            self.assertEqual(read_wav_geometry(Path("input.wav")), (6, 44100))
        geometry.assert_called_once_with(Path("input.wav"))


if __name__ == "__main__":
    unittest.main()
