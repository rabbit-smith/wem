from __future__ import annotations

import json
import unittest
from pathlib import Path

from wwise_wem.vorbis.bitio import BitReader
from wwise_wem.profiles.book_ids import resolve_book_id
from wwise_wem.profiles.codebooks import load_setup_codebooks
from tests.codebook_resource_support import installed_codebook_tables
from wwise_wem.vorbis.bitio import OggPack
from wwise_wem.vorbis.setup import parse_setup
from wwise_wem.vorbis.codebook import StaticCodebook, make_codewords
from wwise_wem.profiles.registry import resolve_wem_profile


class CodebookBehaviorTests(unittest.TestCase):
    def test_oggpack_lsb_and_cross_word_vectors(self) -> None:
        packed = OggPack()
        packed.write(3, 2)
        packed.write(5, 3)
        self.assertEqual((packed.endbyte, packed.endbit), (0, 5))
        self.assertEqual(packed.get_buffer(), bytes([0x17]))

        word = OggPack()
        word.write(0xDEADBEEF, 32)
        self.assertEqual(word.get_buffer(), bytes.fromhex("efbeadde"))

        crossed = OggPack()
        crossed.write(1, 1)
        crossed.write(0xFFFFFFFF, 32)
        self.assertEqual(crossed.bytes_used(), 5)

        split = OggPack()
        split.write(0x15, 5)
        split.write(0x2A, 6)
        joined = OggPack()
        joined.write(0x15 | (0x2A << 5), 11)
        self.assertEqual(split.get_buffer(), joined.get_buffer())

    def test_static_codebook_unordered_pack(self) -> None:
        lengths = [0, 2, 3, 3, 3, 3, 4, 3, 4]
        codebook = StaticCodebook(1, 9, lengths)
        self.assertFalse(codebook.is_ordered())

        expected = OggPack()
        expected.write(1, 4)
        expected.write(9, 14)
        expected.write(0, 1)
        expected.write(3, 3)
        expected.write(1, 1)
        for length in lengths:
            expected.write(bool(length), 1)
            if length:
                expected.write(length - 1, 3)
        expected.write(0, 1)
        self.assertEqual(codebook.pack(), expected.get_buffer())

    def test_setup_books_encode_decode_and_vq(self) -> None:
        profile = resolve_wem_profile(6, 44100)
        setup = parse_setup(profile.setup_packet(), channels=6)
        tables = installed_codebook_tables()
        books = load_setup_codebooks(setup["book_ids"], tables)
        self.assertEqual(len(books), setup["nbooks"])

        for codebook in books:
            entries = codebook.used_entries()[:3]
            self.assertTrue(entries)
            packed = OggPack()
            for entry in entries:
                codebook.encode(packed, entry)
            reader = BitReader(packed.get_buffer())
            self.assertEqual([codebook.decode(reader) for _ in entries], entries)
            if codebook.maptype == 1:
                self.assertGreater(codebook.quantvals, 0)
                self.assertEqual(len(codebook.vq_values(entries[0])), codebook.dim)

        lattice = load_setup_codebooks([214], installed_codebook_tables())[0]
        self.assertEqual((lattice.dim, lattice.entries, lattice.quantvals), (4, 81, 3))
        self.assertEqual(lattice.vq_values(0), [0.0] * 4)
        self.assertEqual(lattice.vq_values(1), [-1.0, 0.0, 0.0, 0.0])
        self.assertEqual(make_codewords([1, 1]), [0, 1])

    def test_published_tables_and_resolver_have_no_pointer_provenance(self) -> None:
        root = Path(__file__).resolve().parents[3]
        table_dir = root / "src" / "wwise_wem" / "data" / "profiles" / "wwise2013-6ch-44100" / "vorbis" / "codebooks"
        forbidden = {"ptr", "lengthlist_ptr", "quantlist_ptr"}
        for path in sorted(table_dir.glob("*_decoded.json")):
            for row in json.loads(path.read_text(encoding="utf-8")):
                self.assertTrue(forbidden.isdisjoint(row), path.name)
        tables = installed_codebook_tables()
        self.assertTrue(forbidden.isdisjoint(resolve_book_id(0, tables)))
        self.assertTrue(forbidden.isdisjoint(resolve_book_id(214, tables)))

    def test_modules_have_no_hidden_diagnostic_or_cli_surface(self) -> None:
        from wwise_wem.profiles import book_ids, codebooks
        from wwise_wem.vorbis import bitio, codebook

        forbidden = {
            book_ids: ("main",),
            codebooks: ("self_test", "_entry_roundtrip", "_pick_test_entries"),
            bitio: ("test_basic",),
            codebook: ("test_book0",),
        }
        for module, names in forbidden.items():
            for name in names:
                self.assertFalse(hasattr(module, name), f"{module.__name__}.{name}")


if __name__ == "__main__":
    unittest.main()
