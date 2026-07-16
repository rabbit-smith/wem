"""Package-resource and cache regressions for installed codebook tables."""

from __future__ import annotations

import unittest
from unittest.mock import patch

from tests.codebook_resource_support import installed_codebook_tables
from wwise2013_wem.vorbis.bitio import BitReader
from wwise2013_wem.profiles.book_ids import (
    T219_COUNT,
    T97_COUNT,
    load_book_table,
    resolve_book_id,
)
from wwise2013_wem.profiles.codebooks import load_codebook
from wwise2013_wem.profiles.bundle import load_profile_bundle
from wwise2013_wem.vorbis.bitio import OggPack
from wwise2013_wem.profiles.resources import ResourceRef


class CodebookResourceTests(unittest.TestCase):
    tables = installed_codebook_tables()

    def tearDown(self) -> None:
        load_book_table.cache_clear()

    def test_installed_tables_are_checked_and_have_expected_counts(self):
        self.assertEqual(len(self.tables["t97"]), T97_COUNT)
        self.assertEqual(len(self.tables["t219"]), T219_COUNT)

    def test_mapping_and_runtime_share_one_cached_table_load(self):
        load_book_table.cache_clear()
        original = ResourceRef.read_json
        with patch.object(ResourceRef, "read_json", autospec=True) as read_json:
            read_json.side_effect = lambda resource: original(resource)
            ref = next(
                resource
                for resource in load_profile_bundle(verify_all=False)
                .runtime_manifest.resources.values()
                if resource.path.name == "t97.json"
            )
            first = resolve_book_id(0, self.tables)
            same_table = resolve_book_id(1, self.tables)
            runtime_table = load_book_table("t97", ref)

        self.assertEqual(read_json.call_count, 1)
        self.assertEqual((first["table"], same_table["table"]), ("t97", "t97"))
        self.assertIs(runtime_table, load_book_table("t97", ref))
        self.assertEqual(load_book_table.cache_info().misses, 1)

    def test_book_mapping_boundaries_and_errors_are_preserved(self):
        self.assertEqual(resolve_book_id(0, self.tables)["index"], 0)
        self.assertEqual(resolve_book_id(T97_COUNT - 1, self.tables)["index"], T97_COUNT - 1)
        self.assertEqual(resolve_book_id(T97_COUNT, self.tables)["index"], 0)
        self.assertEqual(
            resolve_book_id(T97_COUNT + T219_COUNT - 1, self.tables)["index"],
            T219_COUNT - 1,
        )
        self.assertEqual(resolve_book_id(-1, self.tables)["error"], "negative id")
        self.assertIn(
            "outside", resolve_book_id(T97_COUNT + T219_COUNT, self.tables)["error"]
        )
        with self.assertRaises(FileNotFoundError):
            load_book_table("unknown", ResourceRef("wwise2013_wem", "x", "0" * 64))

    def test_inline_resolved_book_remains_filesystem_independent(self):
        book = load_codebook(
            {
                "book_id": 999,
                "dim": 1,
                "entries": 2,
                "lengthlist": [1, 1],
                "maptype": 0,
            },
            self.tables,
        )
        op = OggPack()
        book.encode(op, 1)
        self.assertEqual(book.decode(BitReader(op.get_buffer())), 1)

    def test_runtime_book_uses_the_same_checked_row(self):
        resolved = resolve_book_id(214, self.tables)
        book = load_codebook(resolved, self.tables)
        row = self.tables["t219"][resolved["index"]]
        self.assertEqual((book.dim, book.entries), (row["dim"], row["entries"]))


if __name__ == "__main__":
    unittest.main()
