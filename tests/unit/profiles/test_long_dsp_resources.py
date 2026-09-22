"""Long psychoacoustic tables and analysis variants, read from the carrier."""

from __future__ import annotations

import unittest
from unittest.mock import patch

from tests.analysis_resource_support import installed_profile
from wwise_wem_reference.profiles.psychoacoustics.long_tables import load_long_psy_tables
from wwise_wem_reference.profiles.psychoacoustics.long_variants import load_long_variant


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
