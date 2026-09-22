"""The carrier's resource loaders: codebooks and the DSP table banks.

Every loader reads one region of the same compiled stream and caches on the
identity it resolved. The codebook rows, the fixed banks (MDCT trigonometry,
short seed, transient), and the long psychoacoustic tables and their analysis
variants are three regions of one carrier, so their geometry, cache and
identity rules live in one module. Each class keeps its own test names and
failure messages.
"""

from __future__ import annotations

import unittest
import wwise_wem_reference.profiles.artifact as artifact
import wwise_wem_reference.profiles.psychoacoustics.config as psy_profiles

from tests.analysis_resource_support import installed_profile
from tests.codebook_resource_support import installed_codebook_tables
from unittest.mock import patch
from wwise_wem_reference.profiles.book_ids import (
    T219_COUNT,
    T97_COUNT,
    load_book_table,
    resolve_book_id,
)
from wwise_wem_reference.profiles.codebooks import load_codebook
from wwise_wem_reference.profiles.psychoacoustics.long_tables import (
    load_long_psy_tables,
)
from wwise_wem_reference.profiles.psychoacoustics.long_variants import load_long_variant
from wwise_wem_reference.profiles.psychoacoustics.short_tables import (
    load_short_psy_profiles,
)
from wwise_wem_reference.profiles.transform import load_mdct_looks
from wwise_wem_reference.profiles.transient import load_transient_tables
from wwise_wem_reference.vorbis.bitio import BitReader, OggPack

# --------------------------------------------------------------------------
# merged from tests/unit/profiles/test_codebook_resources.py
# --------------------------------------------------------------------------

# Package-resource and cache regressions for installed codebook tables.

class CodebookResourceTests(unittest.TestCase):
    tables = installed_codebook_tables()

    def tearDown(self) -> None:
        load_book_table.cache_clear()

    def test_installed_tables_are_checked_and_have_expected_counts(self):
        self.assertEqual(len(self.tables["t97"]), T97_COUNT)
        self.assertEqual(len(self.tables["t219"]), T219_COUNT)

    def test_mapping_and_runtime_share_one_cached_table_load(self):
        profile = installed_profile(6, 44100)
        load_book_table.cache_clear()
        read = profile.table
        calls: list[str] = []

        def counted(key: str):
            calls.append(key)
            return read(key)

        with patch.object(type(profile), "table", lambda _self, key: counted(key)):
            first = resolve_book_id(0, self.tables)
            same_table = resolve_book_id(1, self.tables)
            runtime_table = load_book_table("t97", profile)
            self.assertIs(runtime_table, load_book_table("t97", profile))

        self.assertEqual((first["table"], same_table["table"]), ("t97", "t97"))
        self.assertEqual(load_book_table.cache_info().misses, 1)
        # The cached call reads every column once, never twice.
        self.assertEqual(len(calls), len(set(calls)))

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
            load_book_table("unknown", installed_profile(6, 44100))

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


# --------------------------------------------------------------------------
# merged from tests/unit/profiles/test_fixed_dsp_resources.py
# --------------------------------------------------------------------------

# Regressions for the fixed DSP tables the compiled carrier holds.

class FixedDspResourceTests(unittest.TestCase):
    profile = installed_profile(6, 44100)

    def tearDown(self) -> None:
        load_mdct_looks.cache_clear()
        psy_profiles.load_short_seed_surface.cache_clear()
        load_transient_tables.cache_clear()
        load_short_psy_profiles.cache_clear()

    def test_mdct_trig_tables_preserve_all_recorded_banks(self):
        tables = load_mdct_looks(self.profile)
        self.assertEqual(set(tables), {128, 256, 512, 1024, 2048})
        for n, look in tables.items():
            self.assertEqual(len(look.trig), n + n // 4)

    def test_short_seed_and_tone_tables_preserve_geometry(self):
        seed = psy_profiles.load_short_seed_surface(self.profile)
        self.assertEqual((seed.sample_rate, seed.n), (44100, 128))
        self.assertEqual(len(seed.tone_curves), 17)
        self.assertEqual(len(seed.mask_curve), 128)
        self.assertEqual(len(seed.interval_table), 128)

    def test_carrier_tables_declare_no_digest_or_path(self):
        # The identity travels once, in the stream header. A resource tree
        # needed a path and a digest per payload; the carrier has neither, and
        # this pins that no block grows one back.
        import wwise_wem._core as core

        names = set(artifact.decode(bytes(core.profile_tables()))[0].tables)
        offenders = sorted(
            name
            for name in names
            if any(marker in name.casefold() for marker in ("sha", "digest", "path"))
        )
        self.assertEqual(offenders, [])

    def test_c5_and_short_profile_tables_preserve_geometry(self):
        transient = load_transient_tables(self.profile)
        short = load_short_psy_profiles(self.profile)
        self.assertEqual((transient.n, len(transient.window), len(transient.bands)), (128, 128, 12))
        self.assertEqual(
            tuple(profile.key for profile in short),
            ("short_look_0", "short_look_1"),
        )
        self.assertTrue(all(len(profile.mask_curves) == 3 for profile in short))

    def test_the_kernel_dump_is_decoded_once_per_process(self):
        # Every loader caches on the resolved profile, and the profile dump
        # itself is decoded once: the whole analysis surface costs one read of
        # the kernel's tables, however many loaders ask for it.
        loaders = (
            load_mdct_looks,
            psy_profiles.load_short_seed_surface,
            load_transient_tables,
            load_short_psy_profiles,
        )
        decode = artifact.decode
        artifact.compiled_profiles.cache_clear()
        try:
            with patch.object(artifact, "decode", side_effect=decode) as counted:
                first = {loader: loader(self.profile) for loader in loaders}
                for loader in loaders:
                    loader.cache_clear()
                    self.assertEqual(loader(self.profile), first[loader])
                artifact.compiled_profiles()
                artifact.compiled_profiles()
            self.assertEqual(counted.call_count, 1)
        finally:
            artifact.compiled_profiles.cache_clear()


# --------------------------------------------------------------------------
# merged from tests/unit/profiles/test_long_dsp_resources.py
# --------------------------------------------------------------------------

# Long psychoacoustic tables and analysis variants, read from the carrier.

class LongDspResourceTests(unittest.TestCase):
    profile = installed_profile(6, 44100)
    base = load_long_psy_tables(profile)

    def tearDown(self) -> None:
        load_long_variant.cache_clear()

    def test_carrier_identity_addresses_the_long_tables(self):
        # The long tables are addressed by the profile's own identity: there is
        # no resource path and no payload digest left to address them by.
        self.assertEqual(self.profile.key.channels, 6)
        self.assertEqual(self.profile.int("long_base.n"), 1024)
        self.assertEqual(self.profile.int("long_base.sample_rate"), 44100)

    def test_explicit_tables_load_all_variants(self):
        self.assertEqual((self.base.n, self.base.sample_rate), (1024, 44100))
        self.assertEqual(len(self.base.seed_tone_banks), 17)
        self.assertEqual(
            load_long_variant(2, self.profile, self.base).profile_key,
            "1024:2",
        )
        self.assertEqual(
            load_long_variant(3, self.profile, self.base).profile_key,
            "1024:3",
        )

    def test_variant_cache_includes_table_and_base_identity(self):
        self.assertIs(
            load_long_variant(2, self.profile, self.base),
            load_long_variant(2, self.profile, self.base),
        )

    def test_variant_is_reshaped_once_per_cached_identity(self):
        # The variant surface is a pure function of the carrier and the base
        # table, so the cached call rebuilds nothing the second time.
        profile = self.profile
        read = profile.table
        calls: list[str] = []

        def counted(key: str):
            calls.append(key)
            return read(key)

        load_long_variant.cache_clear()
        with patch.object(type(profile), "table", lambda _self, key: counted(key)):
            first = load_long_variant(2, profile, self.base)
            after_first = len(calls)
            self.assertIs(load_long_variant(2, profile, self.base), first)
        self.assertGreater(after_first, 0)
        self.assertEqual(len(calls), after_first)


if __name__ == "__main__":
    unittest.main()
