"""Pure regressions for psychoacoustic temporal weighting and history."""
from __future__ import annotations

import unittest

from wwise_wem_reference.analysis.psychoacoustics.temporal import (
    TemporalKernelInputs,
    SHORT_HISTORY_RELAXATION_WIDTHS,
    rebase_history,
    relax_history,
    relax_short_history,
    update_short_history,
    compute_temporal_kernel,
)


class StateKernelTests(unittest.TestCase):
    def test_long_kernel_is_inactive_but_updates_state(self):
        result = compute_temporal_kernel(
            TemporalKernelInputs(1024, 2, 5, True, 9.0, 2, 0, 0, 1)
        )
        self.assertEqual(result.words(), (0, 1, 0.0, 0.0, 0.0, 0.0))

    def test_disabled_rate_and_transition_paths(self):
        disabled = compute_temporal_kernel(
            TemporalKernelInputs(128, 1, 0, False, 9.0, 0, 0, 0, 1)
        )
        self.assertEqual(disabled.words(), (0, 0, 0.0, 0.0, 0.0, 0.0))
        transition = compute_temporal_kernel(
            TemporalKernelInputs(128, 1, 0, True, 9.0, 2, 0, 0, 1)
        )
        self.assertEqual(transition.words(), (0, 1, 0.0, 0.0, 0.0, 0.0))

    def test_short_first_run_and_saturated_run(self):
        first_inputs = TemporalKernelInputs(128, 1, 0, True, 9.0, 0, 0, 0, 1)
        first = compute_temporal_kernel(first_inputs)
        self.assertEqual(first.active, 1)
        self.assertEqual(first.update_state, 1)
        self.assertEqual(first.lower_weight, 0.7)
        self.assertEqual(first.state_bias, 3.0)
        self.assertEqual(first.chase_seed, 7.0)
        saturated = compute_temporal_kernel(
            TemporalKernelInputs(128, 8, 0, True, 9.0, 0, 0, 0, 1)
        )
        self.assertEqual(saturated.lower_weight, 0.3)
        self.assertEqual(saturated.state_bias, 25.0)
        self.assertEqual(saturated.chase_seed, 0.0)

    def test_tail_scales_only_lower_weight(self):
        result = compute_temporal_kernel(
            TemporalKernelInputs(128, 1, 4, True, 9.0, 0, 0, 0, 1)
        )
        self.assertEqual(result.lower_weight, 0.35)
        self.assertEqual(result.state_bias, 3.0)

    def test_256_branch_remains_distinct(self):
        result = compute_temporal_kernel(
            TemporalKernelInputs(256, 1, 0, True, 9.0, 0, 0, 0, 1)
        )
        self.assertEqual(
            result.words(), (1, 1, 0.4, 0.0, 18.0, 6.0)
        )


class HistoryTests(unittest.TestCase):
    def test_width_table_shape_and_ends(self):
        self.assertEqual(len(SHORT_HISTORY_RELAXATION_WIDTHS), 128)
        self.assertEqual(SHORT_HISTORY_RELAXATION_WIDTHS[:8], (1,) * 8)
        self.assertEqual(SHORT_HISTORY_RELAXATION_WIDTHS[-1], 1)

    def test_first_short_rebases_history_to_minus_five(self):
        inputs = TemporalKernelInputs(128, 1, 0, True, 9.0, 0, 0, 0, 1)
        result = compute_temporal_kernel(inputs)
        self.assertEqual(
            rebase_history(inputs, result, [0.0] * 128, [0.0] * 128),
            [-5.0] * 128,
        )

    def test_previous_transition_rebases_from_state(self):
        inputs = TemporalKernelInputs(128, 1, 0, True, 9.0, 0, 1, 0, 1)
        result = compute_temporal_kernel(inputs)
        state = [float(index) for index in range(128)]
        history = [999.0] * 128
        self.assertEqual(
            rebase_history(inputs, result, state, history),
            [float(index) - 5.0 for index in range(128)],
        )

    def test_sliding_relaxation_and_noop_widths(self):
        self.assertEqual(
            relax_history(
                [-5.0] * 4, [-10.0] * 4, [1, 1, 1, 1], delta=5.0
            ),
            [-5.0] * 4,
        )
        self.assertEqual(
            relax_history(
                [-100.0, -100.0, -100.0],
                [0.0, -100.0, -100.0],
                [3, 1, 1],
                delta=5.0,
            ),
            [-100.0, -95.0, -95.0],
        )

    def test_history_geometry_validation(self):
        inputs = TemporalKernelInputs(128, 1, 0, True, 9.0, 0, 0, 0, 1)
        with self.assertRaisesRegex(ValueError, "length must equal n"):
            rebase_history(inputs, state=[0.0], history=[0.0], result=compute_temporal_kernel(inputs))
        with self.assertRaisesRegex(ValueError, "equal length"):
            relax_history([0.0], [0.0, 0.0], [1], delta=5.0)
        with self.assertRaisesRegex(ValueError, "nonpositive"):
            relax_history([-100.0, -100.0], [0.0, 0.0], [2, 0], delta=5.0)

    def test_complete_128_relaxation_keeps_cleared_first_frame(self):
        self.assertEqual(
            relax_short_history([-5.0] * 128, [-10.0] * 128),
            [-5.0] * 128,
        )

    def test_short_history_direct_raw_write(self):
        inputs = TemporalKernelInputs(128, 1, 0, True, 9.0, 0, 0, 0, 1)
        result = update_short_history(
            inputs,
            state=[-20.0] * 128,
            history=[0.0] * 128,
            raw=[10.0] * 128,
            seed=[-20.0] * 128,
            remap=[0.0] * 128,
            mask_curve=[0.0] * 128,
            curve_cap=0.0,
        )
        self.assertEqual(result, [10.0] * 128)

if __name__ == "__main__":
    unittest.main()
