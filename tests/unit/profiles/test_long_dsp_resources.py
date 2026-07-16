from __future__ import annotations

import unittest
from unittest.mock import patch

from wwise_wem.profiles.bundle import load_profile_bundle
from wwise_wem.profiles.psychoacoustics.long_tables import (
    load_long_psy_tables,
    table_sha256,
)
from wwise_wem.profiles.psychoacoustics.long_variants import load_long_variant
from wwise_wem.profiles.resources import ResourceRef


class LongDspResourceTests(unittest.TestCase):
    manifest = load_profile_bundle(verify_all=False).runtime_manifest
    base_ref = manifest.resource("psychoacoustics.long-base")
    modes_ref = manifest.resource("psychoacoustics.long-modes")

    def tearDown(self) -> None:
        load_long_variant.cache_clear()

    def test_manifest_refs_are_checksum_addressed(self):
        self.assertIsInstance(self.base_ref, ResourceRef)
        self.assertIsInstance(self.modes_ref, ResourceRef)
        self.assertEqual(table_sha256(self.base_ref), self.base_ref.sha256)

    def test_explicit_resources_load_all_variants(self):
        base = load_long_psy_tables(self.base_ref)
        self.assertEqual((base.n, base.sample_rate), (1024, 44100))
        self.assertEqual(len(base.seed_tone_banks), 17)
        self.assertEqual(
            load_long_variant(2, self.modes_ref, base).profile_key,
            "1024:2",
        )
        self.assertEqual(
            load_long_variant(3, self.modes_ref, base).profile_key,
            "1024:3",
        )

    def test_variant_cache_includes_resource_and_base_identity(self):
        base = load_long_psy_tables(self.base_ref)
        self.assertIs(
            load_long_variant(2, self.modes_ref, base),
            load_long_variant(2, self.modes_ref, base),
        )

    def test_resource_reader_is_used_once_per_cached_variant(self):
        base = load_long_psy_tables(self.base_ref)
        original = ResourceRef.read_json
        load_long_variant.cache_clear()
        with patch.object(ResourceRef, "read_json", autospec=True) as reader:
            reader.side_effect = lambda resource: original(resource)
            first = load_long_variant(2, self.modes_ref, base)
            self.assertIs(load_long_variant(2, self.modes_ref, base), first)
        self.assertEqual(reader.call_count, 1)


if __name__ == "__main__":
    unittest.main()
