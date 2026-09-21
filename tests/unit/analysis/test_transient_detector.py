from __future__ import annotations

import hashlib
import struct
import unittest

import wwise_wem_reference.analysis.dsp.transform as transform
import wwise_wem_reference.analysis.transient.detector as transient
from tests.analysis_resource_support import installed_analysis_resources
from wwise_wem_reference.analysis.transient.detector import (
    TransientDetector,
    WwisePsyHistory,
    wwise_psy_mask,
)


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
        digest = hashlib.sha256()
        flags = []
        for samples, history_window in zip(
            (ZERO, ZERO, ZERO, IMPULSE, ZERO, ZERO, ZERO),
            (1, 2, 3, 4, 0, 0, 1),
        ):
            mask = wwise_psy_mask(
                samples, RESOURCES.transient, RESOURCES.mdct_looks[128], history, history_window=history_window
            )
            flags.append(history.band_flags)
            float_words = (
                *mask,
                *history.energy_ring,
                history.energy_sum,
                history.last_energy,
                *(value for row in history.band_rings for value in row),
            )
            int_words = (
                history.cursor,
                *history.band_cursors,
                history.band_flags,
            )
            for value in float_words:
                digest.update(struct.pack("<f", float(value)))
            for value in int_words:
                digest.update(struct.pack("<i", int(value)))

        self.assertEqual(tuple(flags), (2, 2, 0, 5, 5, 0, 0))
        # Kept deliberately: the mask/history word sequence has no committed
        # artifact, so the digest is its only record -- the flag tuple above
        # pins the decisions, not the evolved state.
        self.assertEqual(
            digest.hexdigest(),
            "2851d13322f1b57d7be0decaf3900bcf6419125767d5b1dcfaa0ca77a99f531f",
        )

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


if __name__ == "__main__":
    unittest.main()
