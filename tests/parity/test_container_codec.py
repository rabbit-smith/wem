"""The reference container codec: packet framing and the whole-file adapters.

The framing cases characterize the u16 length stream — sizes, offsets,
truncation schemas, the derived ``fmt `` sizes and the terminal overlap
excess. The adapter cases characterize ``load_wem_parts_bytes`` and
``parse_wem_bytes`` over synthetic containers and their byte-exact rebuild.
Both are one codec, so they share a module; each class keeps its own test
names and failure messages.
"""

from __future__ import annotations

import struct
import tempfile
import unittest

from pathlib import Path
from wwise_wem_reference.container.fmt import pack_pcm_ext_fmt
from wwise_wem_reference.container.packets import (
    build_packet_stream,
    extract_packets,
    recompute_vorbis_fmt_sizes,
    walk_packets,
)
from wwise_wem_reference.container.riff import build_riff
from wwise_wem_reference.container.wem import (
    build_pcm_wem,
    build_vorbis_wem,
    load_wem_parts_bytes,
    parse_wem_bytes,
)

# --------------------------------------------------------------------------
# merged from tests/parity/test_packet_framing.py
# --------------------------------------------------------------------------

# Characterization tests for Wwise packet framing helpers.

class PacketFramingTests(unittest.TestCase):
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


class VorbisFmtSizeTests(unittest.TestCase):
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

    def test_max_packet_size_counts_audio_packets_only(self) -> None:
        """The setup packet is excluded from the maximum.

        Evidence: the paired build writes 1 for an all-silent 2ch/48k stream
        whose setup packet is 215 bytes and whose audio packets are all 1 byte.
        Counting the setup packet only ever changes the field when it happens to
        be the largest one.
        """
        result = recompute_vorbis_fmt_sizes({}, [b"setup-packet", b"a", b"bb"], b"")

        self.assertEqual(result["uMaxPacketSize"], 2)

    def test_terminal_overlap_excess_is_derived_from_audio_modes(self) -> None:
        fields = {
            "dwTotalPCMFrames": 2304,
            "uBlocksize0Pow": 8,
            "uBlocksize1Pow": 11,
            "dwUnknown_0x24": 99,
            "uUnknown_0x32": 99,
        }
        # setup + four long packets produce three 1024-sample transitions:
        # 3072 rendered frames, 768 past the requested frame count.
        result = recompute_vorbis_fmt_sizes(
            fields, [b"setup", b"\x01", b"\x01", b"\x01", b"\x01"]
        )

        self.assertEqual(result["uUnknown_0x32"], 768)
        self.assertEqual(result["dwUnknown_0x24"], 768 << 16)


# --------------------------------------------------------------------------
# merged from tests/parity/test_wem_adapter.py
# --------------------------------------------------------------------------

# Path-adapter expectations for synthetic WEM containers.
#
# These tests characterize the dictionary schemas and byte preservation
# boundary of the canonical container codecs.

def _load_path(path: Path) -> dict:
    return load_wem_parts_bytes(
        path.read_bytes(), file=path.name, path=str(path)
    )


def _parse_path(path: Path) -> dict:
    return parse_wem_bytes(path.read_bytes(), file=path.name, path=str(path))


def _vorbis_fmt() -> dict:
    return {
        "wFormatTag": 0xFFFF,
        "nChannels": 6,
        "nSamplesPerSec": 44100,
        "nAvgBytesPerSec": 34381,
        "nBlockAlign": 0,
        "wBitsPerSample": 0,
        "cbSize": 48,
        "wReserved0": 0,
        "dwChannelMask": 0x3F,
        "dwTotalPCMFrames": 4096,
        "dwDataPayloadSize": 0,
        "dwUnknown_0x24": 0x03BA0000,
        "dwSeekTableSize": 0,
        "dwVorbisDataOffset": 0,
        "uMaxPacketSize": 0,
        "uUnknown_0x32": 954,
        "dwUnknown_0x34": 18180,
        "dwUnknown_0x38": 18636,
        "dwUnknown_0x3C": 0xB3DEA448,
        "uBlocksize0Pow": 8,
        "uBlocksize1Pow": 11,
    }


def _pcm_fmt(tag: int = 0xFFFE) -> dict:
    return {
        "wFormatTag": tag,
        "nChannels": 6,
        "nSamplesPerSec": 48000,
        "nAvgBytesPerSec": 576000,
        "nBlockAlign": 12,
        "wBitsPerSample": 16,
        "cbSize": 6 if tag == 0xFFFE else 0,
        "wSamples": 0,
        "dwChannelMask": 0x3F,
    }


class WemAdapterTests(unittest.TestCase):
    def _write(self, directory: str, name: str, raw: bytes) -> Path:
        path = Path(directory) / name
        path.write_bytes(raw)
        return path

    def test_path_adapters_match_pure_bytes_results_exactly(self):
        raw = build_vorbis_wem(
            _vorbis_fmt(),
            [b"setup", b"audio"],
            seek_table=b"seek",
            extra_chunks=[(b"JUNK", b"abc")],
        )
        with tempfile.TemporaryDirectory() as directory:
            path = self._write(directory, "pure-match.wem", raw)
            self.assertEqual(
                _parse_path(path),
                parse_wem_bytes(raw, file=path.name, path=str(path)),
            )
            self.assertEqual(
                _load_path(path),
                load_wem_parts_bytes(raw, file=path.name, path=str(path)),
            )

    def test_pure_bytes_defaults_supply_stable_source_metadata(self):
        raw = build_pcm_wem(_pcm_fmt(), b"\x00" * 12, junk=None)

        summary = parse_wem_bytes(raw)
        parts = load_wem_parts_bytes(raw)

        self.assertEqual((summary["file"], summary["path"]), ("<bytes>", "<bytes>"))
        self.assertEqual((parts["file"], parts["path"]), ("<bytes>", "<bytes>"))
        self.assertEqual(summary["kind"], "pcm_extensible")
        self.assertEqual(parts["pcm_data"], b"\x00" * 12)

    def test_vorbis_path_parts_summary_and_byte_exact_rebuild(self):
        packets = [b"setup-bytes", b"", b"\x01audio", b"\xff" * 5]
        seek = b"\x01\x00\x00\x00\x02\x00\x00\x00"
        extras = [(b"JUNK", b"abc"), (b"akd ", b"meta")]
        original = build_vorbis_wem(
            _vorbis_fmt(), packets, seek_table=seek, extra_chunks=extras
        )

        with tempfile.TemporaryDirectory() as directory:
            path = self._write(directory, "vorbis.wem", original)
            parts = _load_path(path)
            self.assertEqual(
                set(parts),
                {
                    "path",
                    "file",
                    "endian",
                    "raw",
                    "fmt_raw",
                    "data_raw",
                    "extras_before_data",
                    "extras_after_data",
                    "chunk_ids",
                    "kind",
                    "fmt",
                    "seek_table",
                    "packets",
                    "sizes",
                },
            )
            self.assertEqual(parts["path"], str(path))
            self.assertEqual(parts["file"], "vorbis.wem")
            self.assertEqual(parts["endian"], "le")
            self.assertEqual(parts["raw"], original)
            self.assertEqual(parts["kind"], "wwise_vorbis")
            self.assertEqual(parts["seek_table"], seek)
            self.assertEqual(parts["packets"], packets)
            self.assertEqual(parts["sizes"], [len(packet) for packet in packets])
            self.assertEqual(parts["extras_before_data"], extras)
            self.assertEqual(parts["extras_after_data"], [])
            self.assertEqual(parts["chunk_ids"], ["fmt ", "JUNK", "akd ", "data"])

            fmt = parts["fmt"]
            expected_first_audio = len(seek) + 2 + len(packets[0])
            self.assertEqual(fmt["dwSeekTableSize"], len(seek))
            self.assertEqual(fmt["dwDataPayloadSize"], len(parts["data_raw"]))
            self.assertEqual(fmt["dwFirstAudioPacketOffset"], expected_first_audio)
            self.assertEqual(fmt["dwVorbisDataOffset"], expected_first_audio)
            # largest *audio* packet: the paired build writes 1 for an all-silent
            # stream whose setup packet is 215 bytes, so the setup is not counted
            self.assertEqual(fmt["uMaxPacketSize"], max(map(len, packets[1:])))

            rebuilt = build_vorbis_wem(
                fmt,
                parts["packets"],
                seek_table=parts["seek_table"],
                endian=parts["endian"],
                extra_chunks=parts["extras_before_data"],
                recompute_sizes=False,
                fmt_raw=parts["fmt_raw"],
            )
            self.assertEqual(rebuilt, original)

            summary = _parse_path(path)
            self.assertEqual(
                set(summary),
                {
                    "file",
                    "path",
                    "size",
                    "endian",
                    "chunks",
                    "fmt_chunk_size",
                    "kind",
                    "fmt",
                    "data",
                    "has_junk",
                    "has_smpl",
                },
            )
            self.assertEqual(summary["kind"], "wwise_vorbis")
            self.assertEqual(summary["data"]["packet_count"], len(packets))
            self.assertEqual(summary["data"]["setup_packet_size"], len(packets[0]))
            self.assertTrue(summary["data"]["checks"]["data_size_field"])
            self.assertTrue(summary["data"]["checks"]["first_audio_field_eq_vorbis_off"])
            self.assertTrue(summary["data"]["checks"]["packets_consume_data"])
            self.assertTrue(summary["data"]["checks"]["vorbis_offset_vs_setup"])
            self.assertTrue(summary["has_junk"])
            self.assertFalse(summary["has_smpl"])

    def test_big_endian_vorbis_packet_framing_and_exact_rebuild(self):
        packets = [b"setup", b"a", b"audio"]
        seek = b"seek"
        extras = [(b"JUNK", b"even")]
        original = build_vorbis_wem(
            _vorbis_fmt(),
            packets,
            seek_table=seek,
            endian="be",
            extra_chunks=extras,
        )

        with tempfile.TemporaryDirectory() as directory:
            path = self._write(directory, "big-endian.wem", original)
            parts = _load_path(path)
            self.assertEqual(original[:4], b"RIFX")
            self.assertEqual(parts["endian"], "be")
            self.assertEqual(parts["seek_table"], seek)
            self.assertEqual(parts["packets"], packets)
            self.assertEqual(parts["sizes"], [5, 1, 5])

            summary = _parse_path(path)
            self.assertTrue(summary["data"]["checks"]["packets_consume_data"])
            self.assertEqual(summary["data"]["packet_count"], len(packets))
            self.assertTrue(summary["data"]["checks"]["vorbis_offset_vs_setup"])

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

    def test_pcm_extensible_path_summary_and_exact_rebuild(self):
        pcm = bytes(range(36))  # three 6-channel, 16-bit frames
        extras = [(b"smpl", b"loop"), (b"LIST", b"tags")]
        original = build_pcm_wem(
            _pcm_fmt(), pcm, junk=b"pad!", extra_chunks=extras
        )

        with tempfile.TemporaryDirectory() as directory:
            path = self._write(directory, "pcm-ext.wem", original)
            parts = _load_path(path)
            self.assertEqual(
                set(parts),
                {
                    "path",
                    "file",
                    "endian",
                    "raw",
                    "fmt_raw",
                    "data_raw",
                    "extras_before_data",
                    "extras_after_data",
                    "chunk_ids",
                    "kind",
                    "fmt",
                    "pcm_data",
                },
            )
            self.assertEqual(parts["kind"], "pcm_extensible")
            self.assertEqual(parts["pcm_data"], pcm)
            self.assertEqual(
                parts["extras_before_data"], [(b"JUNK", b"pad!"), *extras]
            )
            self.assertEqual(parts["chunk_ids"], ["fmt ", "JUNK", "smpl", "LIST", "data"])

            summary = _parse_path(path)
            self.assertEqual(summary["kind"], "pcm_extensible")
            self.assertEqual(summary["data"]["frames"], 3)
            self.assertEqual(summary["data"]["duration_sec"], round(3 / 48000, 6))
            self.assertTrue(summary["has_junk"])
            self.assertTrue(summary["has_smpl"])

            rebuilt = build_pcm_wem(
                parts["fmt"],
                parts["pcm_data"],
                endian=parts["endian"],
                junk=parts["extras_before_data"][0][1],
                extra_chunks=parts["extras_before_data"][1:],
                fmt_raw=parts["fmt_raw"],
            )
            self.assertEqual(rebuilt, original)

    def test_pcm_plain_rifx_path_adapter_and_exact_rebuild(self):
        pcm = b"\x00\x01" * 8
        original = build_pcm_wem(
            _pcm_fmt(1),
            pcm,
            endian="be",
            junk=None,
            extra_chunks=[(b"LIST", b"metadata")],
        )

        with tempfile.TemporaryDirectory() as directory:
            path = self._write(directory, "pcm-plain-rifx.wem", original)
            parts = _load_path(path)
            summary = _parse_path(path)
            self.assertEqual(original[:4], b"RIFX")
            self.assertEqual(parts["endian"], "be")
            self.assertEqual(parts["kind"], "pcm_plain")
            self.assertEqual(parts["pcm_data"], pcm)
            self.assertEqual(parts["extras_before_data"], [(b"LIST", b"metadata")])
            self.assertEqual(summary["endian"], "be")
            self.assertEqual(summary["kind"], "pcm_plain")
            self.assertEqual(summary["data"]["frames"], len(pcm) // 12)

            rebuilt = build_pcm_wem(
                parts["fmt"],
                parts["pcm_data"],
                endian=parts["endian"],
                junk=None,
                extra_chunks=parts["extras_before_data"],
                fmt_raw=parts["fmt_raw"],
            )
            self.assertEqual(rebuilt, original)

    def test_load_parts_partitions_extras_around_fmt_and_data(self):
        fmt = pack_pcm_ext_fmt(_pcm_fmt())
        raw = build_riff(
            [
                (b"PRE ", b"before-fmt"),
                (b"fmt ", fmt),
                (b"MID ", b"before-data"),
                (b"data", b"\x00" * 12),
                (b"POST", b"after-data"),
            ]
        )
        with tempfile.TemporaryDirectory() as directory:
            path = self._write(directory, "extra-order.wem", raw)
            parts = _load_path(path)
            self.assertEqual(parts["extras_before_data"], [(b"MID ", b"before-data")])
            self.assertEqual(
                parts["extras_after_data"],
                [(b"PRE ", b"before-fmt"), (b"POST", b"after-data")],
            )
            self.assertEqual(
                parts["chunk_ids"], ["PRE ", "fmt ", "MID ", "data", "POST"]
            )

    def test_missing_fmt_errors(self):
        raw = build_riff([(b"data", b"payload")])
        with tempfile.TemporaryDirectory() as directory:
            path = self._write(directory, "missing-fmt.wem", raw)
            with self.assertRaisesRegex(ValueError, "missing fmt or data"):
                _load_path(path)
            summary = _parse_path(path)
            self.assertEqual(summary["error"], "missing fmt")
            self.assertNotIn("kind", summary)


if __name__ == "__main__":
    unittest.main()
