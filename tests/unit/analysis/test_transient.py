"""The transient subsystem: the detector and the quanta it is fed.

The feeder decides which samples of a PCM stream each detector quantum sees;
the detector decides which bands flag a transient and carries the history
between quanta. Both were separate files about the same object — a transient
is only observable where the two meet — so they share one module and keep
their own test names and failure messages.
"""

from __future__ import annotations

import unittest
import wwise_wem_reference.analysis.dsp.transform as transform
import wwise_wem_reference.analysis.transient.detector as transient

from tests.analysis_resource_support import installed_analysis_resources
from wwise_wem_reference.analysis.preprocessing.detector_input import (
    detector_pcm_streams,
    iter_detector_quanta,
)
from wwise_wem_reference.analysis.transient.detector import (
    TransientDetector,
    WwisePsyHistory,
    wwise_psy_mask,
)

# --------------------------------------------------------------------------
# merged from tests/unit/analysis/test_transient_detector.py
# --------------------------------------------------------------------------

ZERO = (0.0,) * 128
IMPULSE = (0.0,) * 64 + (1.0,) + (0.0,) * 63
RESOURCES = installed_analysis_resources()


def _detector(channels: int, *, bins: int = 128) -> TransientDetector:
    return TransientDetector(
        channels, RESOURCES.transient, RESOURCES.mdct_looks[128], bins
    )


def _run(
    detector: TransientDetector,
    quanta: tuple[tuple[tuple[float, ...], ...], ...],
    history_windows: tuple[int, ...],
) -> tuple[int, ...]:
    if len(quanta) != len(history_windows):
        raise ValueError("test quantum/window rows must align")
    return tuple(
        detector.analyze_quantum(rows, history_window=history_window)
        for rows, history_window in zip(quanta, history_windows)
    )


class TransientDetectorTests(unittest.TestCase):
    def test_transient_numeric_implementation_has_one_module_owner(self):
        for name in (
            "WwisePsyHistory",
            "_wwise_psy_band_update",
            "wwise_psy_mask",
        ):
            self.assertFalse(hasattr(transform, name), name)
            self.assertEqual(getattr(transient, name).__module__, transient.__name__)

    def test_mask_and_history_words_have_stable_state_evolution(self):
        history = WwisePsyHistory()
        flags = []
        for samples, history_window in zip(
            (ZERO, ZERO, ZERO, IMPULSE, ZERO, ZERO, ZERO),
            (1, 2, 3, 4, 0, 0, 1),
        ):
            wwise_psy_mask(
                samples, RESOURCES.transient, RESOURCES.mdct_looks[128], history, history_window=history_window
            )
            flags.append(history.band_flags)

        self.assertEqual(tuple(flags), (2, 2, 0, 5, 5, 0, 0))
        # The evolved mask/history words -- mask, energy ring, band rings,
        # cursors and flags after every quantum -- are asserted
        # cross-implementation in crates/wem-analysis/tests/transient_parity.rs
        # (recorded from this oracle sequence by
        # scripts/record_transient_parity.py).

    def test_zero_and_impulse_sequence_snapshot(self):
        detector = _detector(1)

        self.assertEqual(
            _run(
                detector,
                (
                    (ZERO,),
                    (ZERO,),
                    (ZERO,),
                    (IMPULSE,),
                    (ZERO,),
                    (ZERO,),
                    (ZERO,),
                ),
                (1, 2, 3, 4, 0, 0, 1),
            ),
            (2, 2, 0, 5, 5, 0, 0),
        )

    def test_channel_results_are_combined_with_bitwise_or(self):
        left = _detector(1)
        right = _detector(1)
        stereo = _detector(2)

        for history_window in (1, 2, 3):
            left.analyze_quantum((ZERO,), history_window=history_window)
            right.analyze_quantum((ZERO,), history_window=history_window)
            stereo.analyze_quantum(
                (ZERO, ZERO), history_window=history_window
            )

        left_flags = left.analyze_quantum((ZERO,), history_window=4)
        right_flags = right.analyze_quantum((IMPULSE,), history_window=4)
        stereo_flags = stereo.analyze_quantum(
            (ZERO, IMPULSE), history_window=4
        )

        self.assertEqual((left_flags, right_flags), (0, 5))
        self.assertEqual(stereo_flags, left_flags | right_flags)

    def test_continuous_quanta_keep_history_and_reset_replays_fresh_state(self):
        detector = _detector(1)
        sequence = (
            *((ZERO,) for _ in range(25)),
            (IMPULSE,),
            (ZERO,),
            (ZERO,),
        )
        history_windows = tuple(range(1, 25)) + (24, 24, 0, 0)

        first = _run(detector, sequence, history_windows)
        self.assertEqual(first[:5], (2, 2, 0, 0, 0))
        self.assertEqual(first[-3:], (5, 5, 0))

        detector.reset()
        self.assertEqual(_run(detector, sequence, history_windows), first)

    def test_constructor_and_quantum_geometry_validation(self):
        with self.assertRaisesRegex(ValueError, "at least one channel"):
            _detector(0)
        with self.assertRaisesRegex(ValueError, "128"):
            _detector(1, bins=64)

        detector = _detector(2, bins=128)
        with self.assertRaisesRegex(ValueError, "channel count"):
            detector.analyze_quantum((ZERO,), history_window=1)
        with self.assertRaisesRegex(ValueError, "128 samples"):
            detector.analyze_quantum((ZERO, ZERO[:-1]), history_window=1)


# --------------------------------------------------------------------------
# merged from tests/unit/analysis/test_detector_feeder.py
# --------------------------------------------------------------------------

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
