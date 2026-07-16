from __future__ import annotations

import unittest

from wwise2013_wem.scheduling.model import FramePlan, SchedulerState
from wwise2013_wem.scheduling.planner import (
    append_samples,
    emit_block,
    initial_state,
    plan_mode_sequence,
    required_samples,
)


BLOCKSIZES = (256, 2048)


class FrameSchedulerTests(unittest.TestCase):
    def test_complete_mode_sequence_has_one_canonical_plan_stream(self):
        plans = plan_mode_sequence((0, 1, 1, 0), blocksizes=BLOCKSIZES)
        self.assertEqual(
            [
                (
                    plan.index,
                    plan.previous,
                    plan.current,
                    plan.following,
                    plan.sample_start,
                    plan.advance,
                )
                for plan in plans
            ],
            [
                (0, 0, 0, 1, 896, 576),
                (1, 0, 1, 1, 576, 1024),
                (2, 1, 1, 0, 1600, 576),
                (3, 1, 0, 1, 3072, 576),
            ],
        )

    def test_all_mode_edges_have_exact_readiness_and_advance(self):
        expected = {
            (0, 0): (1280, 128, 896, 1152),
            (0, 1): (2624, 576, 896, 1152),
            (1, 0): (1728, 576, 0, 2048),
            (1, 1): (3072, 1024, 0, 2048),
        }

        for (current, following), (
            required,
            advance,
            sample_start,
            sample_end,
        ) in expected.items():
            with self.subTest(current=current, following=following):
                underfilled = SchedulerState(
                    previous=0,
                    current=current,
                    cursor=1024,
                    filled=required - 1,
                )
                with self.assertRaisesRegex(
                    ValueError, rf"need {required} buffered samples"
                ):
                    emit_block(underfilled, following, BLOCKSIZES)

                state = SchedulerState(
                    previous=0,
                    current=current,
                    cursor=1024,
                    filled=required,
                )
                self.assertEqual(
                    required_samples(state, following, BLOCKSIZES), required
                )

                plan, next_state = emit_block(state, following, BLOCKSIZES)

                self.assertEqual(
                    plan,
                    FramePlan(
                        index=0,
                        previous=0,
                        current=current,
                        following=following,
                        sample_start=sample_start,
                        sample_end=sample_end,
                        required_filled=required,
                        advance=advance,
                    ),
                )
                self.assertEqual(
                    next_state,
                    SchedulerState(
                        previous=current,
                        current=following,
                        cursor=1024,
                        filled=required - advance,
                        buffer_base=advance,
                        emitted=1,
                    ),
                )

    def test_required_minus_one_fails_and_exact_threshold_succeeds(self):
        state = initial_state(BLOCKSIZES)
        required = required_samples(state, 1, BLOCKSIZES)

        almost_ready = append_samples(state, required - state.filled - 1)
        with self.assertRaisesRegex(
            ValueError,
            rf"need {required} buffered samples for mode 0->1; have {required - 1}",
        ):
            emit_block(almost_ready, 1, BLOCKSIZES)

        ready = append_samples(almost_ready, 1)
        plan, _ = emit_block(ready, 1, BLOCKSIZES)
        self.assertEqual(plan.required_filled, required)

    def test_continuous_planning_preserves_the_immutable_state_sequence(self):
        state = initial_state(BLOCKSIZES)
        original = state
        plans: list[FramePlan] = []
        states: list[SchedulerState] = []

        for following in (0, 1, 1, 0, 0):
            required = required_samples(state, following, BLOCKSIZES)
            state = append_samples(state, max(0, required - state.filled))
            plan, state = emit_block(state, following, BLOCKSIZES)
            plans.append(plan)
            states.append(state)

        self.assertEqual(
            original,
            SchedulerState(0, 0, 1024, 1024, buffer_base=0, emitted=0),
        )
        self.assertEqual(
            [
                (
                    plan.index,
                    plan.previous,
                    plan.current,
                    plan.following,
                    plan.sample_start,
                    plan.sample_end,
                    plan.required_filled,
                    plan.advance,
                )
                for plan in plans
            ],
            [
                (0, 0, 0, 0, 896, 1152, 1280, 128),
                (1, 0, 0, 1, 1024, 1280, 2624, 576),
                (2, 0, 1, 1, 704, 2752, 3072, 1024),
                (3, 1, 1, 0, 1728, 3776, 1728, 576),
                (4, 1, 0, 0, 3200, 3456, 1280, 128),
            ],
        )
        self.assertEqual(
            states,
            [
                SchedulerState(0, 0, 1024, 1152, 128, 1),
                SchedulerState(0, 1, 1024, 2048, 704, 2),
                SchedulerState(1, 1, 1024, 2048, 1728, 3),
                SchedulerState(1, 0, 1024, 1472, 2304, 4),
                SchedulerState(0, 0, 1024, 1344, 2432, 5),
            ],
        )


if __name__ == "__main__":
    unittest.main()
