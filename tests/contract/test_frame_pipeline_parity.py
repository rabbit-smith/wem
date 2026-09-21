"""Live per-frame parity: the shipped binding against the pure-Python oracle.

``tests/fixtures/input.wav`` is encoded at test time on both sides — the oracle
by :mod:`tests.contract.oracle_frame_values` in this process, the native kernel
through the binding the package ships (``wwise_wem._core.Encoder``) — and the
audio packets are compared frame by frame, all 205 of them, plus the frame
count and the short/long split both sides report.  Nothing is recorded and
nothing is re-recorded: a divergence is a divergence, not a stale asset.

The kernel's per-frame intermediates (analysis stages, floor posts, residue
rows) are crate surfaces this binding does not expose, so the value comparison
over all of them lives in ``crates/wem-core/tests/frame_pipeline_parity.rs``,
which drives both sides the same way through the kernel crates.  This module is
the binding half of that pair: the packets a caller actually receives.

The mode sequence the oracle selects for the fixture is also what the
scheduling unit tests used to read out of a recorded per-frame file; those
shape assertions live here now, on the live sequence.
"""

from __future__ import annotations

import unittest
from collections import Counter
from pathlib import Path

from wwise_wem import WwiseProfile, WwiseVersion
from wwise_wem import _core
from wwise_wem.adapters.wav import read_pcm_wav
from wwise_wem_reference.container.wem import load_wem_parts_bytes
from wwise_wem_reference.scheduling.model import FramePlan
from wwise_wem_reference.scheduling.planner import (
    append_samples,
    emit_block,
    initial_state,
)

from tests.contract.oracle_frame_values import frame_records

ROOT = Path(__file__).resolve().parents[2]
INPUT = ROOT / "tests" / "fixtures" / "input.wav"
SELECTION = WwiseProfile(WwiseVersion.WWISE2013, 6, 44100)

#: The fixture's audio packet count: the frame count both sides must reach.
AUDIO_PACKETS = 205


def _kernel_rows(pcm) -> list[list[int]]:
    """The fixture PCM as channel-major signed-16 rows (the binding's form)."""
    return [[int(sample * 32768.0) for sample in row] for row in pcm.channels]


class FramePipelineParityTests(unittest.TestCase):
    header: dict
    oracle_frames: list[dict]
    kernel_packets: list[bytes]
    kernel_result: object

    @classmethod
    def setUpClass(cls) -> None:
        records = list(frame_records(INPUT, stages=()))
        cls.header = records[0]
        cls.oracle_frames = records[1:]

        pcm = read_pcm_wav(INPUT)
        cls.kernel_result = _core.Encoder(SELECTION).encode_pcm(
            pcm.sample_rate, _kernel_rows(pcm)
        )
        cls.kernel_packets = load_wem_parts_bytes(bytes(cls.kernel_result.data))[
            "packets"
        ]

    def test_both_sides_reach_the_fixture_frame_count(self):
        self.assertEqual(self.header["audio_packets"], AUDIO_PACKETS)
        self.assertEqual(len(self.oracle_frames), AUDIO_PACKETS)
        self.assertEqual(self.kernel_result.audio_packets, AUDIO_PACKETS)
        # The container carries the setup packet plus one packet per frame.
        self.assertEqual(len(self.kernel_packets), AUDIO_PACKETS + 1)

    def test_every_frame_packet_matches_the_oracle(self):
        self.assertEqual(
            [record["index"] for record in self.oracle_frames],
            list(range(AUDIO_PACKETS)),
            "the oracle's frames arrive in encoder order",
        )
        audio = self.kernel_packets[1:]
        self.assertEqual(len(audio), len(self.oracle_frames), "frame count")
        for index, (packet, record) in enumerate(zip(audio, self.oracle_frames)):
            expected = bytes.fromhex(record["packet"])
            if packet == expected:
                continue
            at = next(
                (
                    offset
                    for offset, (kernel_byte, oracle_byte) in enumerate(
                        zip(packet, expected)
                    )
                    if kernel_byte != oracle_byte
                ),
                min(len(packet), len(expected)),
            )
            self.fail(
                f"frame {index}: kernel packet is {len(packet)} bytes, oracle "
                f"{len(expected)} bytes; first difference at byte {at}: "
                f"kernel 0x{packet[at:at + 1].hex() or '--'} != "
                f"oracle 0x{expected[at:at + 1].hex() or '--'}"
            )

    def test_short_and_long_frame_counts_match_the_oracle_modes(self):
        short = sum(1 for record in self.oracle_frames if record["mode"] == 0)
        long = sum(1 for record in self.oracle_frames if record["mode"] == 1)
        self.assertEqual(short + long, AUDIO_PACKETS, "every frame has a mode")
        self.assertEqual(self.kernel_result.short_packets, short)
        self.assertEqual(self.kernel_result.long_packets, long)

    def test_fixture_mode_sequence_plans_as_the_scheduler_says(self):
        modes = [record["mode"] for record in self.oracle_frames]
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


if __name__ == "__main__":
    unittest.main()
