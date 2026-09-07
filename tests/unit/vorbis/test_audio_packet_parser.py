"""Pure packet-header, floor, and residue parser regressions."""
from __future__ import annotations

import unittest

import wwise_wem_reference.vorbis.packet_decoder as parser_module
from wwise_wem_reference.vorbis.bitio import BitReader
from wwise_wem_reference.vorbis.bitio import OggPack
from wwise_wem_reference.vorbis.packet_decoder import (
    decode_floor1_body,
    extract_packets,
    parse_audio_header,
    parse_audio_packet,
)


def _setup(*, channels: int = 2) -> dict:
    floor = {
        "multiplier": 1,
        "x_list": [],
        "partitions": 0,
        "partition_classes": [],
        "class_dims": [],
        "class_subs": [],
        "class_masterbooks": [],
        "subclass_books": [],
    }
    residue = {
        "begin": 0,
        "end": 0,
        "partition_size": 1,
        "classbook": 0,
        "classifications": 1,
        "cascades": [0],
        "books": [[-1] * 8],
    }
    return {
        "nmodes": 2,
        "modes": [
            {"blockflag": 0, "mapping": 0},
            {"blockflag": 1, "mapping": 0},
        ],
        "maps": [
            {
                "submaps": 1,
                "floors": [0],
                "residues": [0],
                "chmux": [0] * channels,
            }
        ],
        "floors": [floor],
        "residues": [residue],
    }


class PacketExtractionTests(unittest.TestCase):
    def test_extracts_size_prefixed_packets(self):
        data = b"\x02\x00ab\x01\x00z"
        self.assertEqual(extract_packets(data), [b"ab", b"z"])
        self.assertEqual(extract_packets(b"skip" + data, 4), [b"ab", b"z"])

    def test_rejects_truncated_packet_and_trailing_byte(self):
        with self.assertRaisesRegex(ValueError, "truncated packet"):
            extract_packets(b"\x03\x00ab")
        with self.assertRaisesRegex(ValueError, "trailing bytes"):
            extract_packets(b"\x01\x00ab")


class AudioHeaderTests(unittest.TestCase):
    def test_long_header_serializes_only_mode(self):
        br = BitReader(b"\x01")
        header = parse_audio_header(br, _setup())
        self.assertEqual(header["mode"], 1)
        self.assertEqual(header["blockflag"], 1)
        self.assertEqual(header["bit_end"], 1)
        self.assertIsNone(header["prev_window"])
        self.assertIsNone(header["next_window"])

    def test_rejects_mode_outside_setup(self):
        setup = _setup()
        setup["nmodes"] = 3
        setup["modes"].append({"blockflag": 0, "mapping": 0})
        with self.assertRaisesRegex(ValueError, "out of range"):
            parse_audio_header(BitReader(b"\x03"), setup)


class FloorAndResidueTests(unittest.TestCase):
    def test_minimal_floor_body_reads_two_y_posts(self):
        op = OggPack()
        op.write(17, 8)
        op.write(99, 8)
        floor = _setup(channels=1)["floors"][0]
        self.assertEqual(
            decode_floor1_body(BitReader(op.get_buffer()), floor, []),
            [17, 99],
        )

    def test_one_byte_long_zero_floor_packet_is_complete(self):
        parsed = parse_audio_packet(b"\x01", _setup(), [], 2)
        self.assertTrue(parsed["complete"])
        self.assertEqual(parsed["header"]["mode"], 1)
        self.assertEqual(parsed["floors"]["nonzero"], [0, 0])
        self.assertEqual(parsed["residues"][0]["status"], "empty")

    def test_nonzero_floor_body_reaches_empty_residue(self):
        op = OggPack()
        op.write(0, 1)
        op.write(1, 1)
        op.write(17, 8)
        op.write(99, 8)
        parsed = parse_audio_packet(op.get_buffer(), _setup(channels=1), [], 1)
        self.assertTrue(parsed["complete"])
        self.assertEqual(parsed["floors"]["nonzero"], [1])
        self.assertEqual(parsed["floors"]["y0_head"], [17, 99])
        self.assertEqual(parsed["residues"][0]["status"], "empty")

    def test_reports_header_and_floor_eof(self):
        header_eof = parse_audio_packet(b"", _setup(), [], 2)
        self.assertFalse(header_eof["complete"])
        self.assertRegex(header_eof["error"], "header")

        op = OggPack()
        op.write(0, 1)
        op.write(1, 1)
        floor_eof = parse_audio_packet(
            op.get_buffer(), _setup(channels=1), [], 1
        )
        self.assertFalse(floor_eof["complete"])
        self.assertRegex(floor_eof["error"], "floor body")

    def test_module_has_no_file_analyzer_or_hidden_cli(self):
        for name in ("analyze_wem_audio", "self_test", "main"):
            self.assertFalse(hasattr(parser_module, name), name)


if __name__ == "__main__":
    unittest.main()
