"""Cross-frame analysis state: what survives a quantum, and what a reset restores.

The state is compared as f32 words, never as a digest of them. Two sessions
driven through the same input must agree word for word, a reset session must
equal a fresh one, and where a row is sparse the expectation spells the words
out (zero everywhere except the named indices) so a failure names the word that
moved. The fixture's own per-frame values are compared against the live oracle
word by word by `crates/wem-core/tests/frame_pipeline_parity.rs`; this suite
covers the cross-frame memo that no per-frame record shows.
"""

from __future__ import annotations

from tests.analysis_resource_support import installed_analysis_resources

import math
import struct
import unittest
from collections.abc import Iterable, Mapping

from wwise_wem_reference.scheduling.planner import plan_mode_sequence
from wwise_wem_reference.analysis.preprocessing.windowing import WindowedFrame
from wwise_wem_reference.analysis.session import AnalysisSession


def _stream() -> AnalysisSession:
    return AnalysisSession(
        1,
        sample_rate=44100,
        blocksizes=(256, 2048),
        resources=installed_analysis_resources(),
    )


def _words(values: Iterable[float]) -> tuple[bytes, ...]:
    """A row of f32 words as the bytes a comparison can hold."""
    return tuple(struct.pack("<f", float(value)) for value in values)


#: One f32 zero word: the untouched value of a ring or a curve.
_ZERO = struct.pack("<f", 0.0)


def _sparse_row(length: int, nonzero: Mapping[int, str]) -> tuple[bytes, ...]:
    """`length` f32 words: zero everywhere except the named indices.

    The bits are spelled the way the oracle stream spells a word (8 hex
    digits), so a sparse row is still an exact expectation of every word.
    """
    words = [_ZERO] * length
    for index, bits in nonzero.items():
        words[index] = bytes.fromhex(bits)
    return tuple(words)


def _f32_hex(value: float) -> str:
    return struct.pack("<f", float(value)).hex()


def _state_snapshot(stream: AnalysisSession) -> dict[str, object]:
    """Serialize cross-frame values without exposing allocation identity."""
    detector = tuple(
        {
            "cursor": history.cursor,
            "energy_sum": _f32_hex(history.energy_sum),
            "last_energy": _f32_hex(history.last_energy),
            "band_cursors": tuple(history.band_cursors),
            "band_flags": history.band_flags,
            "energy_ring": _words(history.energy_ring),
            "band_rings": _words(
                value for row in history.band_rings for value in row
            ),
        }
        for history in stream._transient_detector.histories
    )
    selector = stream._mode_selector
    temporal = stream._short_psy_analyzer.temporal
    return {
        "geometry": (stream.channels, stream.sample_rate, stream.blocksizes),
        "transient_quanta": stream.transient_quanta,
        "detector": detector,
        "selector": {
            "hop": selector.hop,
            "capacity": selector.capacity,
            "cooldown": selector.cooldown,
            "generated": selector.generated,
            "selected": selector.selected,
            "scan_cursor": selector.scan_cursor,
            "queue": _words(selector.queue),
        },
        "short": {
            "temporal": (
                temporal.previous_transition,
                temporal.run_count,
                temporal.tail_count,
            ),
            "channels": tuple(
                (_words(channel.state), _words(channel.history))
                for channel in stream._short_psy_analyzer.channels
            ),
        },
        "specmax": _f32_hex(stream._spectrum_peak.value),
    }


FRESH_SNAPSHOT = {
    "geometry": (1, 44100, (256, 2048)),
    "transient_quanta": 0,
    "detector": (
        {
            "cursor": 0,
            "energy_sum": "00000000",
            "last_energy": "00000000",
            "band_cursors": (0,) * 12,
            "band_flags": 0,
            "energy_ring": (_ZERO,) * 15,
            "band_rings": (_ZERO,) * (12 * 17),
        },
    ),
    "selector": {
        "hop": 64,
        "capacity": 128,
        "cooldown": 0,
        "generated": 0,
        "selected": 0,
        "scan_cursor": 1024,
        "queue": (_ZERO,) * 128,
    },
    "short": {
        "temporal": (0, 0, 0),
        "channels": (((_ZERO,) * 1024, (_ZERO,) * 128),),
    },
    "specmax": "003c1cc6",
}


def _two_quantum_stream() -> AnalysisSession:
    """One session fed the two quanta whose memo the second test pins."""
    stream = _stream()
    stream.ingest_transient_quanta(
        (
            ([0.0] * 128,),
            ([1.0 if index == 64 else 0.0 for index in range(128)],),
        )
    )
    return stream


class AnalysisStateSnapshotTests(unittest.TestCase):
    def test_reset_restores_the_fresh_behavioral_snapshot(self):
        stream = _stream()
        self.assertEqual(_state_snapshot(stream), FRESH_SNAPSHOT)

        stream.ingest_transient_quantum([[0.0] * stream.short_bins])
        self.assertNotEqual(_state_snapshot(stream), FRESH_SNAPSHOT)

        stream.reset()
        self.assertEqual(_state_snapshot(stream), FRESH_SNAPSHOT)
        # The claim is not a recorded expectation alone: a session that has
        # been fed and reset must match one that was never fed, word for word,
        # so a drifting reset cannot hide behind the constants above.
        self.assertEqual(_state_snapshot(stream), _state_snapshot(_stream()))

    def test_two_transient_quanta_have_a_stable_cross_frame_snapshot(self):
        stream = _two_quantum_stream()
        snapshot = _state_snapshot(stream)

        self.assertEqual(snapshot["transient_quanta"], 2)
        self.assertEqual(
            snapshot["detector"],
            (
                {
                    "cursor": 2,
                    "energy_sum": "2ccf723a",
                    "last_energy": "2ccf723a",
                    "band_cursors": (2,) * 12,
                    "band_flags": 2,
                    # Fifteen words, one of them non-zero: the ring is written
                    # at its cursor, so the expectation names the word that
                    # moved instead of hashing the row.
                    "energy_ring": _sparse_row(15, {1: "2ccf723a"}),
                    # Twelve bands of seventeen words: the first two words of
                    # each band carry the level and the band's own sum.
                    "band_rings": _sparse_row(
                        12 * 17,
                        {
                            0: "0000a0c2",
                            1: "1cedf1c1",
                            17: "0000a0c2",
                            18: "40c8f1c1",
                            34: "0000a0c2",
                            35: "0395f1c1",
                            51: "0100a0c2",
                            52: "772df1c1",
                            68: "0100a0c2",
                            69: "d9cbf0c1",
                            85: "0100a0c2",
                            86: "a08ef0c1",
                            102: "0100a0c2",
                            103: "fa58f0c1",
                            119: "0000a0c2",
                            120: "8611f1c1",
                            136: "0000a0c2",
                            137: "217ef1c1",
                            153: "0000a0c2",
                            154: "8de7f1c1",
                            170: "0000a0c2",
                            171: "e1f7f1c1",
                            187: "0000a0c2",
                            188: "10f7f1c1",
                        },
                    ),
                },
            ),
        )
        self.assertEqual(
            snapshot["selector"],
            {
                "hop": 64,
                "capacity": 128,
                "cooldown": 2,
                "generated": 128,
                "selected": 0,
                "scan_cursor": 1024,
                "queue": _sparse_row(128, {0: "0000803f", 1: "0000803f"}),
            },
        )
        self.assertEqual(snapshot["short"], FRESH_SNAPSHOT["short"])
        self.assertEqual(snapshot["specmax"], FRESH_SNAPSHOT["specmax"])
        # Same input, independently constructed session: the memo is a function
        # of what was fed, compared word for word.
        self.assertEqual(snapshot, _state_snapshot(_two_quantum_stream()))

    def test_short_frames_advance_shared_state_and_transition_history(self):
        stream = _stream()
        samples = tuple(
            math.sin(2.0 * math.pi * 7.0 * index / 256.0) * 0.25
            + 0.001 * (index - 128)
            for index in range(256)
        )
        first_plan, second_plan = plan_mode_sequence(
            (0, 0), blocksizes=(256, 2048), terminal_following=1
        )
        first = WindowedFrame(
            plan=first_plan,
            center=0,
            samples=(samples,),
        )
        second = WindowedFrame(
            plan=second_plan,
            center=64,
            samples=(tuple(reversed(samples)),),
        )

        fresh = _state_snapshot(_stream())
        first_analysis = stream.analyze_short(first, short_variant=0)
        first_snapshot = _state_snapshot(stream)
        second_analysis = stream.analyze_short(second, short_variant=1)
        second_snapshot = _state_snapshot(stream)

        self.assertEqual(first_snapshot["short"]["temporal"], (0, 1, 0))
        # That the shared state advances is this test's claim; the exact
        # per-frame values are compared against the live oracle by
        # `crates/wem-core/tests/frame_pipeline_parity.rs`.
        self.assertNotEqual(
            first_snapshot["short"]["channels"],
            fresh["short"]["channels"],
            "the first short frame must advance the per-channel state",
        )
        self.assertEqual(second_snapshot["short"]["temporal"], (1, 1, 1))
        self.assertNotEqual(
            second_snapshot["short"]["channels"],
            first_snapshot["short"]["channels"],
            "the second short frame must advance the per-channel state again",
        )
        # The analysis output itself is pinned exactly, and a second session
        # driven through the same two frames must leave the same memo.
        self.assertEqual(first_analysis.channel_specmax, (-5.876872539520264,))
        self.assertEqual(second_analysis.channel_specmax, (-5.876872539520264,))
        self.assertEqual(first_snapshot["specmax"], "570fbcc0")
        self.assertEqual(second_snapshot["specmax"], "570fbcc0")

        twin = _stream()
        twin.analyze_short(first, short_variant=0)
        twin.analyze_short(second, short_variant=1)
        self.assertEqual(second_snapshot, _state_snapshot(twin))


if __name__ == "__main__":
    unittest.main()
