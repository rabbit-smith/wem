import copy
import hashlib
import json
import unittest
from pathlib import Path

from tests.parity.stage_records_support import (
    build_stage_records,
    first_stage_record_difference,
    representative_frame_indices,
    transition_frame_indices,
)


TESTS = Path(__file__).resolve().parents[1]
FIXTURES = TESTS / "fixtures"
STAGES_DIR = TESTS / "data" / "stage-records" / "stages"
BASELINE = STAGES_DIR / "index.json"

FLOAT_STAGES = (
    "window",
    "coefficients",
    "raw_mdct",
    "fft",
    "remap",
    "seed",
    "post",
    "side",
)
INT_STAGES = ("floor_posts", "residue_q")


class StagePipelineTests(unittest.TestCase):
    @classmethod
    def setUpClass(cls):
        cls.expected = json.loads(BASELINE.read_text())
        cls.actual, cls.actual_dumps = build_stage_records(FIXTURES / "input.wav")

    def test_complete_stage_index_matches(self):
        self.assertEqual(len(self.actual["frames"]), 205)
        stage_fields = (
            *tuple(f"{stage}_sha256" for stage in FLOAT_STAGES),
            "floor_posts_sha256",
            "residue_q_after_sha256",
            "packet_size",
            "packet_sha256",
        )
        self.assertTrue(
            all(
                field in frame
                for frame in self.actual["frames"]
                for field in stage_fields
            )
        )
        self.assertIsNone(
            first_stage_record_difference(self.expected, self.actual),
            "stage-records index differs from the live encoder output",
        )

    def test_first_difference_reports_frame_and_stage(self):
        changed = copy.deepcopy(self.expected)
        changed["frames"][17]["coefficients_sha256"] = "0" * 64
        difference = first_stage_record_difference(self.expected, changed)
        self.assertEqual(
            (difference["frame"], difference["stage"]),
            (17, "coefficients_sha256"),
        )

    def test_container_difference_reports_segment(self):
        changed = copy.deepcopy(self.expected)
        changed["container"]["setup_sha256"] = "1" * 64
        difference = first_stage_record_difference(self.expected, changed)
        self.assertEqual(
            (difference["frame"], difference["stage"]),
            (None, "container.setup_sha256"),
        )

    def test_representative_dump_hashes_match_index(self):
        expected_dumps = self.expected["dumps"]
        self.assertEqual(set(expected_dumps), set(self.actual["dumps"]))
        for key, entry in expected_dumps.items():
            self.assertEqual(
                len(self.actual_dumps[key]),
                entry["size"],
                f"dump {key} size differs",
            )
            self.assertEqual(
                hashlib.sha256(self.actual_dumps[key]).hexdigest(),
                entry["sha256"],
                f"dump {key} bytes differ",
            )

    def test_dump_files_match_index(self):
        for key, entry in self.expected["dumps"].items():
            blob = (STAGES_DIR / entry["path"]).read_bytes()
            self.assertEqual(
                hashlib.sha256(blob).hexdigest(),
                entry["sha256"],
                f"committed dump {key} bytes differ",
            )

    def test_representative_frames_follow_the_selection_rule(self):
        frames = self.expected["frames"]
        modes = [frame["mode"] for frame in frames]
        self.assertEqual(
            self.expected["transition_frames"], list(transition_frame_indices(modes))
        )
        self.assertEqual(
            self.expected["representative_frames"],
            list(representative_frame_indices(modes)),
        )
        for index in self.expected["representative_frames"]:
            for stage in (*FLOAT_STAGES, *INT_STAGES, "packet"):
                self.assertIn(
                    f"f{index:03d}.{stage}",
                    self.expected["dumps"],
                    f"missing dump for frame {index} stage {stage}",
                )


if __name__ == "__main__":
    unittest.main()
