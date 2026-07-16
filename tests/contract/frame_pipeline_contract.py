import copy
import json
import unittest
from pathlib import Path

from tests.contract.frame_contract_support import (
    build_frame_contract,
    first_nested_word_difference,
    first_frame_contract_difference,
)


TESTS = Path(__file__).resolve().parents[1]
FIXTURES = TESTS / "fixtures"
BASELINE = TESTS / "data" / "frame-contract" / "wwise2013_6ch_44100.json"


class FramePipelineContractTests(unittest.TestCase):
    @classmethod
    def setUpClass(cls):
        cls.expected = json.loads(BASELINE.read_text())
        cls.actual = build_frame_contract(FIXTURES / "input.wav")

    def test_complete_frame_contract_matches(self):
        self.assertEqual(len(self.actual["frames"]), 205)
        self.assertTrue(
            all("residue_q_after_sha256" in frame for frame in self.actual["frames"])
        )
        self.assertIsNone(
            first_frame_contract_difference(self.expected, self.actual)
        )

    def test_first_difference_reports_frame_and_field(self):
        changed = copy.deepcopy(self.expected)
        changed["frames"][17]["packet_size"] += 1
        self.assertEqual(
            first_frame_contract_difference(self.expected, changed),
            {
                "frame": 17,
                "field": "packet_size",
                "expected": self.expected["frames"][17]["packet_size"],
                "actual": self.expected["frames"][17]["packet_size"] + 1,
            },
        )

    def test_frame_difference_precedes_aggregate_packet_hash(self):
        changed = copy.deepcopy(self.expected)
        changed["frames"][23]["packet_sha256"] = "0" * 64
        changed["packet_stream_sha256"] = "1" * 64
        difference = first_frame_contract_difference(self.expected, changed)
        self.assertEqual(difference["frame"], 23)
        self.assertEqual(difference["field"], "packet_sha256")

    def test_raw_buffer_difference_reports_first_float32_word(self):
        expected = [[[0.0, 1.0]], [[2.0, 3.0]]]
        actual = [[[0.0, 1.0]], [[2.0, 3.25]]]
        self.assertEqual(
            first_nested_word_difference(expected, actual),
            {
                "frame": 1,
                "channel": 0,
                "bin": 1,
                "expected": 3.0,
                "actual": 3.25,
                "expected_word": "0x40400000",
                "actual_word": "0x40500000",
            },
        )

    def test_raw_buffer_difference_detects_signed_zero(self):
        difference = first_nested_word_difference([[[0.0]]], [[[-0.0]]])
        self.assertEqual(difference["frame"], 0)
        self.assertEqual(difference["channel"], 0)
        self.assertEqual(difference["bin"], 0)
        self.assertEqual(difference["expected_word"], "0x00000000")
        self.assertEqual(difference["actual_word"], "0x80000000")

    def test_raw_integer_buffer_difference_reports_int32_word(self):
        difference = first_nested_word_difference(
            [[[0, -1]]], [[[0, -2]]], word_kind="int32"
        )
        self.assertEqual(difference["expected_word"], "0xffffffff")
        self.assertEqual(difference["actual_word"], "0xfffffffe")


if __name__ == "__main__":
    unittest.main()
