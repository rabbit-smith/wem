from __future__ import annotations

import tempfile
import unittest
import wave
from pathlib import Path

from wwise_wem.model import PcmBuffer
from wwise_wem.adapters.wav import read_pcm16, read_wav_geometry


class WavAdapterTests(unittest.TestCase):
    def _path(self, directory: str, name: str = "input.wav") -> Path:
        return Path(directory) / name

    def test_read_pcm16_preserves_channel_order_and_legacy_normalization(self):
        with tempfile.TemporaryDirectory() as directory:
            path = self._path(directory)
            interleaved = (
                -32768, 32767,
                -1, 1,
                0, -16384,
            )
            raw = b"".join(value.to_bytes(2, "little", signed=True) for value in interleaved)
            with wave.open(str(path), "wb") as target:
                target.setnchannels(2)
                target.setsampwidth(2)
                target.setframerate(22050)
                target.writeframes(raw)

            pcm = read_pcm16(path)

            self.assertIsInstance(pcm, PcmBuffer)
            self.assertEqual(pcm.sample_rate, 22050)
            self.assertEqual(pcm.channel_count, 2)
            self.assertEqual(pcm.frame_count, 3)
            self.assertEqual(pcm.channels[0], (-1.0, -1 / 32768.0, 0.0))
            self.assertEqual(
                pcm.channels[1],
                (32767 / 32768.0, 1 / 32768.0, -0.5),
            )
            self.assertEqual(read_wav_geometry(path), (2, 22050))

    def test_empty_wav_is_rejected_at_typed_buffer_boundary(self):
        with tempfile.TemporaryDirectory() as directory:
            path = self._path(directory, "empty.wav")
            with wave.open(str(path), "wb") as target:
                target.setnchannels(1)
                target.setsampwidth(2)
                target.setframerate(44100)
                target.writeframes(b"")

            with self.assertRaisesRegex(ValueError, "at least one frame"):
                read_pcm16(path)
            self.assertEqual(read_wav_geometry(path), (1, 44100))

    def test_pcm8_uses_the_existing_signed16_error_contract(self):
        with tempfile.TemporaryDirectory() as directory:
            path = self._path(directory, "pcm8.wav")
            with wave.open(str(path), "wb") as target:
                target.setnchannels(1)
                target.setsampwidth(1)
                target.setframerate(8000)
                target.writeframes(b"\x80\x81")

            with self.assertRaisesRegex(
                ValueError, "uncompressed signed-16 PCM WAV"
            ):
                read_pcm16(path)
            self.assertEqual(read_wav_geometry(path), (1, 8000))


if __name__ == "__main__":
    unittest.main()
