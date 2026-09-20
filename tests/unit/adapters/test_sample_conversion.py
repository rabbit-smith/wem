"""Boundary tests for the deterministic signed-16 sample conversion rules.

These tests lock the exact arithmetic of the conversion helpers: the
24-bit rule (integer add + shift + explicit saturation), the float rule
(finiteness check, scale, round ties away from zero, saturation), and the
encoder float-domain mapping. Every case is a named boundary value from
the documented conversion rules.
"""

from __future__ import annotations

import unittest

from wwise_wem.adapters.sample_conversion import (
    float_to_int16,
    int16_to_domain_value,
    sample24_to_int16,
    unpack_sample24,
)


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


if __name__ == "__main__":
    unittest.main()
