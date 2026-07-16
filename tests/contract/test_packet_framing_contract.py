"""Characterization contract for Wwise packet framing helpers."""

from __future__ import annotations

import struct
import unittest

from wwise2013_wem.container.packets import (
    build_packet_stream,
    extract_packets,
    recompute_vorbis_fmt_sizes,
    walk_packets,
)


class PacketFramingContractTests(unittest.TestCase):
    def test_little_and_big_endian_streams_preserve_seek_and_packets(self) -> None:
        packets = [b"setup", b"", b"audio"]
        seek = b"SEEK"
        cases = (("le", "<H"), ("be", ">H"))

        for endian, size_fmt in cases:
            with self.subTest(endian=endian):
                stream = build_packet_stream(packets, seek, endian=endian)
                self.assertEqual(stream[:4], seek)
                self.assertEqual(struct.unpack_from(size_fmt, stream, 4)[0], 5)

                result = extract_packets(stream, len(seek), endian=endian)
                self.assertEqual(
                    result,
                    {
                        "ok": True,
                        "seek_table": seek,
                        "packets": packets,
                        "sizes": [5, 0, 5],
                        "packet_count": 3,
                        "end": len(stream),
                        "data_size": len(stream),
                        "setup_packet_size": 5,
                        "first_audio_offset": 11,
                        "sizes_head": [5, 0, 5],
                    },
                )

    def test_empty_stream_and_seek_only_stream_schema(self) -> None:
        empty = extract_packets(b"")
        self.assertEqual(
            empty,
            {
                "ok": True,
                "seek_table": b"",
                "packets": [],
                "sizes": [],
                "packet_count": 0,
                "end": 0,
                "data_size": 0,
                "setup_packet_size": None,
                "first_audio_offset": None,
                "sizes_head": [],
            },
        )

        seek_only = extract_packets(b"abc", 3)
        self.assertTrue(seek_only["ok"])
        self.assertEqual(seek_only["seek_table"], b"abc")
        self.assertEqual(seek_only["packets"], [])
        self.assertEqual(seek_only["end"], 3)
        self.assertEqual(seek_only["data_size"], 3)

    def test_empty_setup_packet_has_two_byte_first_audio_offset(self) -> None:
        stream = build_packet_stream([b"", b"a"])
        result = extract_packets(stream)

        self.assertTrue(result["ok"])
        self.assertEqual(result["sizes"], [0, 1])
        self.assertEqual(result["setup_packet_size"], 0)
        self.assertEqual(result["first_audio_offset"], 2)

    def test_sizes_head_is_limited_without_losing_packets(self) -> None:
        packets = [bytes([i]) * i for i in range(12)]
        result = extract_packets(build_packet_stream(packets))

        self.assertEqual(result["packet_count"], 12)
        self.assertEqual(result["packets"], packets)
        self.assertEqual(result["sizes"], list(range(12)))
        self.assertEqual(result["sizes_head"], list(range(8)))

    def test_truncated_payload_returns_error_schema_and_completed_prefix(self) -> None:
        data = struct.pack("<H", 2) + b"ok" + struct.pack("<H", 4) + b"x"
        result = extract_packets(data)

        self.assertEqual(
            result,
            {
                "ok": False,
                "seek_table": b"",
                "packets": [b"ok"],
                "sizes": [2],
                "packet_count": 1,
                "error_at": 4,
                "error_size": 4,
                "end": 4,
                "data_size": 7,
            },
        )

    def test_trailing_single_byte_is_reported_by_ok_and_end(self) -> None:
        framed = build_packet_stream([b"a"])
        result = extract_packets(framed + b"\xff")

        self.assertFalse(result["ok"])
        self.assertEqual(result["packets"], [b"a"])
        self.assertEqual(result["end"], len(framed))
        self.assertEqual(result["data_size"], len(framed) + 1)
        self.assertNotIn("error_at", result)
        self.assertEqual(result["setup_packet_size"], 1)

    def test_seek_table_bounds_are_validated(self) -> None:
        for seek_size in (-1, 4):
            with self.subTest(seek_size=seek_size):
                with self.assertRaisesRegex(ValueError, "bad seek_table_size"):
                    extract_packets(b"abc", seek_size)

    def test_builder_rejects_oversize_and_accepts_maximum_packet(self) -> None:
        maximum = b"x" * 0xFFFF
        stream = build_packet_stream([maximum])
        self.assertEqual(stream[:2], b"\xff\xff")
        self.assertEqual(extract_packets(stream)["packets"], [maximum])

        with self.assertRaisesRegex(ValueError, "packet too large: 65536"):
            build_packet_stream([b"x" * 0x10000])

    def test_walk_packets_is_metadata_only_with_stable_schema(self) -> None:
        stream = build_packet_stream([b"setup", b"audio"], b"seek")
        result = walk_packets(stream, 4)

        self.assertEqual(
            result,
            {
                "ok": True,
                "packet_count": 2,
                "end": len(stream),
                "data_size": len(stream),
                "setup_packet_size": 5,
                "first_audio_offset": 11,
                "sizes_head": [5, 5],
            },
        )
        self.assertNotIn("seek_table", result)
        self.assertNotIn("packets", result)
        self.assertNotIn("sizes", result)


class VorbisFmtSizeContractTests(unittest.TestCase):
    def test_recompute_sets_offsets_sizes_max_and_preserves_input(self) -> None:
        original = {
            "nChannels": 6,
            "wFormatTag": 0x1234,
            "dwDataPayloadSize": 999,
            "dwFirstAudioPacketOffset": 888,
            "dwVorbisDataOffset": 777,
            "uMaxPacketSize": 666,
            "unknown": "keep",
        }
        packets = [b"setup", b"a", b"long-audio"]
        seek = b"12345678"

        result = recompute_vorbis_fmt_sizes(original, packets, seek)

        self.assertIsNot(result, original)
        self.assertEqual(original["dwDataPayloadSize"], 999)
        self.assertEqual(result["unknown"], "keep")
        self.assertEqual(result["wFormatTag"], 0x1234)
        self.assertEqual(result["dwSeekTableSize"], 8)
        self.assertEqual(result["dwDataPayloadSize"], 8 + 7 + 3 + 12)
        self.assertEqual(result["dwFirstAudioPacketOffset"], 8 + 2 + 5)
        self.assertEqual(result["dwVorbisDataOffset"], 8 + 2 + 5)
        self.assertEqual(result["uMaxPacketSize"], 10)

    def test_empty_packets_use_seek_extent_and_preserve_existing_max(self) -> None:
        result = recompute_vorbis_fmt_sizes(
            {"uMaxPacketSize": 77, "unknown": 1}, [], b"seek"
        )

        self.assertEqual(result["wFormatTag"], 0xFFFF)
        self.assertEqual(result["dwSeekTableSize"], 4)
        self.assertEqual(result["dwDataPayloadSize"], 4)
        self.assertEqual(result["dwFirstAudioPacketOffset"], 4)
        self.assertEqual(result["dwVorbisDataOffset"], 4)
        self.assertEqual(result["uMaxPacketSize"], 77)
        self.assertEqual(result["unknown"], 1)

    def test_empty_setup_packet_offsets_and_max_include_all_packets(self) -> None:
        result = recompute_vorbis_fmt_sizes({}, [b"", b"audio"], b"seek")

        self.assertEqual(result["dwDataPayloadSize"], 4 + 2 + 7)
        self.assertEqual(result["dwFirstAudioPacketOffset"], 6)
        self.assertEqual(result["dwVorbisDataOffset"], 6)
        self.assertEqual(result["uMaxPacketSize"], 5)


if __name__ == "__main__":
    unittest.main()
