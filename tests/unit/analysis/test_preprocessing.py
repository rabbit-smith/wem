"""The preprocessing path: input conditioning, window geometry, and the transform.

The conditioner rewrites the caller's rows, the feeder turns a plan into
windowed rows, and the transform supplies the window those rows are shaped
with together with the MDCT and packed FFT the next stage reads. Each class
below is one of those angles and keeps its own test names and failure
messages.
"""

from __future__ import annotations

import math
import struct
import unittest
import wwise_wem_reference.analysis.dsp.spectrum as log_spectrum
import wwise_wem_reference.analysis.dsp.transform as transform

from dataclasses import FrozenInstanceError, fields, replace
from wwise_wem_reference.analysis.config import InputConditionerConfig
from wwise_wem_reference.analysis.dsp.transform import apply_vorbis_window
from wwise_wem_reference.analysis.preprocessing.conditioner import InputConditioner
from wwise_wem_reference.analysis.preprocessing.windowing import (
    WindowedFrame,
    iter_pcm_windows,
)
from wwise_wem_reference.scheduling.model import FramePlan

# --------------------------------------------------------------------------
# merged from tests/unit/analysis/test_input_conditioner.py
# --------------------------------------------------------------------------

class InputConditionerTests(unittest.TestCase):
    def test_recorded_stereo_prefix_and_chunking_are_exact(self) -> None:
        config = InputConditionerConfig.from_bits(0x3F7F546D)
        rows = (
            (0.3812255859375, 0.352569580078125, 0.3597412109375, 0.3544921875),
            (0.080780029296875, 0.091217041015625, 0.089935302734375, 0.0841064453125),
        )
        expected = (
            (0.3812255859375, 0.3515625, 0.357818603515625, 0.35162353515625),
            (0.080780029296875, 0.09100341796875, 0.0894775390625, 0.08343505859375),
        )

        whole = InputConditioner(2, config).process(rows)
        self.assertEqual(whole, expected)

        chunked = InputConditioner(2, config)
        first = chunked.process(tuple(row[:1] for row in rows))
        rest = chunked.process(tuple(row[1:] for row in rows))
        joined = tuple(a + b for a, b in zip(first, rest))
        self.assertEqual(joined, expected)


# --------------------------------------------------------------------------
# merged from tests/unit/analysis/test_windowing.py
# --------------------------------------------------------------------------

# Hybrid-window feeding: the plan's decisions, and the rows they produce.
#
# The rows are compared, never hashed. A frame's geometry is pinned against the
# plan the scheduler decided, and a frame's *content* is compared against the
# bytes it is about — the plan's own PCM interval passed through the Vorbis
# window — rather than against a digest of a previous run. The frames whose rows
# carry the LPC prime at packet zero and the predicted tail at end of stream are
# covered end to end instead: `crates/wem-core/tests/frame_pipeline_parity.rs`
# compares the `window` stage of all 205 fixture frames against the live oracle
# word by word, and `tests/parity/test_pcm_edges.py` encodes the 4096/4097/8192
# frame boundaries through both implementations and compares whole containers.

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


# --------------------------------------------------------------------------
# merged from tests/unit/analysis/test_transform_runtime.py
# --------------------------------------------------------------------------

def f32(value):
    return struct.unpack("<f", struct.pack("<f", float(value)))[0]


class TransformRuntimeTests(unittest.TestCase):
    def test_packed_fft_anchor_vectors(self):
        self.assertEqual(
            log_spectrum.wwise_fft_packed([1.0] + [0.0] * 7),
            [1.0, 1.0, 0.0, 1.0, 0.0, 1.0, 0.0, 1.0],
        )
        constant = log_spectrum.wwise_fft_packed([1.0] * 8)
        self.assertEqual(constant[0], 8.0)
        self.assertEqual(max(abs(value) for value in constant[1:]), 0.0)

    def test_mdct_float32_spill_vector_and_direct_reference(self):
        samples = [
            f32(math.sin(0.13 * index) * (0.2 + index / 80.0))
            for index in range(64)
        ]
        actual = transform.mdct_forward(transform.make_mdct_look(64), samples)
        # The float32 spill of this synthetic vector is recorded word for word
        # on the kernel side (crates/wem-analysis/tests/transform_parity.rs,
        # recorded from this oracle stage by scripts/record_transform_parity.py).
        direct = [
            (4.0 / 64)
            * sum(
                samples[index]
                * math.cos(
                    math.pi
                    / 32.0
                    * (index + 0.5 + 16.0)
                    * (coefficient + 0.5)
                )
                for index in range(64)
            )
            for coefficient in range(32)
        ]
        self.assertLess(
            max(abs(left - right) for left, right in zip(actual, direct)), 2e-6
        )

    def test_window_float32_endpoint_anchor(self):
        self.assertEqual(struct.pack("<f", transform.vorbis_window(256)[0]).hex(), "050c7838")
        window = transform.vorbis_window(64)
        self.assertEqual(window, window[::-1])


if __name__ == "__main__":
    unittest.main()
