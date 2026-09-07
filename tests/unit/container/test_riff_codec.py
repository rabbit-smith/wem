from __future__ import annotations

import struct
import unittest

from wwise_wem_reference.container.riff import build_riff, parse_chunks


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

if __name__ == "__main__":
    unittest.main()
