from __future__ import annotations

import unittest

from wwise2013_wem.container import RiffChunk, WemFmt, WemParts
from wwise2013_wem.container.riff import build_riff


class ContainerModelTests(unittest.TestCase):
    def test_riff_chunk_validates_and_defensively_copies(self) -> None:
        chunk_id = bytearray(b"JUNK")
        payload = bytearray(b"abc")
        chunk = RiffChunk(chunk_id, payload)
        chunk_id[0] = ord("X")
        payload[0] = ord("z")
        self.assertEqual((chunk.id, chunk.payload, chunk.fourcc), (b"JUNK", b"abc", "JUNK"))
        self.assertEqual(RiffChunk.from_legacy({"id": "JUNK", "payload": b"abc"}), chunk)
        with self.assertRaisesRegex(ValueError, "exactly 4 bytes"):
            RiffChunk(b"bad", b"")

    def test_fmt_view_recursively_freezes_and_thaws(self) -> None:
        fields = {"nested": {"values": [1, 2]}}
        fmt = WemFmt("wwise_vorbis", bytearray(b"fmt"), fields)
        fields["nested"]["values"].append(3)
        self.assertEqual(fmt.fields["nested"]["values"], (1, 2))
        copy = fmt.to_legacy_dict()
        copy["nested"]["values"].append(4)
        self.assertEqual(fmt.fields["nested"]["values"], (1, 2))

    def test_full_order_and_legacy_roundtrip_preserve_all_extras(self) -> None:
        fmt_raw = bytes(range(66))
        chunks = (
            RiffChunk(b"PRE ", b"before-fmt"),
            RiffChunk(b"fmt ", fmt_raw),
            RiffChunk(b"MID ", b"before-data"),
            RiffChunk(b"data", b"framed-packets"),
            RiffChunk(b"POST", b"after-data"),
        )
        raw = build_riff([chunk.to_legacy_tuple() for chunk in chunks])
        fmt = WemFmt("wwise_vorbis", fmt_raw, {"wFormatTag": "0xffff"})
        parts = WemParts(
            "le", "wwise_vorbis", fmt, chunks,
            setup_packet=bytearray(b"setup"),
            audio_packets=(bytearray(b"a0"), bytearray(b"a1")),
            seek_table=bytearray(b"seek"), raw=raw,
        )
        self.assertEqual(parts.ordered_chunks, chunks)
        self.assertEqual(parts.extras_before_data, (chunks[2],))
        self.assertEqual(parts.extras_after_data, (chunks[0], chunks[4]))
        self.assertEqual(parts.packets, (b"setup", b"a0", b"a1"))

        legacy = parts.to_legacy_dict()
        self.assertEqual(legacy["chunk_ids"], ["PRE ", "fmt ", "MID ", "data", "POST"])
        rebuilt = WemParts.from_legacy_dict(legacy)
        self.assertEqual(rebuilt.chunks, chunks)
        self.assertEqual(rebuilt.packets, parts.packets)
        self.assertEqual(rebuilt.to_bytes(), raw)

    def test_legacy_without_raw_uses_chunk_ids_for_pre_and_post_extras(self) -> None:
        fmt_raw = b"f" * 24
        legacy = {
            "endian": "be", "kind": "pcm_extensible",
            "fmt": {"wFormatTag": "0xfffe"}, "fmt_raw": fmt_raw,
            "data_raw": b"pcm",
            "extras_before_data": [(b"MID ", bytearray(b"middle"))],
            "extras_after_data": [(b"PRE ", bytearray(b"prefix")), (b"POST", b"suffix")],
            "chunk_ids": ["PRE ", "fmt ", "MID ", "data", "POST"],
        }
        parts = WemParts.from_legacy(legacy)
        legacy["extras_after_data"][0][1][0] = ord("X")
        self.assertEqual(
            [(c.id, c.payload) for c in parts.chunks],
            [(b"PRE ", b"prefix"), (b"fmt ", fmt_raw), (b"MID ", b"middle"),
             (b"data", b"pcm"), (b"POST", b"suffix")],
        )
        self.assertEqual(parts.to_legacy()["pcm_data"], b"pcm")
        self.assertTrue(parts.to_bytes().startswith(b"RIFX"))

    def test_incoherent_models_are_rejected(self) -> None:
        raw = b"f" * 24
        fmt = WemFmt("wwise_vorbis", raw, {})
        chunks = (RiffChunk(b"fmt ", raw), RiffChunk(b"data", b""))
        with self.assertRaisesRegex(ValueError, "endian"):
            WemParts("middle", "wwise_vorbis", fmt, chunks)
        with self.assertRaisesRegex(ValueError, "exactly one data"):
            WemParts("le", "wwise_vorbis", fmt, chunks[:1])
        with self.assertRaisesRegex(ValueError, "fmt view raw"):
            WemParts("le", "wwise_vorbis", WemFmt("wwise_vorbis", b"other", {}), chunks)
        with self.assertRaisesRegex(ValueError, "only Wwise Vorbis"):
            WemParts("le", "pcm_plain", WemFmt("pcm_plain", raw, {}), chunks, setup_packet=b"x")


if __name__ == "__main__":
    unittest.main()
