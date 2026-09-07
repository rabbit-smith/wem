from __future__ import annotations

import hashlib
import struct
import unittest

from wwise_wem_reference.analysis.preprocessing.detector_input import (
    detector_pcm_streams,
    iter_detector_quanta,
)


def _float_sha256(values) -> str:
    digest = hashlib.sha256()
    for value in values:
        digest.update(struct.pack("<f", float(value)))
    return digest.hexdigest()


class DetectorFeederTests(unittest.TestCase):
    def test_lpc_padded_timeline_and_quantum_boundaries_are_word_exact(self):
        signal = tuple(
            (((index * 37) & 2047) - 1024) / 32768
            for index in range(4096)
        )
        stream = detector_pcm_streams((signal,))[0]
        quanta = tuple(iter_detector_quanta((signal,)))

        self.assertEqual(len(stream), 13312)
        self.assertEqual(
            _float_sha256(stream),
            "cfea0c73a1d69192da99fb8df11d2ca06f4878ca5d7f4c0fe10a0e29ef6b2f6e",
        )
        self.assertEqual(len(quanta), 207)
        self.assertEqual(
            {index: _float_sha256(quanta[index][0]) for index in (0, 15, 16, 79, 80, 206)},
            {
                0: "e8bd35d84e86e66371bf349803bbde066dc363e9e5e88f7b4a48d831ec738fa1",
                15: "b67bf73f5f9370e16ecdb40b4ae31113e03b260cf5405d49d84642f21e8709c5",
                16: "1517965797e845fb468925ffb976041ab8bdf4b8f647f62b675a8400fa25d544",
                79: "789bd8e4b113422a81cc951d916e0eb95b454650ef9f231c078a4e4aff1595b6",
                80: "d65235724dd5a4bbe053e84f6ecda7a9355dd803340f200a197c57a4f1ed8d2c",
                206: "ac97b7ed699bcb7638cf35124276dd8e31ad1fe6738c38194616cc4065600a6d",
            },
        )

    def test_geometry_and_requested_count_validation(self):
        signal = (0.0,) * 4096
        with self.assertRaisesRegex(ValueError, "at least one PCM channel"):
            detector_pcm_streams(())
        with self.assertRaisesRegex(ValueError, "equal-length"):
            detector_pcm_streams((signal, signal[:-1]))
        with self.assertRaisesRegex(ValueError, "non-negative"):
            detector_pcm_streams((signal,), terminal_samples=-1)
        with self.assertRaisesRegex(ValueError, "only"):
            tuple(iter_detector_quanta((signal,), count=10_000))


if __name__ == "__main__":
    unittest.main()
