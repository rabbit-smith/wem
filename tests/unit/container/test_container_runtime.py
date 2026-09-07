"""Runtime regressions for WEM RIFF fields, packet framing, and rebuilding."""
from __future__ import annotations

import struct
import tempfile
import unittest
from pathlib import Path

from wwise_wem_reference.container.fmt import pack_vorbis_fmt, parse_vorbis_fmt
from wwise_wem_reference.container.packets import build_packet_stream, extract_packets
from wwise_wem_reference.container.riff import build_riff, parse_chunks
from wwise_wem_reference.container.wem import (
    build_vorbis_wem,
    load_wem_parts_bytes,
    parse_wem_bytes,
)


def _fmt_fields() -> dict:
    return {
        "nChannels": 2,
        "nSamplesPerSec": 44100,
        "nAvgBytesPerSec": 12000,
        "dwTotalPCMFrames": 4096,
        "dwDataPayloadSize": 0,
        "dwChannelMask": 3,
        "uBlocksize0Pow": 8,
        "uBlocksize1Pow": 11,
    }


class FormatAndPacketTests(unittest.TestCase):
    def test_fmt66_pack_parse_preserves_layout_fields(self):
        payload = pack_vorbis_fmt(_fmt_fields())
        self.assertEqual(len(payload), 66)
        parsed = parse_vorbis_fmt(payload)
        self.assertEqual(parsed["wFormatTag"], "0xffff")
        self.assertEqual(parsed["nChannels"], 2)
        self.assertEqual(parsed["nSamplesPerSec"], 44100)
        self.assertEqual(parsed["dwTotalPCMFrames"], 4096)
        self.assertEqual(parsed["blocksize0"], 256)
        self.assertEqual(parsed["blocksize1"], 2048)

    def test_fmt66_raw_override_validates_size(self):
        raw = bytes(range(66))
        self.assertEqual(pack_vorbis_fmt({"raw_hex": raw.hex()}), raw)
        with self.assertRaisesRegex(ValueError, "fmt len"):
            pack_vorbis_fmt({"raw_hex": "00"})

    def test_packet_framing_roundtrips_little_and_big_endian(self):
        packets = [b"setup", b"a", b"audio"]
        seek = b"seek"
        for endian in ("le", "be"):
            with self.subTest(endian=endian):
                stream = build_packet_stream(packets, seek, endian=endian)
                parsed = extract_packets(stream, len(seek), endian=endian)
                self.assertTrue(parsed["ok"])
                self.assertEqual(parsed["seek_table"], seek)
                self.assertEqual(parsed["packets"], packets)
                self.assertEqual(parsed["sizes"], [5, 1, 5])

    def test_packet_framing_rejects_oversized_payload(self):
        with self.assertRaisesRegex(ValueError, "packet too large"):
            build_packet_stream([b"x" * 0x10000])


class RiffAndWemTests(unittest.TestCase):
    def test_final_odd_chunk_padding_matches_wwise_and_strict_riff(self):
        compact = build_riff([(b"data", b"x")])
        strict = build_riff([(b"data", b"x")], pad_final=True)
        self.assertEqual(compact[-1:], b"x")
        self.assertEqual(strict[-2:], b"x\x00")
        self.assertEqual(len(strict), len(compact) + 1)
        self.assertEqual(struct.unpack_from("<I", compact, 4)[0], len(compact) - 8)
        self.assertEqual(struct.unpack_from("<I", strict, 4)[0], len(strict) - 8)

    def test_vorbis_build_load_and_rebuild_is_byte_exact(self):
        packets = [b"setup", b"a", b"audio"]
        seek = b"seek"
        original = build_vorbis_wem(
            _fmt_fields(),
            packets,
            seek_table=seek,
            extra_chunks=[(b"JUNK", b"abc")],
        )
        with tempfile.TemporaryDirectory() as directory:
            path = Path(directory) / "sample.wem"
            path.write_bytes(original)
            parts = load_wem_parts_bytes(
                path.read_bytes(), file=path.name, path=str(path)
            )
            self.assertEqual(parts["kind"], "wwise_vorbis")
            self.assertEqual(parts["packets"], packets)
            self.assertEqual(parts["seek_table"], seek)
            self.assertEqual(parts["extras_before_data"], [(b"JUNK", b"abc")])

            rebuilt = build_vorbis_wem(
                parts["fmt"],
                parts["packets"],
                seek_table=parts["seek_table"],
                endian=parts["endian"],
                extra_chunks=parts["extras_before_data"],
                recompute_sizes=False,
                fmt_raw=parts["fmt_raw"],
            )
            self.assertEqual(rebuilt, original)

            summary = parse_wem_bytes(
                path.read_bytes(), file=path.name, path=str(path)
            )
            self.assertEqual(summary["kind"], "wwise_vorbis")
            self.assertTrue(summary["data"]["checks"]["packets_consume_data"])

    def test_chunk_parser_handles_rifx(self):
        raw = build_riff([(b"data", b"xy")], endian="be")
        endian, chunks = parse_chunks(raw)
        self.assertEqual(endian, "be")
        self.assertEqual(chunks[0]["payload"], b"xy")

if __name__ == "__main__":
    unittest.main()
