from __future__ import annotations

import hashlib
import struct
import unittest
from collections.abc import Iterable
from dataclasses import FrozenInstanceError, fields, replace

from wwise_wem_reference.scheduling.model import FramePlan
from wwise_wem_reference.analysis.preprocessing.windowing import WindowedFrame, iter_pcm_windows


def _sample(frame: int) -> float:
    if frame % 1024 < 32:
        return (((frame * 1103 + 12345) & 0xFFFF) - 32768) / 32768.0
    return (((frame * 37) & 0x7FF) - 1024) / 32768.0


def _pcm(frames: int) -> tuple[tuple[float, ...], ...]:
    return (tuple(_sample(frame) for frame in range(frames)),)


def _word_hash(rows: Iterable[Iterable[float]]) -> str:
    digest = hashlib.sha256()
    for row in rows:
        for value in row:
            digest.update(struct.pack("<f", float(value)))
    return digest.hexdigest()


def _snapshot(frame: WindowedFrame) -> tuple[object, ...]:
    return (
        frame.index,
        frame.previous,
        frame.current,
        frame.following,
        frame.center,
        tuple(len(row) for row in frame.samples),
        _word_hash(frame.samples),
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

    def test_short_and_all_long_hybrid_windows_have_stable_word_hashes(self):
        pcm = _pcm(4096)
        short = next(iter_pcm_windows(pcm, (0, 1)))
        self.assertEqual(short.plan.current, short.current)
        self.assertEqual(short.plan.following, short.following)
        self.assertEqual(
            _snapshot(short),
            (
                0,
                0,
                0,
                1,
                0,
                (256,),
                "db416cf12c9d101c38ebf8a38fe619b4"
                "9e6e9b30848596d0e7908f289d8aec2a",
            ),
        )

        expected = {
            (0, 0): (
                1,
                0,
                1,
                0,
                576,
                (2048,),
                "49378fc35f3574d431133ec66fb2460b"
                "af3bbcb5a815a9590dbd1cd0c451bcdd",
            ),
            (0, 1): (
                1,
                0,
                1,
                1,
                576,
                (2048,),
                "da8e533b85863f3fae34eca7273b7f3"
                "c57722046fb0b564890196038fd8d6368",
            ),
            (1, 0): (
                1,
                1,
                1,
                0,
                1024,
                (2048,),
                "ce2b0efc3b9a6deb845b39199530e553"
                "17eec1e1060f4a8943b0c641e508be65",
            ),
            (1, 1): (
                1,
                1,
                1,
                1,
                1024,
                (2048,),
                "a08195e8f201606275fcb94f1e23b97"
                "e7736910274618160dab283f9761e9690",
            ),
        }

        for (previous, following), snapshot in expected.items():
            with self.subTest(previous=previous, following=following):
                frame = tuple(
                    iter_pcm_windows(pcm, (previous, 1, following))
                )[1]
                self.assertEqual(_snapshot(frame), snapshot)

    def test_pcm_length_edges_lock_prefix_and_terminal_tail(self):
        # Terminal windows pin the 32-tap predictor trained on one long block.
        expected = {
            4096: {
                "modes": (0,) * 34 + (1,),
                "last_center": 4800,
                "last_hash": (
                    "d007eab41c074bdfb5cfd77d641afbf7"
                    "c769d3dc4696d5d65c2ea2e2ebbb4a98"
                ),
            },
            4097: {
                "modes": (0,) * 34 + (1,),
                "last_center": 4800,
                "last_hash": (
                    "d8e9ac73ce1d81411e739ed8f2abe7a7"
                    "eecc90678f69d5096696e849178b74da"
                ),
            },
            8192: {
                "modes": (0,) * 66 + (1,),
                "last_center": 8896,
                "last_hash": (
                    "f6d9e6feded020b5fcfae4fa96b9d58e"
                    "5d9a3fd4645cd09faa61ab1a2273ba06"
                ),
            },
        }
        first_hash = (
            "db416cf12c9d101c38ebf8a38fe619b4"
            "9e6e9b30848596d0e7908f289d8aec2a"
        )

        for frames, case in expected.items():
            with self.subTest(frames=frames):
                windows = tuple(
                    iter_pcm_windows(_pcm(frames), case["modes"])
                )
                first = windows[0]
                last = windows[-1]

                self.assertEqual((first.index, first.center), (0, 0))
                self.assertEqual(_word_hash(first.samples), first_hash)
                self.assertTrue(any(first.samples[0][:128]))

                self.assertEqual(last.center, case["last_center"])
                self.assertEqual(_word_hash(last.samples), case["last_hash"])
                last_start = last.center - len(last.samples[0]) // 2
                source_words = max(0, frames - last_start)
                self.assertLess(source_words, len(last.samples[0]))
                self.assertTrue(any(last.samples[0][source_words:]))


if __name__ == "__main__":
    unittest.main()
