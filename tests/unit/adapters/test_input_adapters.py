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

def _write_riff_fmt(path: Path, fmt_bytes: bytes, data: bytes) -> None:
    # One fixed layout: the caller's fmt chunk payload, then the data chunk.
    # Cases that need a different shape (a truncated fmt, no data chunk) write
    # their bytes inline instead, so the RIFF size below is exactly this file's
    # length by construction.  ``_write_riff`` is this with the canonical
    # 16-byte PCM payload; the input contract's cases need the 40-byte
    # extensible payload too.
    with open(path, "wb") as handle:
        handle.write(b"RIFF")
        handle.write(struct.pack("<I", 20 + len(fmt_bytes) + len(data)))
        handle.write(b"WAVE")
        handle.write(b"fmt ")
        handle.write(struct.pack("<I", len(fmt_bytes)))
        handle.write(fmt_bytes)
        handle.write(b"data")
        handle.write(struct.pack("<I", len(data)))
        handle.write(data)


def _write_riff(path: Path, *, fmt_tag: int, channels: int, rate: int,
                bits: int, data: bytes) -> None:
    # The canonical 16-byte PCM fmt chunk: the shape every 16/24-bit and
    # float32 file written by a tool with no extension to declare has.
    _write_riff_fmt(
        path,
        struct.pack(
            "<HHIIHH", fmt_tag, channels, rate, rate * channels * bits // 8,
            channels * bits // 8, bits,
        ),
        data,
    )


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


# --------------------------------------------------------------------------
# the WAV input contract: the accepted set, and every refusal
# --------------------------------------------------------------------------

# What a file may be is a table, and this is it.  The reader's gate is the
# pair (format tag, sample width): tag 1 (PCM) at 16 or 24 bits, and tag 3
# (IEEE float) at 32 bits.  The width must be a whole byte count, so the
# accepted pairs are exactly (1, 2), (1, 3) and (3, 4) -- the three
# representations of ``_ACCEPTED_WAV_FORMATS`` -- and ``_REFUSED_WAV_FORMATS``
# holds the neighbouring widths of each accepted tag plus one row per other tag
# family that WAV defines.  The same three forms are what ``docs/guides/usage.md``
# ("Preparing input") tells a caller to produce, so that list and this gate
# cannot drift apart unnoticed: a row added, removed or reworded on either
# side fails here.
#
# A header may spell those representations two ways, and both are accepted
# rows: the canonical 16-byte ``fmt `` chunk, which carries the tag directly,
# and the 40-byte ``WAVE_FORMAT_EXTENSIBLE`` chunk, which carries it in the
# sub-format GUID.  The extensible header is a wider *header*, not a wider set
# of audio formats -- it is what a writer emits when the original header could
# not name the representation, and ``ffmpeg`` emits it for more than two
# channels or for integer samples wider than 16 bits.  Both spellings of a
# representation must convert to the same signed-16 words, and a sub-format
# this reader does not resolve is refused exactly as its tag would be, which
# is what the extensible rows of ``_REFUSED_WAV_FORMATS`` pin.
#
# ``wValidBitsPerSample`` is refused unless it equals the container's width:
# the samples would be left-aligned with the unused low bits zero-padded, and
# the reader takes only full-width containers, so a padded one is reported with
# its count instead of being read as if it were full-width.  ``dwChannelMask``
# is read and ignored -- the reader's contract is interleaved samples in the
# file's own order -- and the channel-mask case below pins that a mask changes
# nothing about the words.
#
# Each accepted row also names the rule its samples convert by, and those
# rules are the repository's own -- no specification states a conversion
# between PCM representations (see ``docs/findings/input-format-practice.md``,
# N8).  All three live in ``adapters/sample_conversion.py``:
#
#   * 16-bit: the word is already the signed-16 value; the domain float is
#     ``int16_to_domain_value`` (``value / 32768.0``), the expression the
#     signed-16 path has always used;
#   * 24-bit: ``sample24_to_int16`` -- round to nearest, ties away from zero
#     (``(s + 128) >> 8`` for ``s >= 0``, ``-((-s + 128) >> 8)`` below), then
#     saturate to ``[-32768, 32767]``;
#   * float32: ``float_to_int16`` -- reject non-finite values, scale by
#     ``32768.0``, round to nearest with ties away from zero, then saturate.
#
# A refused format leaves the reader as a bare ``ValueError``: it is raised at
# the adapter boundary, before the kernel is reached, so it carries none of the
# ``WwiseWemError`` codes (``FORMAT_UNSUPPORTED`` among them).  Its message
# names the three forms the reader accepts and then what it was handed -- the
# tag and width it read, or the extensible sub-format, or the valid-bits count
# -- so a caller can see which of the two the file actually is.  Every refused
# row therefore spells its own ``got`` clause out and compares the whole
# message, rather than matching a substring read back out of the error.

_KSDATAFORMAT_SUFFIX = bytes.fromhex("00001000800000aa00389b71")


def _ksdataformat(format_tag: int) -> bytes:
    """The KSDATAFORMAT sub-format GUID whose format tag is ``format_tag``."""
    return struct.pack("<I", format_tag) + _KSDATAFORMAT_SUFFIX


_VENDOR_SUB_FORMAT = bytes.fromhex("deadbeefcafe1000800000aa00389b71")

_WAV_FORMAT_REFUSAL = (
    "encoder input WAV must be 16-bit PCM, 24-bit PCM, or 32-bit IEEE-float PCM"
)
_WAV_BITS_REFUSAL = "WAV bits per sample is not a whole byte count"
_WAV_EXTENSIBLE_FMT_REFUSAL = "WAV extensible fmt chunk is malformed or truncated"


def _refused_at(seen: str) -> str:
    """The whole refusal message for a header the reader saw as ``seen``."""
    return f"{_WAV_FORMAT_REFUSAL}; got {seen}"


def _wav_fmt_bytes(
    format_tag: int,
    bits: int,
    *,
    channels: int = 1,
    rate: int = 44100,
    extensible: bool = False,
    sub_format: bytes | None = None,
    valid_bits: int | None = None,
    channel_mask: int = 0,
) -> bytes:
    """One ``fmt `` chunk payload: canonical, or the extensible 40-byte form.

    The extensible form is the same six fields under tag 0xFFFE plus a 22-byte
    extension (valid bits, channel mask) and the 16-byte sub-format GUID the
    tag's own meaning moved into.  ``format_tag`` is the tag the canonical
    header would carry and selects the matching KSDATAFORMAT sub-format unless
    ``sub_format`` names another one; ``valid_bits`` defaults to the container
    width, the full-width case every accepted row is.
    """
    block = channels * bits // 8
    if not extensible:
        return struct.pack(
            "<HHIIHH", format_tag, channels, rate, rate * block, block, bits
        )
    if sub_format is None:
        sub_format = _ksdataformat(format_tag)
    return (
        struct.pack("<HHIIHH", 0xFFFE, channels, rate, rate * block, block, bits)
        + struct.pack(
            "<HHI", 22, bits if valid_bits is None else valid_bits, channel_mask
        )
        + sub_format
    )


def _signed16_word(word: int) -> int:
    """The 16-bit path's rule: the stored word is the signed-16 value."""
    return word


_SAMPLE_BYTES_BY_WIDTH = {
    16: int16_le_bytes,
    24: int24_le_bytes,
    32: float32_le_bytes,
}

# (label, fmt chunk, sample width, the file's words, their signed-16 words,
#  the conversion rule the two word lists are a case of)
_ACCEPTED_WAV_FORMATS = (
    (
        "tag 1, 16-bit signed PCM",
        _wav_fmt_bytes(1, 16),
        16,
        (0, 1, -1, 32767, -32768),
        (0, 1, -1, 32767, -32768),
        _signed16_word,
    ),
    (
        "tag 1, 24-bit signed PCM",
        _wav_fmt_bytes(1, 24),
        24,
        (0, 128, -128, 8388607, -8388608),
        (0, 1, -1, 32767, -32768),
        sample24_to_int16,
    ),
    (
        "tag 3, 32-bit IEEE-float PCM",
        _wav_fmt_bytes(3, 32),
        32,
        (0.0, 0.5, -0.5, 1.0, -1.0),
        (0, 16384, -16384, 32767, -32768),
        float_to_int16,
    ),
    (
        "tag 0xFFFE, extensible, 16-bit signed PCM sub-format",
        _wav_fmt_bytes(1, 16, extensible=True),
        16,
        (0, 1, -1, 32767, -32768),
        (0, 1, -1, 32767, -32768),
        _signed16_word,
    ),
    (
        "tag 0xFFFE, extensible, 24-bit signed PCM sub-format",
        _wav_fmt_bytes(1, 24, extensible=True),
        24,
        (0, 128, -128, 8388607, -8388608),
        (0, 1, -1, 32767, -32768),
        sample24_to_int16,
    ),
    (
        "tag 0xFFFE, extensible, 32-bit IEEE-float sub-format",
        _wav_fmt_bytes(3, 32, extensible=True),
        32,
        (0.0, 0.5, -0.5, 1.0, -1.0),
        (0, 16384, -16384, 32767, -32768),
        float_to_int16,
    ),
)

# (label, fmt chunk, the message the reader refuses it with)
_REFUSED_WAV_FORMATS = (
    ("tag 1, 8-bit unsigned PCM", _wav_fmt_bytes(1, 8),
     _refused_at("format tag 1 at 8 bits per sample")),
    ("tag 1, 32-bit signed PCM", _wav_fmt_bytes(1, 32),
     _refused_at("format tag 1 at 32 bits per sample")),
    ("tag 3, 16-bit (a width IEEE float does not use)",
     _wav_fmt_bytes(3, 16),
     _refused_at("format tag 3 at 16 bits per sample")),
    ("tag 3, 64-bit float", _wav_fmt_bytes(3, 64),
     _refused_at("format tag 3 at 64 bits per sample")),
    ("tag 2, ADPCM (4-bit samples)", _wav_fmt_bytes(2, 4), _WAV_BITS_REFUSAL),
    ("tag 6, A-law", _wav_fmt_bytes(6, 8),
     _refused_at("format tag 6 at 8 bits per sample")),
    ("tag 7, mu-law", _wav_fmt_bytes(7, 8),
     _refused_at("format tag 7 at 8 bits per sample")),
    ("tag 0x0055, MPEG Layer-3", _wav_fmt_bytes(0x0055, 0),
     _refused_at("format tag 85 at 0 bits per sample")),
    ("tag 0xF1AC, FLAC", _wav_fmt_bytes(0xF1AC, 0),
     _refused_at("format tag 61868 at 0 bits per sample")),
    ("tag 1, 12-bit (not a whole byte count)", _wav_fmt_bytes(1, 12),
     _WAV_BITS_REFUSAL),
    # The same refusals under the extensible header: a sub-format this reader
    # does not resolve is not a representation it takes, and which of the two
    # header spellings named it changes only the message's ``got`` clause.
    (
        "tag 0xFFFE, extensible, 8-bit A-law sub-format",
        _wav_fmt_bytes(6, 8, extensible=True),
        _refused_at("WAVE_FORMAT_EXTENSIBLE sub-format tag 6 at 8 bits per sample"),
    ),
    (
        "tag 0xFFFE, extensible, 8-bit mu-law sub-format",
        _wav_fmt_bytes(7, 8, extensible=True),
        _refused_at("WAVE_FORMAT_EXTENSIBLE sub-format tag 7 at 8 bits per sample"),
    ),
    (
        "tag 0xFFFE, extensible, 4-bit ADPCM sub-format",
        _wav_fmt_bytes(2, 4, extensible=True),
        _WAV_BITS_REFUSAL,
    ),
    (
        "tag 0xFFFE, extensible, 8-bit signed PCM sub-format",
        _wav_fmt_bytes(1, 8, extensible=True),
        _refused_at(
            "WAVE_FORMAT_EXTENSIBLE sub-format tag 1 at 8 bits per sample"
        ),
    ),
    (
        "tag 0xFFFE, extensible, 32-bit signed PCM sub-format",
        _wav_fmt_bytes(1, 32, extensible=True),
        _refused_at(
            "WAVE_FORMAT_EXTENSIBLE sub-format tag 1 at 32 bits per sample"
        ),
    ),
    (
        "tag 0xFFFE, extensible, 64-bit float sub-format",
        _wav_fmt_bytes(3, 64, extensible=True),
        _refused_at(
            "WAVE_FORMAT_EXTENSIBLE sub-format tag 3 at 64 bits per sample"
        ),
    ),
    (
        "tag 0xFFFE, extensible, a vendor sub-format GUID",
        _wav_fmt_bytes(1, 16, extensible=True, sub_format=_VENDOR_SUB_FORMAT),
        _refused_at(
            "WAVE_FORMAT_EXTENSIBLE sub-format "
            "efbeadde-feca-0010-8000-00aa00389b71 at 16 bits per sample"
        ),
    ),
    # The padded container: 20 valid bits left-aligned in 24, with the low four
    # bits zero. Reading it as a full-width 24-bit container would be a fourth
    # conversion rule, so it is refused with the count it saw.
    (
        "tag 0xFFFE, extensible, 20 valid bits in a 24-bit container",
        _wav_fmt_bytes(1, 24, extensible=True, valid_bits=20),
        _refused_at(
            "WAVE_FORMAT_EXTENSIBLE wValidBitsPerSample=20 in a 24-bit container"
        ),
    ),
    (
        "tag 0xFFFE, extensible, fmt chunk shorter than the 40-byte layout",
        _wav_fmt_bytes(1, 16, extensible=True)[:30],
        _WAV_EXTENSIBLE_FMT_REFUSAL,
    ),
)


class WavInputContractTests(unittest.TestCase):
    """Which sample formats a WAV may hold, and what a refusal says."""

    def _tmp(self, name: str) -> Path:
        return Path(tempfile.mkdtemp()) / name

    def test_accepted_formats_convert_by_their_named_rule(self):
        for label, fmt_bytes, width, words, expected, rule in _ACCEPTED_WAV_FORMATS:
            with self.subTest(format=label):
                # The row's two word lists are a case of the rule it names: if
                # the rule's arithmetic moves, the row moves with it and says
                # so, rather than agreeing with whatever the reader does.
                self.assertEqual(
                    tuple(rule(word) for word in words),
                    expected,
                    f"{label}: the rule this row names must still produce these "
                    "signed-16 words",
                )
                path = self._tmp("accepted.wav")
                _write_riff_fmt(
                    path, fmt_bytes, _SAMPLE_BYTES_BY_WIDTH[width](*words)
                )
                pcm = read_pcm_wav(path)
                self.assertEqual(pcm.sample_rate, 44100, label)
                self.assertEqual(pcm.channel_count, 1, label)
                self.assertEqual(pcm.frame_count, len(words), label)
                self.assertEqual(
                    pcm.channels[0],
                    tuple(int16_to_domain_value(word) for word in expected),
                    f"{label}: the reader must convert by that rule and no other",
                )

    def test_extensible_channel_mask_changes_nothing_about_the_words(self):
        # Six channels is the geometry this header exists for, and the mask is
        # the one place it states a speaker order. The reader's contract is
        # interleaved samples in the file's own order, so the words come back
        # as written for any mask -- including one that names a different
        # order, and including none.
        channels = 6
        words = tuple(
            channel - 3 for _ in range(2) for channel in range(channels)
        )
        read = []
        for channel_mask in (0x0000, 0x003F):
            with self.subTest(channel_mask=channel_mask):
                path = self._tmp("extensible6.wav")
                _write_riff_fmt(
                    path,
                    _wav_fmt_bytes(
                        1, 16, channels=channels, extensible=True,
                        channel_mask=channel_mask,
                    ),
                    int16_le_bytes(*words),
                )
                pcm = read_pcm_wav(path)
                self.assertEqual(pcm.channel_count, channels)
                self.assertEqual(pcm.frame_count, 2)
                self.assertEqual(
                    pcm.channels,
                    tuple(
                        (int16_to_domain_value(channel - 3),) * 2
                        for channel in range(channels)
                    ),
                    "the words keep their file order and are not regrouped by "
                    "the mask",
                )
                read.append(pcm)
        self.assertEqual(read[0], read[1], "a mask is read and not acted on")

    def test_refused_formats_are_refused_with_the_documented_message(self):
        for label, fmt_bytes, message in _REFUSED_WAV_FORMATS:
            with self.subTest(format=label):
                path = self._tmp("refused.wav")
                # The header alone decides, so the payload only has to exist.
                _write_riff_fmt(path, fmt_bytes, b"\x00" * 16)
                with self.assertRaises(ValueError) as raised:
                    read_pcm_wav(path)
                error = raised.exception
                self.assertIs(
                    type(error),
                    ValueError,
                    f"{label}: the refusal is the reader's own ValueError, "
                    "raised before the kernel and carrying no error code",
                )
                self.assertEqual(str(error), message, label)

    def test_wrappers_that_are_not_riff_wave_are_refused(self):
        # The magic decides before anything inside it is read, so each case is
        # only a four-byte wrapper name carrying a payload in its own byte
        # order; the chunk layout behind an RF64 or RIFX file is never reached.
        payload = _wav_fmt_bytes(1, 16)
        for label, magic, order in (
            ("RF64, the 64-bit broadcast wrapper", b"RF64", "<"),
            ("RIFX, big-endian RIFF", b"RIFX", ">"),
        ):
            with self.subTest(wrapper=label):
                path = self._tmp("wrapper.wav")
                path.write_bytes(
                    magic
                    + struct.pack(f"{order}I", 36)
                    + b"WAVEfmt "
                    + struct.pack(f"{order}I", 16)
                    + payload
                    + b"data"
                    + struct.pack(f"{order}I", 4)
                    + b"\x00" * 4
                )
                with self.assertRaises(ValueError) as raised:
                    read_pcm_wav(path)
                self.assertIs(type(raised.exception), ValueError, label)
                self.assertEqual(
                    str(raised.exception),
                    "encoder input must be a RIFF/WAVE file",
                    label,
                )


if __name__ == "__main__":
    unittest.main()
