"""Hybrid-window feeding: the plan's decisions, and the rows they produce.

The rows are compared, never hashed. A frame's geometry is pinned against the
plan the scheduler decided, and a frame's *content* is compared against the
bytes it is about — the plan's own PCM interval passed through the Vorbis
window — rather than against a digest of a previous run. The frames whose rows
carry the LPC prime at packet zero and the predicted tail at end of stream are
covered end to end instead: `crates/wem-core/tests/frame_pipeline_parity.rs`
compares the `window` stage of all 205 fixture frames against the live oracle
word by word, and `tests/parity/test_pcm_edges.py` encodes the 4096/4097/8192
frame boundaries through both implementations and compares whole containers.
"""

from __future__ import annotations

import unittest
from dataclasses import FrozenInstanceError, fields, replace

from wwise_wem_reference.scheduling.model import FramePlan
from wwise_wem_reference.analysis.dsp.transform import apply_vorbis_window
from wwise_wem_reference.analysis.preprocessing.windowing import (
    WindowedFrame,
    iter_pcm_windows,
)


def _sample(frame: int) -> float:
    if frame % 1024 < 32:
        return (((frame * 1103 + 12345) & 0xFFFF) - 32768) / 32768.0
    return (((frame * 37) & 0x7FF) - 1024) / 32768.0


def _pcm(frames: int) -> tuple[tuple[float, ...], ...]:
    return (tuple(_sample(frame) for frame in range(frames)),)


def _geometry(frame: WindowedFrame) -> tuple[object, ...]:
    """The plan's decisions plus the row shape."""
    return (
        frame.index,
        frame.previous,
        frame.current,
        frame.following,
        frame.center,
        tuple(len(row) for row in frame.samples),
    )


class WindowingTests(unittest.TestCase):
    def test_frame_has_one_immutable_scheduling_owner(self):
        plan = FramePlan(7, 0, 1, 0, 64, 2112, 3072, 576)
        frame = WindowedFrame(plan, 1088, ((0.0,) * 2048,))

        self.assertEqual(
            tuple(field.name for field in fields(WindowedFrame)),
            ("plan", "center", "samples"),
        )
        self.assertEqual(
            (frame.index, frame.previous, frame.current, frame.following),
            (7, 0, 1, 0),
        )
        self.assertIs(frame.plan, plan)
        with self.assertRaises(FrozenInstanceError):
            frame.current = 0  # type: ignore[misc]
        with self.assertRaises(TypeError):
            replace(frame, current=0)
        with self.assertRaisesRegex(TypeError, "requires a FramePlan"):
            WindowedFrame(None, 0, ())  # type: ignore[arg-type]

    def test_short_and_all_long_hybrid_windows_have_stable_geometry(self):
        pcm = _pcm(4096)
        short = next(iter_pcm_windows(pcm, (0, 1)))
        self.assertEqual(short.plan.current, short.current)
        self.assertEqual(short.plan.following, short.following)
        self.assertEqual(
            _geometry(short),
            (0, 0, 0, 1, 0, (256,)),
        )

        expected = {
            (0, 0): (1, 0, 1, 0, 576, (2048,)),
            (0, 1): (1, 0, 1, 1, 576, (2048,)),
            (1, 0): (1, 1, 1, 0, 1024, (2048,)),
            (1, 1): (1, 1, 1, 1, 1024, (2048,)),
        }

        for (previous, following), geometry in expected.items():
            with self.subTest(previous=previous, following=following):
                frame = tuple(iter_pcm_windows(pcm, (previous, 1, following)))[1]
                self.assertEqual(_geometry(frame), geometry)

    def test_a_long_window_is_the_windowed_view_of_its_plan_interval(self):
        """The rows themselves, against the derivation the module documents.

        For a long frame that lies strictly inside the source — no LPC prime at
        the head, no predicted tail at the end — the feeder's rule is exact:
        the plan's interval, widened by the plan's own modes, passed through
        the Vorbis window. Every word is compared, and the values here are all
        dyadic rationals, so the f32 round the feeder applies is a no-op and
        the derivation needs no rounding of its own.
        """
        frames = 8192
        blocksizes = (256, 2048)
        modes = (0,) * 32 + (1,) * 4
        pcm = _pcm(frames)
        windows = tuple(iter_pcm_windows(pcm, modes))
        self.assertEqual(len(windows), len(modes))

        checked = 0
        for frame in windows:
            if frame.current != 1:
                continue
            start = frame.plan.sample_start - blocksizes[1] // 2
            if start < 0 or start + blocksizes[frame.current] > frames:
                continue
            checked += 1
            for channel_index, row in enumerate(frame.samples):
                raw = list(pcm[channel_index][start : start + len(row)])
                self.assertEqual(
                    list(row),
                    apply_vorbis_window(
                        raw,
                        blocksizes,
                        frame.previous,
                        frame.current,
                        frame.following,
                    ),
                    f"frame {frame.index} channel {channel_index}: rows differ "
                    f"from the windowed view of interval [{start}, "
                    f"{start + len(row)})",
                )
        self.assertGreater(
            checked, 0, "the case must cover at least one interior long frame"
        )

    def test_pcm_length_edges_lock_prefix_and_terminal_tail(self):
        # Terminal windows pin the 32-tap predictor trained on one long block:
        # the mode sequence and the last centre are the scheduler's decisions,
        # and the row must reach past the source endpoint.
        expected = {
            4096: {
                "modes": (0,) * 34 + (1,),
                "last_center": 4800,
            },
            4097: {
                "modes": (0,) * 34 + (1,),
                "last_center": 4800,
            },
            8192: {
                "modes": (0,) * 66 + (1,),
                "last_center": 8896,
            },
        }

        for frames, case in expected.items():
            with self.subTest(frames=frames):
                windows = tuple(
                    iter_pcm_windows(_pcm(frames), case["modes"])
                )
                first = windows[0]
                last = windows[-1]

                self.assertEqual((first.index, first.center), (0, 0))
                self.assertTrue(any(first.samples[0][:128]))
                self.assertEqual(len(windows), len(case["modes"]))

                self.assertEqual(last.center, case["last_center"])
                last_start = last.center - len(last.samples[0]) // 2
                source_words = max(0, frames - last_start)
                self.assertLess(source_words, len(last.samples[0]))
                self.assertTrue(any(last.samples[0][source_words:]))


if __name__ == "__main__":
    unittest.main()
