from __future__ import annotations

import struct
import tempfile
import unittest
import wave
from pathlib import Path

from wwise_wem.application.compat import read_pcm16_wav


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


if __name__ == "__main__":
    unittest.main()
