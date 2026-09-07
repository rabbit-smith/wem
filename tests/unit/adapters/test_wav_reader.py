"""Boundary tests for the extended WAV reader (16/24-bit, float32)."""

from __future__ import annotations

import struct
import tempfile
import unittest
import wave
from pathlib import Path

from wwise_wem.adapters.sample_conversion import sample24_to_int16
from wwise_wem.adapters.wav import read_pcm16, read_pcm_wav
from wwise_wem.model import PcmBuffer


def _int16_bytes(*values: int) -> bytes:
    return b"".join(v.to_bytes(2, "little", signed=True) for v in values)


def _int24_bytes(*values: int) -> bytes:
    return b"".join((v & 0xFFFFFF).to_bytes(3, "little") for v in values)


def _float32_bytes(*values: float) -> bytes:
    return struct.pack(f"<{len(values)}f", *values)


def _write_riff(path: Path, *, fmt_tag: int, channels: int, rate: int,
                bits: int, data: bytes, fmt_bytes: bytes | None = None) -> None:
    if fmt_bytes is None:
        fmt_bytes = struct.pack(
            "<HHIIHH", fmt_tag, channels, rate, rate * channels * bits // 8,
            channels * bits // 8, bits,
        )
    with open(path, "wb") as handle:
        handle.write(b"RIFF")
        handle.write(struct.pack("<I", 36 + len(data)))
        handle.write(b"WAVE")
        handle.write(b"fmt ")
        handle.write(struct.pack("<I", len(fmt_bytes)))
        handle.write(fmt_bytes)
        handle.write(b"data")
        handle.write(struct.pack("<I", len(data)))
        handle.write(data)


class ReadPcmWavFormatTests(unittest.TestCase):
    def _tmp(self, name: str) -> Path:
        return Path(tempfile.mkdtemp()) / name

    def test_int16_matches_the_legacy_reader(self):
        values = (-32768, 32767, -1, 1, 0, -16384)
        path = self._tmp("a16.wav")
        with wave.open(str(path), "wb") as target:
            target.setnchannels(2)
            target.setsampwidth(2)
            target.setframerate(22050)
            target.writeframes(_int16_bytes(*values))
        pcm = read_pcm_wav(path)
        self.assertIsInstance(pcm, PcmBuffer)
        self.assertEqual(pcm, read_pcm16(path))
        self.assertEqual(
            pcm.channels,
            ((-1.0, -1 / 32768.0, 0.0), (32767 / 32768.0, 1 / 32768.0, -0.5)),
        )

    def test_int24_wav_converts_with_documented_rules(self):
        samples = (-8388608, 8388607, 128, -128, 0)
        path = self._tmp("a24.wav")
        with wave.open(str(path), "wb") as target:
            target.setnchannels(1)
            target.setsampwidth(3)
            target.setframerate(44100)
            target.writeframes(_int24_bytes(*samples))
        pcm = read_pcm_wav(path)
        self.assertEqual(pcm.sample_rate, 44100)
        self.assertEqual(pcm.frame_count, 5)
        expected = tuple(
            int16 / 32768.0
            for int16 in map(sample24_to_int16, samples)
        )
        self.assertEqual(pcm.channels[0], expected)
        self.assertEqual(expected[0], -1.0)
        self.assertEqual(expected[1], 32767 / 32768.0)

    def test_float32_wav_converts_with_documented_rules(self):
        values = (1.0, -1.0, 0.25, -0.75, 0.0, 2.0, -2.0)
        path = self._tmp("a32f.wav")
        _write_riff(
            path,
            fmt_tag=3,
            channels=1,
            rate=48000,
            bits=32,
            data=_float32_bytes(*values),
        )
        pcm = read_pcm_wav(path)
        self.assertEqual(pcm.sample_rate, 48000)
        self.assertEqual(
            pcm.channels[0],
            (
                32767 / 32768.0,
                -1.0,
                8192 / 32768.0,
                -24576 / 32768.0,
                0.0,
                32767 / 32768.0,
                -1.0,
            ),
        )

    def test_float32_wav_interleaved_channels(self):
        # frame0: (ch0=1.0, ch1=-1.0) frame1: (ch0=0.5, ch1=-0.5)
        path = self._tmp("a32f2.wav")
        _write_riff(
            path,
            fmt_tag=3,
            channels=2,
            rate=44100,
            bits=32,
            data=_float32_bytes(1.0, -1.0, 0.5, -0.5),
        )
        pcm = read_pcm_wav(path)
        self.assertEqual(pcm.channel_count, 2)
        self.assertEqual(pcm.channels[0], (32767 / 32768.0, 16384 / 32768.0))
        self.assertEqual(pcm.channels[1], (-1.0, -16384 / 32768.0))

    def test_zero_frame_wav_is_rejected(self):
        path = self._tmp("empty.wav")
        _write_riff(path, fmt_tag=3, channels=1, rate=44100, bits=32, data=b"")
        with self.assertRaisesRegex(ValueError, "at least one frame"):
            read_pcm_wav(path)


class ReadPcmWavRejectionTests(unittest.TestCase):
    def _tmp(self, name: str) -> Path:
        return Path(tempfile.mkdtemp()) / name

    def test_eight_bit_wav_is_rejected(self):
        path = self._tmp("pcm8.wav")
        with wave.open(str(path), "wb") as target:
            target.setnchannels(1)
            target.setsampwidth(1)
            target.setframerate(8000)
            target.writeframes(b"\x80\x81")
        with self.assertRaisesRegex(ValueError, "16-bit PCM, 24-bit PCM"):
            read_pcm_wav(path)

    def test_32_bit_pcm_tag_is_rejected(self):
        path = self._tmp("pcm32.wav")
        _write_riff(
            path, fmt_tag=1, channels=1, rate=44100, bits=32,
            data=b"\x00" * 16,
        )
        with self.assertRaisesRegex(ValueError, "16-bit PCM, 24-bit PCM"):
            read_pcm_wav(path)

    def test_compressed_format_is_rejected(self):
        path = self._tmp("adpcm.wav")
        # ADPCM (tag 2) with whole-byte samples still fails the format check.
        _write_riff(
            path, fmt_tag=2, channels=1, rate=22050, bits=8,
            data=b"\x00" * 8,
        )
        with self.assertRaisesRegex(ValueError, "16-bit PCM, 24-bit PCM"):
            read_pcm_wav(path)

    def test_non_riff_input_is_rejected(self):
        path = self._tmp("junk.wav")
        path.write_bytes(b"not a riff file at all, really")
        with self.assertRaisesRegex(ValueError, "RIFF/WAVE"):
            read_pcm_wav(path)

    def test_missing_data_chunk_is_rejected(self):
        path = self._tmp("nodata.wav")
        fmt_bytes = struct.pack("<HHIIHH", 1, 1, 44100, 88200, 2, 16)
        with open(path, "wb") as handle:
            handle.write(b"RIFF")
            handle.write(struct.pack("<I", 24))
            handle.write(b"WAVE")
            handle.write(b"fmt ")
            handle.write(struct.pack("<I", len(fmt_bytes)))
            handle.write(fmt_bytes)
        with self.assertRaisesRegex(ValueError, "data chunk"):
            read_pcm_wav(path)

    def test_misaligned_data_is_rejected(self):
        path = self._tmp("misaligned.wav")
        # 1 channel, 3 bytes/sample: 7 bytes is not a whole number of frames.
        _write_riff(
            path, fmt_tag=1, channels=1, rate=44100, bits=24,
            data=b"\x00" * 7,
        )
        with self.assertRaisesRegex(ValueError, "align to whole frames"):
            read_pcm_wav(path)

    def test_truncated_fmt_chunk_is_rejected(self):
        path = self._tmp("shortfmt.wav")
        with open(path, "wb") as handle:
            handle.write(b"RIFF")
            handle.write(struct.pack("<I", 20))
            handle.write(b"WAVE")
            handle.write(b"fmt ")
            handle.write(struct.pack("<I", 16))
            handle.write(bytes([1, 0, 0, 0, 1, 0]))
        with self.assertRaisesRegex(ValueError, "fmt chunk"):
            read_pcm_wav(path)

    def test_float32_nan_sample_is_rejected(self):
        path = self._tmp("nanf32.wav")
        _write_riff(
            path, fmt_tag=3, channels=1, rate=44100, bits=32,
            data=_float32_bytes(0.5, float("nan")),
        )
        with self.assertRaisesRegex(ValueError, "finite"):
            read_pcm_wav(path)


if __name__ == "__main__":
    unittest.main()
