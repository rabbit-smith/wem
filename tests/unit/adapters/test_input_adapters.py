"""The PCM intake adapters: sample conversion, raw PCM, and RIFF/WAVE.

One module per object under test became three files that all answer one
question — what does the encoder domain receive when a caller hands it
samples? The conversion arithmetic is the object; ``read_raw_pcm`` and
``read_pcm_wav`` are the two carriers that feed it. Each class below is one
of those angles and keeps its own test names and failure messages.
"""

from __future__ import annotations

import struct
import tempfile
import unittest
import wave

from pathlib import Path
from wwise_wem.adapters.raw import read_raw_pcm
from wwise_wem.adapters.sample_conversion import (
    float_to_int16,
    int16_to_domain_value,
    sample24_to_int16,
    unpack_sample24,
)
from wwise_wem.adapters.wav import read_pcm_wav
from wwise_wem.model import PcmBuffer

from tests.pcm_sample_support import (
    float32_le_bytes,
    int16_le_bytes,
    int24_le_bytes,
)

# --------------------------------------------------------------------------
# merged from tests/unit/adapters/test_sample_conversion.py
# --------------------------------------------------------------------------

# Boundary tests for the deterministic signed-16 sample conversion rules.
#
# These tests lock the exact arithmetic of the conversion helpers: the
# 24-bit rule (integer add + shift + explicit saturation), the float rule
# (finiteness check, scale, round ties away from zero, saturation), and the
# encoder float-domain mapping. Every case is a named boundary value from
# the documented conversion rules.

class Sample24ToInt16Tests(unittest.TestCase):
    def test_min_sample_maps_to_min_int16(self):
        # 0x800000 as a signed 24-bit sample is -8388608.
        self.assertEqual(sample24_to_int16(-8388608), -32768)

    def test_max_sample_saturates_at_max_int16(self):
        # 0x7FFFFF is 8388607; it rounds to 32768 and saturates.
        self.assertEqual(sample24_to_int16(8388607), 32767)

    def test_upper_band_saturates(self):
        # s >= 8388480 (0x7FF800) rounds to 32768 -> saturates to 32767.
        self.assertEqual(sample24_to_int16(8388480), 32767)
        self.assertEqual(sample24_to_int16(8388481), 32767)
        self.assertEqual(sample24_to_int16(8388479), 32767)  # 8388479/256 ~ 32767.49

    def test_zero_maps_to_zero(self):
        self.assertEqual(sample24_to_int16(0), 0)

    def test_ties_round_away_from_zero(self):
        self.assertEqual(sample24_to_int16(128), 1)
        self.assertEqual(sample24_to_int16(127), 0)
        self.assertEqual(sample24_to_int16(-128), -1)
        self.assertEqual(sample24_to_int16(-129), -1)
        self.assertEqual(sample24_to_int16(-255), -1)
        self.assertEqual(sample24_to_int16(-256), -1)
        self.assertEqual(sample24_to_int16(-257), -1)
        self.assertEqual(sample24_to_int16(-383), -1)  # just above the -1.5 tie
        self.assertEqual(sample24_to_int16(-384), -2)  # the -1.5 tie, away from zero

    def test_mid_scale_values(self):
        self.assertEqual(sample24_to_int16(256), 1)
        self.assertEqual(sample24_to_int16(1 << 16), 256)
        self.assertEqual(sample24_to_int16(-(1 << 16)), -256)

    def test_out_of_range_samples_are_rejected(self):
        for value in (8388608, -8388609, 0x1000000):
            with self.assertRaisesRegex(ValueError, "outside the signed 24-bit range"):
                sample24_to_int16(value)

    def test_non_integer_samples_are_rejected(self):
        for value in (True, False, 1.5, "128", None):
            with self.assertRaises(TypeError):
                sample24_to_int16(value)

    def test_conversion_is_pure_and_repeatable(self):
        for value in (-8388608, -1, 0, 1, 8388607):
            first = sample24_to_int16(value)
            for _ in range(3):
                self.assertEqual(sample24_to_int16(value), first)


class FloatToInt16Tests(unittest.TestCase):
    def test_full_scale_floats(self):
        # Asymmetric signed-16 domain: +1.0 saturates, -1.0 is exact.
        self.assertEqual(float_to_int16(1.0), 32767)
        self.assertEqual(float_to_int16(-1.0), -32768)

    def test_float_just_below_one_saturates(self):
        # 1 - 2**-24 is the largest float32 below 1.0; it rounds to 32768.
        self.assertEqual(float_to_int16(1 - 2**-24), 32767)

    def test_out_of_range_finite_values_saturate(self):
        self.assertEqual(float_to_int16(1.5), 32767)
        self.assertEqual(float_to_int16(2.0), 32767)
        self.assertEqual(float_to_int16(-1.5), -32768)
        self.assertEqual(float_to_int16(-2.0), -32768)

    def test_zero_and_tiny_values(self):
        self.assertEqual(float_to_int16(0.0), 0)
        self.assertEqual(float_to_int16(-0.0), 0)
        self.assertEqual(float_to_int16(1 / 32768.0), 1)
        self.assertEqual(float_to_int16(-1 / 32768.0), -1)
        self.assertEqual(float_to_int16(0.5 / 32768.0), 1)
        self.assertEqual(float_to_int16(-0.5 / 32768.0), -1)

    def test_in_domain_values_round_trip(self):
        for value in range(-32768, 32768, 4096):
            self.assertEqual(float_to_int16(value / 32768.0), value)

    def test_nan_and_inf_are_rejected(self):
        for value in (float("nan"), float("inf"), float("-inf")):
            with self.assertRaisesRegex(ValueError, "finite"):
                float_to_int16(value)

    def test_non_numeric_samples_are_rejected(self):
        for value in (True, False, "1.0", None, [1.0]):
            with self.assertRaises(TypeError):
                float_to_int16(value)

    def test_conversion_is_pure_and_repeatable(self):
        for value in (1.0, -1.0, 0.0, 0.25, -0.75):
            first = float_to_int16(value)
            for _ in range(3):
                self.assertEqual(float_to_int16(value), first)


class Int16ToDomainValueTests(unittest.TestCase):
    def test_extreme_values(self):
        self.assertEqual(int16_to_domain_value(-32768), -1.0)
        self.assertEqual(int16_to_domain_value(32767), 32767 / 32768.0)
        self.assertEqual(int16_to_domain_value(0), 0.0)

    def test_matches_encoder_normalization(self):
        # The signed-16 WAV normalization expression, verbatim.
        for value in (-32768, -1, 0, 1, 16384, 32767):
            self.assertEqual(int16_to_domain_value(value), value / 32768.0)

    def test_out_of_range_is_rejected(self):
        for value in (32768, -32769):
            with self.assertRaisesRegex(ValueError, "outside the signed-16 range"):
                int16_to_domain_value(value)

    def test_non_integers_are_rejected(self):
        for value in (True, 1.0, "32767"):
            with self.assertRaises(TypeError):
                int16_to_domain_value(value)


class UnpackSample24Tests(unittest.TestCase):
    def test_two_complement_little_endian_bytes(self):
        # 0x800000 in little-endian 24-bit is bytes 00 00 80.
        self.assertEqual(unpack_sample24(bytes([0x00, 0x00, 0x80]), 0), -8388608)
        # 0x7FFFFF: bytes FF FF 7F.
        self.assertEqual(unpack_sample24(bytes([0xFF, 0xFF, 0x7F]), 0), 8388607)
        self.assertEqual(unpack_sample24(bytes([0x80, 0x00, 0x00]), 0), 128)
        self.assertEqual(unpack_sample24(bytes([0x00, 0x80, 0x00, 0x00]), 1), 128)
        self.assertEqual(unpack_sample24(bytes([0x01, 0x00, 0x00]), 0), 1)


# --------------------------------------------------------------------------
# merged from tests/unit/adapters/test_raw_pcm_adapter.py
# --------------------------------------------------------------------------

# Boundary tests for the raw PCM adapter (explicit geometry, no framing).

class RawPcmGeometryValidationTests(unittest.TestCase):
    """Rejection semantics mirror PcmBuffer: explicit, at the boundary."""

    def test_sample_rate_requires_positive_integer(self):
        data = int16_le_bytes(0)
        with self.assertRaises(TypeError):
            read_raw_pcm(data, sample_rate=True, channels=1, bits_per_sample=16)
        with self.assertRaises(TypeError):
            read_raw_pcm(data, sample_rate=44100.0, channels=1, bits_per_sample=16)
        for rate in (0, -44100):
            with self.assertRaisesRegex(ValueError, "sample_rate must be positive"):
                read_raw_pcm(data, sample_rate=rate, channels=1, bits_per_sample=16)

    def test_channels_require_positive_integer(self):
        data = int16_le_bytes(0)
        with self.assertRaises(TypeError):
            read_raw_pcm(data, sample_rate=44100, channels=True, bits_per_sample=16)
        for count in (0, -1):
            with self.assertRaisesRegex(ValueError, "channels must be positive"):
                read_raw_pcm(
                    data, sample_rate=44100, channels=count, bits_per_sample=16
                )

    def test_bits_per_sample_accepts_only_16_24_32(self):
        data = int16_le_bytes(0)
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
        data = int16_le_bytes(1, 2, 3)  # 3 samples, 1 channel: 1 leftover byte
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
        data = bytearray(int16_le_bytes(32767))
        pcm = read_raw_pcm(data, sample_rate=44100, channels=1, bits_per_sample=16)
        self.assertEqual(pcm.channels[0], (32767 / 32768.0,))
        pcm2 = read_raw_pcm(
            memoryview(data), sample_rate=44100, channels=1, bits_per_sample=16
        )
        self.assertEqual(pcm, pcm2)


class RawPcmFormatTests(unittest.TestCase):
    def test_int16_preserves_the_encoder_domain(self):
        pcm = read_raw_pcm(
            int16_le_bytes(-32768, 32767, 0, 1, -1),
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
        data = int24_le_bytes(-8388608, 8388607, 0, -128)
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
            float32_le_bytes(1.0, -1.0, 2.0, -2.0, 0.5),
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
            float32_le_bytes(float("nan")),
            float32_le_bytes(float("inf")),
            float32_le_bytes(float("-inf")),
            float32_le_bytes(0.5, float("nan")),
        ):
            with self.assertRaisesRegex(ValueError, "finite"):
                read_raw_pcm(
                    payload, sample_rate=44100, channels=1, bits_per_sample=32
                )

    def test_formats_agree_on_shared_sample_values(self):
        values = (-32768, 32767, 0, 100, -100)
        pcm16 = read_raw_pcm(
            int16_le_bytes(*values), sample_rate=48000, channels=1, bits_per_sample=16
        )
        pcm24 = read_raw_pcm(
            int24_le_bytes(*[v << 8 for v in values]),
            sample_rate=48000,
            channels=1,
            bits_per_sample=24,
        )
        pcm32 = read_raw_pcm(
            float32_le_bytes(*[v / 32768.0 for v in values]),
            sample_rate=48000,
            channels=1,
            bits_per_sample=32,
        )
        self.assertEqual(pcm16, pcm24)
        self.assertEqual(pcm24, pcm32)


# --------------------------------------------------------------------------
# merged from tests/unit/adapters/test_wav_reader.py
# --------------------------------------------------------------------------

# Boundary tests for the extended WAV reader (16/24-bit, float32).

def _write_riff(path: Path, *, fmt_tag: int, channels: int, rate: int,
                bits: int, data: bytes) -> None:
    # One fixed layout: a 16-byte canonical PCM fmt chunk, then the data
    # chunk.  Cases that need a different shape (a truncated fmt, no data
    # chunk) write their bytes inline instead, so the RIFF size below is
    # exactly this file's length by construction.
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

    def test_int16_preserves_channel_order_and_normalization(self):
        values = (-32768, 32767, -1, 1, 0, -16384)
        path = self._tmp("a16.wav")
        with wave.open(str(path), "wb") as target:
            target.setnchannels(2)
            target.setsampwidth(2)
            target.setframerate(22050)
            target.writeframes(int16_le_bytes(*values))
        pcm = read_pcm_wav(path)
        self.assertIsInstance(pcm, PcmBuffer)
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
            target.writeframes(int24_le_bytes(*samples))
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
            data=float32_le_bytes(*values),
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
            data=float32_le_bytes(1.0, -1.0, 0.5, -0.5),
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
            handle.write(struct.pack("<I", 28))
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
            handle.write(struct.pack("<I", 18))
            handle.write(b"WAVE")
            handle.write(b"fmt ")
            handle.write(struct.pack("<I", 16))
            handle.write(bytes([1, 0, 0, 0, 1, 0]))
        with self.assertRaisesRegex(ValueError, "fmt chunk"):
            read_pcm_wav(path)

    def test_huge_declared_chunk_is_rejected_before_reading_payload(self):
        path = self._tmp("huge-data.wav")
        path.write_bytes(
            b"RIFF"
            + struct.pack("<I", 12)
            + b"WAVEdata"
            + struct.pack("<I", 0xFFFF_FFFF)
        )
        with self.assertRaisesRegex(ValueError, "data chunk is truncated"):
            read_pcm_wav(path)

    def test_float32_nan_sample_is_rejected(self):
        path = self._tmp("nanf32.wav")
        _write_riff(
            path, fmt_tag=3, channels=1, rate=44100, bits=32,
            data=float32_le_bytes(0.5, float("nan")),
        )
        with self.assertRaisesRegex(ValueError, "finite"):
            read_pcm_wav(path)


if __name__ == "__main__":
    unittest.main()
