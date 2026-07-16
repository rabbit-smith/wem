import hashlib
import math
import struct
import unittest

import wwise2013_wem.analysis.dsp.transform as transform
import wwise2013_wem.analysis.dsp.spectrum as log_spectrum


def f32(value):
    return struct.unpack("<f", struct.pack("<f", float(value)))[0]


class TransformRuntimeTests(unittest.TestCase):
    def test_packed_fft_anchor_vectors(self):
        self.assertEqual(
            log_spectrum.wwise_fft_packed([1.0] + [0.0] * 7),
            [1.0, 1.0, 0.0, 1.0, 0.0, 1.0, 0.0, 1.0],
        )
        constant = log_spectrum.wwise_fft_packed([1.0] * 8)
        self.assertEqual(constant[0], 8.0)
        self.assertEqual(max(abs(value) for value in constant[1:]), 0.0)

    def test_mdct_float32_spill_vector_and_direct_reference(self):
        samples = [
            f32(math.sin(0.13 * index) * (0.2 + index / 80.0))
            for index in range(64)
        ]
        actual = transform.mdct_forward(transform.make_mdct_look(64), samples)
        packed = b"".join(struct.pack("<f", value) for value in actual)
        self.assertEqual(
            hashlib.sha256(packed).hexdigest(),
            "400d55c7245bbe96714f629438d7b66f938dfde6cd59f3dac3cb81418aab5115",
        )
        direct = [
            (4.0 / 64)
            * sum(
                samples[index]
                * math.cos(
                    math.pi
                    / 32.0
                    * (index + 0.5 + 16.0)
                    * (coefficient + 0.5)
                )
                for index in range(64)
            )
            for coefficient in range(32)
        ]
        self.assertLess(
            max(abs(left - right) for left, right in zip(actual, direct)), 2e-6
        )

    def test_window_float32_endpoint_anchor(self):
        self.assertEqual(struct.pack("<f", transform.vorbis_window(256)[0]).hex(), "050c7838")
        window = transform.vorbis_window(64)
        self.assertEqual(window, window[::-1])

if __name__ == "__main__":
    unittest.main()
