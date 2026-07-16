from __future__ import annotations

import json
import unittest
from collections import Counter
from pathlib import Path

from wwise_wem.analysis.preprocessing.detector_input import iter_detector_quanta
from wwise_wem.scheduling.model import FramePlan
from wwise_wem.scheduling.planner import (
    append_samples,
    emit_block,
    initial_state,
    required_samples,
)
from wwise_wem.scheduling.selector import ModeSelector
from wwise_wem.analysis.preprocessing.windowing import iter_pcm_windows


FRAME_CONTRACT = (
    Path(__file__).resolve().parents[2]
    / "data"
    / "frame-contract"
    / "wwise2013_6ch_44100.json"
)


def _checked_audio_modes(contract_path: Path) -> list[int]:
    """Read the checked per-frame scheduler contract."""
    contract = json.loads(contract_path.read_text(encoding="utf-8"))
    if contract.get("schema") != "wwise-wem.frame-contract.v1":
        raise AssertionError("scheduler frame-contract schema differs")
    frames = contract["frames"]
    if len(frames) != contract["audio_packets"]:
        raise AssertionError("scheduler frame-contract frame count differs")
    return [int(frame["mode"]) for frame in frames]


class ModeSelectorTests(unittest.TestCase):
    def test_quantum_flag_updates_and_cooldown(self):
        state = ModeSelector(64, 4, 0, 0, 0, 0, [])
        self.assertEqual(state.queue, [0, 0, 0, 0])

        state.apply_quantum(0, 1)
        self.assertEqual((state.cooldown, state.queue), (1, [1, 1, 0, 0]))
        state.apply_quantum(1, 2)
        self.assertEqual((state.cooldown, state.queue), (2, [1, 1, 0, 0]))
        state.apply_quantum(0, 4)
        self.assertEqual(state.cooldown, -1)

    def test_generate_grows_queue_and_rejects_bad_flag_count(self):
        state = ModeSelector(64, 4, 0, 0, 0, 0, [])
        self.assertEqual(state.generate(filled=640, flags_by_quantum=[0] * 6), 6)
        self.assertEqual((state.generated, state.capacity), (384, 12))
        self.assertEqual(state.queue, [0] * 12)

        with self.assertRaisesRegex(ValueError, "lacks transient flag rows"):
            ModeSelector(64, 4, 0, 0, 0, 0, []).generate(
                filled=640, flags_by_quantum=[0] * 5
            )
        with self.assertRaisesRegex(ValueError, "surplus transient flag rows"):
            ModeSelector(64, 4, 0, 0, 0, 0, []).generate(
                filled=640, flags_by_quantum=[0] * 7
            )

    def test_scan_and_transition_decisions(self):
        transient = ModeSelector(64, 16, 0, 640, 0, 0, [0, 0, 0, 1])
        self.assertEqual(
            transient.scan(center=100, current_mode=0, blocksizes=(256, 2048)), 0
        )
        self.assertEqual(transient.selected, 192)

        exhausted = ModeSelector(64, 16, 0, 640, 0, 0, [])
        self.assertEqual(
            exhausted.scan(center=100, current_mode=0, blocksizes=(256, 2048)), -1
        )
        boundary = ModeSelector(64, 16, 0, 640, 0, 0, [])
        self.assertEqual(boundary.scan(center=0, current_mode=0, blocksizes=(8, 16)), 1)

        self.assertEqual(
            transient.transition_code(
                center=192,
                previous_mode=0,
                current_mode=0,
                following_mode=1,
                blocksizes=(256, 2048),
            ),
            0,
        )
        self.assertEqual(
            transient.transition_code(
                center=192,
                previous_mode=1,
                current_mode=1,
                following_mode=1,
                blocksizes=(256, 2048),
            ),
            3,
        )


class SchedulerWindowTests(unittest.TestCase):
    def test_state_readiness_and_emit_snapshot(self):
        state = initial_state()
        self.assertEqual((state.previous, state.current, state.cursor, state.filled), (0, 0, 1024, 1024))
        self.assertEqual(required_samples(state, 0), 1280)
        with self.assertRaisesRegex(ValueError, "need 1280 buffered samples"):
            emit_block(state, 0)

        block, following = emit_block(append_samples(state, 256), 0)
        self.assertEqual(
            block,
            FramePlan(0, 0, 0, 0, 896, 1152, 1280, 128),
        )
        self.assertEqual(
            (following.previous, following.current, following.cursor, following.filled, following.buffer_base, following.emitted),
            (0, 0, 1024, 1152, 128, 1),
        )

    def test_checked_mode_sequence(self):
        modes = _checked_audio_modes(FRAME_CONTRACT)
        self.assertEqual(Counter(modes), Counter({0: 77, 1: 128}))
        self.assertEqual(
            Counter(zip(modes, modes[1:])),
            Counter({(0, 0): 72, (0, 1): 5, (1, 0): 4, (1, 1): 123}),
        )

        state = append_samples(initial_state(), 1 << 20)
        blocks = []
        for following in modes[1:]:
            block, state = emit_block(state, following)
            blocks.append(block)
        self.assertEqual(blocks[0], FramePlan(0, 0, 0, 0, 896, 1152, 1280, 128))
        self.assertEqual(
            blocks[-1],
            FramePlan(203, 1, 1, 1, 139328, 141376, 3072, 1024),
        )

    def test_zero_pcm_runtime_views(self):
        pcm = [[0.0] * 4096]
        quantum = next(iter_detector_quanta(pcm, count=1))
        self.assertEqual(len(quantum), 1)
        self.assertEqual(quantum[0], (0.0,) * 128)

        window = next(iter_pcm_windows(pcm, [0]))
        self.assertEqual((window.index, window.previous, window.current, window.following, window.center), (0, 0, 0, 1, 0))
        self.assertEqual(window.samples, ((0.0,) * 256,))


if __name__ == "__main__":
    unittest.main()
