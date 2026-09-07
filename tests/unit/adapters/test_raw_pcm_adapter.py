"""Boundary tests for the raw PCM adapter (explicit geometry, no framing)."""

from __future__ import annotations

import struct
import unittest

from wwise_wem.adapters.raw import read_raw_pcm
from wwise_wem.model import PcmBuffer


def _to_int16_le(*values: int) -> bytes:
    return b"".join(v.to_bytes(2, "little", signed=True) for v in values)


def _to_int24_le(*values: int) -> bytes:
    return b"".join((v & 0xFFFFFF).to_bytes(3, "little") for v in values)


def _to_f32_le(*values: float) -> bytes:
    return struct.pack(f"<{len(values)}f", *values)


class RawPcmGeometryValidationTests(unittest.TestCase):
    """Rejection semantics mirror PcmBuffer: explicit, at the boundary."""

    def test_sample_rate_requires_positive_integer(self):
        data = _to_int16_le(0)
        with self.assertRaises(TypeError):
            read_raw_pcm(data, sample_rate=True, channels=1, bits_per_sample=16)
        with self.assertRaises(TypeError):
            read_raw_pcm(data, sample_rate=44100.0, channels=1, bits_per_sample=16)
        for rate in (0, -44100):
            with self.assertRaisesRegex(ValueError, "sample_rate must be positive"):
                read_raw_pcm(data, sample_rate=rate, channels=1, bits_per_sample=16)

    def test_channels_require_positive_integer(self):
        data = _to_int16_le(0)
        with self.assertRaises(TypeError):
            read_raw_pcm(data, sample_rate=44100, channels=True, bits_per_sample=16)
        for count in (0, -1):
            with self.assertRaisesRegex(ValueError, "channels must be positive"):
                read_raw_pcm(
                    data, sample_rate=44100, channels=count, bits_per_sample=16
                )

    def test_bits_per_sample_accepts_only_16_24_32(self):
        data = _to_int16_le(0)
        for bits in (8, 15, 20, 31, 64, 0, -16):
            with self.assertRaisesRegex(ValueError, "bits_per_sample must be"):
                read_raw_pcm(
                    data, sample_rate=44100, channels=1, bits_per_sample=bits
                )
        for bits in (True, False, "16", 16.0):
            with self.assertRaises(TypeError):
                read_raw_pcm(
                    data, sample_rate=44100, channels=1, bits_per_sample=bits
                )

    def test_size_must_align_to_frames(self):
        data = _to_int16_le(1, 2, 3)  # 3 samples, 1 channel: 1 leftover byte
        with self.assertRaisesRegex(ValueError, "not a multiple of the frame size"):
            read_raw_pcm(data, sample_rate=44100, channels=2, bits_per_sample=16)

    def test_empty_payload_is_rejected(self):
        with self.assertRaisesRegex(ValueError, "at least one frame"):
            read_raw_pcm(b"", sample_rate=44100, channels=1, bits_per_sample=16)

    def test_non_bytes_payload_is_rejected(self):
        for payload in (None, 42, "16", ("16",)):
            with self.assertRaises(TypeError):
                read_raw_pcm(
                    payload, sample_rate=44100, channels=1, bits_per_sample=16
                )

    def test_bytearray_and_memoryview_are_accepted(self):
        data = bytearray(_to_int16_le(32767))
        pcm = read_raw_pcm(data, sample_rate=44100, channels=1, bits_per_sample=16)
        self.assertEqual(pcm.channels[0], (32767 / 32768.0,))
        pcm2 = read_raw_pcm(
            memoryview(data), sample_rate=44100, channels=1, bits_per_sample=16
        )
        self.assertEqual(pcm, pcm2)


class RawPcmFormatTests(unittest.TestCase):
    def test_int16_preserves_the_legacy_domain(self):
        pcm = read_raw_pcm(
            _to_int16_le(-32768, 32767, 0, 1, -1),
            sample_rate=44100,
            channels=1,
            bits_per_sample=16,
        )
        self.assertIsInstance(pcm, PcmBuffer)
        self.assertEqual(pcm.sample_rate, 44100)
        self.assertEqual(pcm.frame_count, 5)
        self.assertEqual(
            pcm.channels[0],
            (-1.0, 32767 / 32768.0, 0.0, 1 / 32768.0, -1 / 32768.0),
        )

    def test_int24_min_and_max_bytes(self):
        # 0x800000 little-endian: 00 00 80 -> -1.0
        pcm = read_raw_pcm(
            bytes([0x00, 0x00, 0x80]),
            sample_rate=44100,
            channels=1,
            bits_per_sample=24,
        )
        self.assertEqual(pcm.channels[0], (-1.0,))
        # 0x7FFFFF little-endian: FF FF 7F -> saturates to 32767
        pcm_max = read_raw_pcm(
            bytes([0xFF, 0xFF, 0x7F]),
            sample_rate=44100,
            channels=1,
            bits_per_sample=24,
        )
        self.assertEqual(pcm_max.channels[0], (32767 / 32768.0,))

    def test_int24_interleaved_channels_are_deinterleaved(self):
        # frame0: (ch0=-32768, ch1=32767) frame1: (ch0=0, ch1=-128)
        data = _to_int24_le(-8388608, 8388607, 0, -128)
        pcm = read_raw_pcm(
            data, sample_rate=22050, channels=2, bits_per_sample=24
        )
        self.assertEqual(pcm.channel_count, 2)
        self.assertEqual(pcm.frame_count, 2)
        self.assertEqual(pcm.channels[0], (-1.0, 0.0))
        # 8388607 saturates; -128 is the tie that rounds away from zero.
        self.assertEqual(pcm.channels[1], (32767 / 32768.0, -1 / 32768.0))

    def test_float32_full_scale_and_saturation(self):
        pcm = read_raw_pcm(
            _to_f32_le(1.0, -1.0, 2.0, -2.0, 0.5),
            sample_rate=44100,
            channels=1,
            bits_per_sample=32,
        )
        self.assertEqual(
            pcm.channels[0],
            (
                32767 / 32768.0,
                -1.0,
                32767 / 32768.0,
                -1.0,
                16384 / 32768.0,
            ),
        )

    def test_float32_nan_and_inf_are_rejected(self):
        for payload in (
            _to_f32_le(float("nan")),
            _to_f32_le(float("inf")),
            _to_f32_le(float("-inf")),
            _to_f32_le(0.5, float("nan")),
        ):
            with self.assertRaisesRegex(ValueError, "finite"):
                read_raw_pcm(
                    payload, sample_rate=44100, channels=1, bits_per_sample=32
                )

    def test_formats_agree_on_shared_sample_values(self):
        values = (-32768, 32767, 0, 100, -100)
        pcm16 = read_raw_pcm(
            _to_int16_le(*values), sample_rate=48000, channels=1, bits_per_sample=16
        )
        pcm24 = read_raw_pcm(
            _to_int24_le(*[v << 8 for v in values]),
            sample_rate=48000,
            channels=1,
            bits_per_sample=24,
        )
        pcm32 = read_raw_pcm(
            _to_f32_le(*[v / 32768.0 for v in values]),
            sample_rate=48000,
            channels=1,
            bits_per_sample=32,
        )
        self.assertEqual(pcm16, pcm24)
        self.assertEqual(pcm24, pcm32)


if __name__ == "__main__":
    unittest.main()
