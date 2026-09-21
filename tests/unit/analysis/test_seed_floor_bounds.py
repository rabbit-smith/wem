"""Bounds for the seed floor walk.

The floor walk advances a cursor through a ``total_octave_lines``-sized seed
surface. The paired build only stays inside that grid because its geometry
keeps the octave-derived ``end`` within it, so the port must report an
out-of-range cursor rather than reading past the surface. Both the kernel and
this reference used to raise an uncontrolled index error on the inner step.
"""

from __future__ import annotations

import unittest

from wwise_wem_reference.analysis.psychoacoustics.seed import (
    wwise_apply_max_seed_floor,
)


class SeedFloorBoundsTests(unittest.TestCase):
    def test_cursor_stepping_past_the_surface_is_reported(self) -> None:
        # The octave pair drives `end` to 15 while the seed surface has 10 slots.
        seed = [0.0] * 10
        floor_curve = [0.0] * 2
        with self.assertRaisesRegex(ValueError, "outside total octave lines"):
            wwise_apply_max_seed_floor(
                seed,
                floor_curve,
                [0, 30],
                first_octave=0,
                eighth_octave_lines=0,
                total_octave_lines=10,
                seed_ceiling=0.0,
            )

    def test_in_range_walk_propagates_the_seed_minimum_into_the_floor(self) -> None:
        seed = [-90.0] * 8
        floor_curve = [-120.0, -120.0]
        wwise_apply_max_seed_floor(
            seed,
            floor_curve,
            [2, 6],
            first_octave=0,
            eighth_octave_lines=0,
            total_octave_lines=8,
            seed_ceiling=-30.0,
        )
        # Cursor 2..4 covers line 2; the trailing walk covers the last line.
        # The seed minimum (-90) is below the ceiling (-30), so it passes through.
        self.assertEqual(floor_curve, [-90.0, -90.0])


if __name__ == "__main__":
    unittest.main()
