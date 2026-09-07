from __future__ import annotations

from tests.analysis_resource_support import installed_analysis_resources

import hashlib
import math
import struct
import unittest
from collections.abc import Iterable

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


def _float_digest(values: Iterable[float]) -> str:
    digest = hashlib.sha256()
    for value in values:
        digest.update(struct.pack("<f", float(value)))
    return digest.hexdigest()


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
            "energy_ring": _float_digest(history.energy_ring),
            "band_rings": _float_digest(
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
            "queue": _float_digest(selector.queue),
        },
        "short": {
            "temporal": (
                temporal.previous_transition,
                temporal.run_count,
                temporal.tail_count,
            ),
            "channels": tuple(
                (_float_digest(channel.state), _float_digest(channel.history))
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
            "energy_ring": (
                "5dcc1b5872dd9ff1c234501f1fefda01"
                "f664164e1583c3e1bb3dbea47588ab31"
            ),
            "band_rings": (
                "0645a4a67dcec462dc9f335bb0564e6e"
                "39bf12ea7e40cf8de81418210102c2d1"
            ),
        },
    ),
    "selector": {
        "hop": 64,
        "capacity": 128,
        "cooldown": 0,
        "generated": 0,
        "selected": 0,
        "scan_cursor": 1024,
        "queue": (
            "076a27c79e5ace2a3d47f9dd2e83e4ff"
            "6ea8872b3c2218f66c92b89b55f36560"
        ),
    },
    "short": {
        "temporal": (0, 0, 0),
        "channels": (
            (
                "ad7facb2586fc6e966c004d7d1d16b02"
                "4f5805ff7cb47c7a85dabd8b48892ca7",
                "076a27c79e5ace2a3d47f9dd2e83e4ff"
                "6ea8872b3c2218f66c92b89b55f36560",
            ),
        ),
    },
    "specmax": "003c1cc6",
}


class AnalysisStateSnapshotTests(unittest.TestCase):
    def test_reset_restores_the_fresh_behavioral_snapshot(self):
        stream = _stream()
        self.assertEqual(_state_snapshot(stream), FRESH_SNAPSHOT)

        stream.ingest_transient_quantum([[0.0] * stream.short_bins])
        self.assertNotEqual(_state_snapshot(stream), FRESH_SNAPSHOT)

        stream.reset()
        self.assertEqual(_state_snapshot(stream), FRESH_SNAPSHOT)

    def test_two_transient_quanta_have_a_stable_cross_frame_snapshot(self):
        stream = _stream()
        flags = stream.ingest_transient_quanta(
            (
                ([0.0] * 128,),
                ([1.0 if index == 64 else 0.0 for index in range(128)],),
            )
        )
        snapshot = _state_snapshot(stream)

        self.assertEqual(flags, (2, 2))
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
                    "energy_ring": (
                        "7ea5c319e102892b060cfbbe9a47a6ba"
                        "7e2513a9c05bab216def783aec2249d4"
                    ),
                    "band_rings": (
                        "2157d184b3d18388af25694d29a5edd7"
                        "9517b73347d1004e81f4772cc2da602b"
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
                "queue": (
                    "acd8a9a76f42cb4bf05f21a3b0e35d1"
                    "6b3855c7284499041b369484bea4fdfff"
                ),
            },
        )
        self.assertEqual(snapshot["short"], FRESH_SNAPSHOT["short"])
        self.assertEqual(snapshot["specmax"], FRESH_SNAPSHOT["specmax"])

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

        first_analysis = stream.analyze_short(first, short_variant=0)
        first_snapshot = _state_snapshot(stream)
        second_analysis = stream.analyze_short(second, short_variant=1)
        second_snapshot = _state_snapshot(stream)

        self.assertEqual(first_snapshot["short"]["temporal"], (0, 1, 0))
        self.assertEqual(
            first_snapshot["short"]["channels"],
            (
                (
                    "23376f500fb60e370cc70192b5070d63"
                    "ee0cb7c4e0d58195160c0511dbf98e7b",
                    "f717a2849033df53907d565a92c4de8e"
                    "30fd3a08cdda31de136a273f3a210fec",
                ),
            ),
        )
        self.assertEqual(second_snapshot["short"]["temporal"], (1, 1, 1))
        self.assertEqual(
            second_snapshot["short"]["channels"],
            (
                (
                    "731e5ca3718b5d37ae22efcff15e57fb"
                    "3bc4bedcb880417235ad3e292fb409b8",
                    "f717a2849033df53907d565a92c4de8e"
                    "30fd3a08cdda31de136a273f3a210fec",
                ),
            ),
        )
        self.assertEqual(first_analysis.channel_specmax, (-5.876872539520264,))
        self.assertEqual(second_analysis.channel_specmax, (-5.876872539520264,))
        self.assertEqual(first_snapshot["specmax"], "570fbcc0")
        self.assertEqual(second_snapshot["specmax"], "570fbcc0")


if __name__ == "__main__":
    unittest.main()
