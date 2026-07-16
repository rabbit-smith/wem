"""Zip-safe package-resource regressions for fixed DSP tables."""

from __future__ import annotations

import unittest
from unittest.mock import patch

import wwise2013_wem.profiles.psychoacoustics.config as psy_profiles
from wwise2013_wem.profiles.bundle import load_profile_bundle
from wwise2013_wem.profiles.transform import load_mdct_looks
from wwise2013_wem.profiles.transient import load_transient_tables
from wwise2013_wem.profiles.psychoacoustics.short_tables import load_short_psy_profiles
from wwise2013_wem.profiles.resources import ResourceRef


class FixedDspResourceTests(unittest.TestCase):
    manifest = load_profile_bundle(verify_all=False).runtime_manifest

    def tearDown(self) -> None:
        load_mdct_looks.cache_clear()
        psy_profiles.load_short_seed_surface.cache_clear()
        load_transient_tables.cache_clear()
        load_short_psy_profiles.cache_clear()

    def test_mdct_trig_resource_preserves_all_checked_banks(self):
        tables = load_mdct_looks(self.manifest.resource("transform.mdct"))
        self.assertEqual(set(tables), {128, 256, 512, 1024, 2048})
        for n, look in tables.items():
            self.assertEqual(len(look.trig), n + n // 4)

    def test_short_seed_and_tone_resources_preserve_geometry(self):
        seed = psy_profiles.load_short_seed_surface(
            self.manifest.resource("psychoacoustics.short-seed")
        )
        self.assertEqual((seed.sample_rate, seed.n), (44100, 128))
        self.assertEqual(len(seed.tone_curves), 17)
        self.assertEqual(len(seed.mask_curve), 128)
        self.assertEqual(len(seed.interval_table), 128)

    def test_resource_payloads_do_not_declare_digests(self):
        resource_names = (
            "transform.mdct",
            "analysis.transient",
            "psychoacoustics.short-profiles",
            "psychoacoustics.short-seed",
        )

        def digest_keys(value: object) -> list[str]:
            if isinstance(value, dict):
                found = [
                    key
                    for key in value
                    if str(key).casefold() in {"sha256", "digest"}
                ]
                return found + [
                    key
                    for child in value.values()
                    for key in digest_keys(child)
                ]
            if isinstance(value, list):
                return [key for child in value for key in digest_keys(child)]
            return []

        for name in resource_names:
            with self.subTest(resource=name):
                payload = self.manifest.resource(name).read_json()
                self.assertEqual(digest_keys(payload), [])

    def test_c5_and_short_profile_resources_preserve_geometry(self):
        transient = load_transient_tables(self.manifest.resource("analysis.transient"))
        short = load_short_psy_profiles(
            self.manifest.resource("psychoacoustics.short-profiles")
        )
        self.assertEqual((transient.n, len(transient.window), len(transient.bands)), (128, 128, 12))
        self.assertEqual(
            tuple(profile.key for profile in short),
            ("short_look_0", "short_look_1"),
        )
        self.assertTrue(all(len(profile.mask_curves) == 3 for profile in short))

    def test_each_lru_loader_reads_its_resource_once(self):
        loaders = (
            load_mdct_looks,
            psy_profiles.load_short_seed_surface,
            load_transient_tables,
            load_short_psy_profiles,
        )
        original = ResourceRef.read_json
        with patch.object(ResourceRef, "read_json", autospec=True) as read_json:
            read_json.side_effect = lambda resource: original(resource)
            refs = {
                load_mdct_looks: self.manifest.resource("transform.mdct"),
                load_transient_tables: self.manifest.resource("analysis.transient"),
                psy_profiles.load_short_seed_surface: self.manifest.resource(
                    "psychoacoustics.short-seed"
                ),
                load_short_psy_profiles: self.manifest.resource(
                    "psychoacoustics.short-profiles"
                ),
            }
            for loader in loaders:
                loader.cache_clear()
                args = (refs[loader],)
                first = loader(*args)
                self.assertIs(loader(*args), first)
        self.assertEqual(read_json.call_count, len(loaders))


if __name__ == "__main__":
    unittest.main()
