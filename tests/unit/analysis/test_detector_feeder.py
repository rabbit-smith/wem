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
        # EOS prediction is trained on the final long block (2048 samples).
        signal = tuple(
            (((index * 37) & 2047) - 1024) / 32768
            for index in range(4096)
        )
        stream = detector_pcm_streams((signal,))[0]
        quanta = tuple(iter_detector_quanta((signal,)))

        self.assertEqual(len(stream), 13312)
        # Kept deliberately: the padded timeline and the quantum boundaries are
        # derived words with no committed artifact, so these digests are the
        # only record of them -- no byte comparison against a fixture exists to
        # take over.
        self.assertEqual(
            _float_sha256(stream),
            "0e445d751f57d9602c4c33277d097608a366b78e5197fd811614f5576b4d7c11",
        )
        self.assertEqual(len(quanta), 207)
        # Same as above: per-quantum words that exist only here.
        self.assertEqual(
            {index: _float_sha256(quanta[index][0]) for index in (0, 15, 16, 79, 80, 206)},
            {
                0: "e8bd35d84e86e66371bf349803bbde066dc363e9e5e88f7b4a48d831ec738fa1",
                15: "b67bf73f5f9370e16ecdb40b4ae31113e03b260cf5405d49d84642f21e8709c5",
                16: "1517965797e845fb468925ffb976041ab8bdf4b8f647f62b675a8400fa25d544",
                79: "28bcefea6cb7ce06fac684a7fb5c3760ff4d28113c168f3b2e02a7a6d90d1f9b",
                80: "0611d5d35a057d11f4248800f61cca4596e5265f237df936d3e4ddb785c8165d",
                206: "bdf6fb2c31886d9fbe2ceffaddc6f2db31ed54249177e5be25eb92a081566bf6",
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
