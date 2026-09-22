"""Regressions for the fixed DSP tables the compiled carrier holds."""

from __future__ import annotations

import unittest
from unittest.mock import patch

import wwise_wem_reference.profiles.artifact as artifact
import wwise_wem_reference.profiles.psychoacoustics.config as psy_profiles
from tests.analysis_resource_support import installed_profile
from wwise_wem_reference.profiles.psychoacoustics.short_tables import load_short_psy_profiles
from wwise_wem_reference.profiles.transform import load_mdct_looks
from wwise_wem_reference.profiles.transient import load_transient_tables


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


if __name__ == "__main__":
    unittest.main()
