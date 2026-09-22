"""The container codec: RIFF chunks, the ``fmt `` payloads, and packet framing.

Three files about one codec. The RIFF layer builds and parses chunk lists in
both endiannesses, the ``fmt `` layer round-trips the 66-byte Vorbis and
24-byte extensible payloads, and the runtime layer puts them together into a
whole WEM and back. Each class below keeps its own test names and failure
messages.
"""

from __future__ import annotations

import struct
import tempfile
import unittest
import wwise_wem_reference.container as container
import wwise_wem_reference.container.fmt as fmt_codec

from pathlib import Path
from wwise_wem_reference.container.fmt import pack_vorbis_fmt, parse_vorbis_fmt
from wwise_wem_reference.container.packets import build_packet_stream, extract_packets
from wwise_wem_reference.container.riff import build_riff, parse_chunks
from wwise_wem_reference.container.wem import (
    build_vorbis_wem,
    load_wem_parts_bytes,
    parse_wem_bytes,
)

# --------------------------------------------------------------------------
# merged from tests/unit/container/test_riff_codec.py
# --------------------------------------------------------------------------

class RiffCodecTests(unittest.TestCase):
    def test_build_and_parse_little_and_big_endian_roundtrip(self) -> None:
        cases = (("le", b"RIFF", "<I"), ("be", b"RIFX", ">I"))
        for endian, magic, size_fmt in cases:
            with self.subTest(endian=endian):
                raw = build_riff(
                    [(b"fmt ", b"ab"), (b"data", b"xyz")], endian=endian
                )

                parsed_endian, chunks = parse_chunks(raw)

                self.assertEqual(raw[:4], magic)
                self.assertEqual(struct.unpack_from(size_fmt, raw, 4)[0], len(raw) - 8)
                self.assertEqual(parsed_endian, endian)
                self.assertEqual(
                    [(c["id"], c["size"], c["payload"]) for c in chunks],
                    [("fmt ", 2, b"ab"), ("data", 3, b"xyz")],
                )

    def test_odd_intermediate_chunk_is_padded_and_aligned(self) -> None:
        raw = build_riff([(b"JUNK", b"abc"), (b"data", b"xy")])
        _, chunks = parse_chunks(raw)

        self.assertEqual(raw[23], 0)
        self.assertEqual(chunks[0]["off"], 12)
        self.assertEqual(chunks[1]["off"], 24)
        self.assertEqual(chunks[1]["payload"], b"xy")

    def test_final_odd_padding_is_optional_and_included_in_size(self) -> None:
        unpadded = build_riff([(b"data", b"abc")])
        padded = build_riff([(b"data", b"abc")], pad_final=True)

        self.assertTrue(unpadded.endswith(b"abc"))
        self.assertTrue(padded.endswith(b"abc\x00"))
        self.assertEqual(len(padded), len(unpadded) + 1)
        self.assertEqual(struct.unpack_from("<I", padded, 4)[0], len(padded) - 8)
        self.assertEqual(parse_chunks(unpadded)[1][0]["payload"], b"abc")
        self.assertEqual(parse_chunks(padded)[1][0]["payload"], b"abc")

    def test_oversized_riff_returns_available_chunk_payload(self) -> None:
        raw = (
            b"RIFF"
            + struct.pack("<I", 4 + 8 + 5)
            + b"WAVE"
            + b"data"
            + struct.pack("<I", 5)
            + b"xy"
        )

        endian, chunks = parse_chunks(raw)

        self.assertEqual(endian, "le")
        self.assertEqual(
            chunks, [{"id": "data", "size": 5, "off": 12, "payload": b"xy"}]
        )

    def test_truncated_chunk_header_is_ignored(self) -> None:
        raw = b"RIFF" + struct.pack("<I", 7) + b"WAVEabc"
        self.assertEqual(parse_chunks(raw), ("le", []))

    def test_invalid_and_short_header_errors_are_preserved(self) -> None:
        with self.assertRaisesRegex(ValueError, "not RIFF/RIFX"):
            parse_chunks(b"NOPE")
        with self.assertRaises(struct.error):
            parse_chunks(b"RIFF")

    def test_builder_rejects_bad_endian_and_fourcc(self) -> None:
        with self.assertRaises(ValueError):
            build_riff([], endian="middle")
        with self.assertRaisesRegex(ValueError, "bad chunk id"):
            build_riff([(b"bad", b"")])


# --------------------------------------------------------------------------
# merged from tests/unit/container/test_fmt_codec.py
# --------------------------------------------------------------------------

def _all_vorbis_fields() -> dict:
    return {
        "wFormatTag": "0xffff",
        "nChannels": 6,
        "nSamplesPerSec": 44100,
        "nAvgBytesPerSec": 34381,
        "nBlockAlign": 7,
        "wBitsPerSample": 16,
        "cbSize": 48,
        "wReserved0": 0x1234,
        "dwChannelMask": "0x3f",
        "dwTotalPCMFrames": 0x10203040,
        "dwFirstAudioPacketOffset": 0x11223344,
        "dwDataPayloadSize": 0x55667788,
        "dwUnknown_0x24": "0x03ba0000",
        "dwSeekTableSize": 0x12345678,
        "dwVorbisDataOffset": 0x22334455,
        "uMaxPacketSize": 0xABCD,
        "uUnknown_0x32": 0x1357,
        "dwUnknown_0x34": 0x89ABCDEF,
        "dwUnknown_0x38": 0x76543210,
        "dwUnknown_0x3C": "0xb3dea448",
        "uBlocksize0Pow": 8,
        "uBlocksize1Pow": 11,
    }


class VorbisFmtCodecTests(unittest.TestCase):
    def test_all_66_bytes_and_fields_roundtrip(self) -> None:
        fields = _all_vorbis_fields()
        encoded = fmt_codec.pack_vorbis_fmt(fields)
        self.assertEqual(len(encoded), 66)
        parsed = fmt_codec.parse_vorbis_fmt(encoded)
        self.assertEqual(parsed["wFormatTag"], "0xffff")
        self.assertEqual(parsed["dwChannelMask"], "0x3f")
        self.assertEqual(parsed["dwUnknown_0x24"], "0x3ba0000")
        self.assertEqual(parsed["dwUnknown_0x3C"], "0xb3dea448")
        for name in (
            "nChannels",
            "nSamplesPerSec",
            "nAvgBytesPerSec",
            "nBlockAlign",
            "wBitsPerSample",
            "cbSize",
            "wReserved0",
            "dwTotalPCMFrames",
            "dwFirstAudioPacketOffset",
            "dwDataPayloadSize",
            "dwSeekTableSize",
            "dwVorbisDataOffset",
            "uMaxPacketSize",
            "uUnknown_0x32",
            "dwUnknown_0x34",
            "dwUnknown_0x38",
            "uBlocksize0Pow",
            "uBlocksize1Pow",
        ):
            self.assertEqual(parsed[name], fields[name], name)
        self.assertEqual((parsed["blocksize0"], parsed["blocksize1"]), (256, 2048))
        self.assertEqual(fmt_codec.pack_vorbis_fmt(parsed), encoded)

    def test_raw_override_and_malformed_payload_errors(self) -> None:
        raw = bytes((index * 37) & 0xFF for index in range(66))
        self.assertEqual(fmt_codec.pack_vorbis_fmt({"raw_hex": raw.hex()}), raw)
        with self.assertRaisesRegex(ValueError, "fmt len"):
            fmt_codec.pack_vorbis_fmt({"raw_hex": "00"})
        with self.assertRaises(ValueError):
            fmt_codec.pack_vorbis_fmt({"raw_hex": "not hex"})
        with self.assertRaisesRegex(ValueError, "expected 66 bytes"):
            fmt_codec.parse_vorbis_fmt(b"\0" * 65)


class PcmFmtCodecTests(unittest.TestCase):
    def test_extensible_24_byte_roundtrip(self) -> None:
        fields = {
            "wFormatTag": "0xfffe",
            "nChannels": 6,
            "nSamplesPerSec": 48000,
            "nAvgBytesPerSec": 576000,
            "nBlockAlign": 12,
            "wBitsPerSample": 16,
            "cbSize": 6,
            "wSamples": 16,
            "dwChannelMask": "0x3f",
        }
        encoded = fmt_codec.pack_pcm_ext_fmt(fields)
        self.assertEqual(len(encoded), 24)
        parsed = fmt_codec.parse_pcm_ext_fmt(encoded)
        self.assertEqual(parsed, fields)
        self.assertEqual(fmt_codec.pack_pcm_ext_fmt(parsed), encoded)

    def test_plain_pcm_and_raw_override_preserve_existing_shapes(self) -> None:
        plain = fmt_codec.pack_pcm_ext_fmt(
            {
                "wFormatTag": 1,
                "nChannels": 2,
                "nSamplesPerSec": 44100,
                "wBitsPerSample": 16,
            }
        )
        self.assertEqual(len(plain), 18)
        self.assertEqual(fmt_codec.parse_pcm_ext_fmt(plain)["wFormatTag"], "0x1")
        raw = bytes(range(24))
        self.assertEqual(fmt_codec.pack_pcm_ext_fmt({"raw_hex": raw.hex()}), raw)


class ContainerExportTests(unittest.TestCase):
    def test_container_exports_share_codec_identity(self) -> None:
        self.assertIs(container.parse_vorbis_fmt, fmt_codec.parse_vorbis_fmt)
        self.assertIs(container.pack_vorbis_fmt, fmt_codec.pack_vorbis_fmt)


# --------------------------------------------------------------------------
# merged from tests/unit/container/test_container_runtime.py
# --------------------------------------------------------------------------

# Runtime regressions for WEM RIFF fields, packet framing, and rebuilding.

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
