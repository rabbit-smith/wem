from __future__ import annotations

import unittest

from wwise_wem_reference.analysis.preprocessing.detector_input import (
    detector_pcm_streams,
    iter_detector_quanta,
)


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
        self.assertEqual(len(quanta), 207)
        # The quantum boundaries are the recorded hop stride of the timeline.
        self.assertTrue(
            all(
                row[0] == stream[index * 64 : index * 64 + 128]
                for index, row in enumerate(quanta)
            )
        )
        # The word-level cross-implementation record of both surfaces lives in
        # the kernel parity suite (crates/wem-analysis/tests/
        # detector_input_parity.rs, recorded from this oracle stage by
        # scripts/record_detector_input_parity.py).

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
