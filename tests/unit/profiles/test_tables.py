"""The compiled carrier's tables: identity, geometry and loader agreement."""

import hashlib
import unittest

from tests.analysis_resource_support import installed_profile
from wwise_wem_reference.profiles.psychoacoustics.long_tables import load_long_psy_tables
from wwise_wem_reference.profiles.psychoacoustics.long_variants import load_long_variant
from wwise_wem_reference.profiles.psychoacoustics.short_tables import load_short_psy_profiles
from wwise_wem_reference.profiles.transform import load_mdct_looks
from wwise_wem_reference.profiles.transient import load_transient_tables


class CarrierTableTests(unittest.TestCase):
    profile = installed_profile(6, 44100)

    def test_recorded_setup_digest_matches_the_carried_packet(self):
        # The one digest the carrier records is the setup identity: it is the
        # payload half of the chain the manifest used to walk, and it still
        # has to describe the bytes the carrier hands out.
        self.assertEqual(
            hashlib.sha256(self.profile.setup_packet).hexdigest(),
            self.profile.setup_sha256,
        )
        self.assertEqual(
            self.profile.key.quality_setup_identity, f"sha256:{self.profile.setup_sha256}"
        )

    def test_analysis_tables_load_and_validate(self):
        self.assertEqual(load_transient_tables(self.profile).n, 128)
        base = load_long_psy_tables(self.profile)
        self.assertEqual(base.n, 1024)
        self.assertEqual(load_long_variant(2, self.profile, base).n, 1024)
        self.assertEqual(load_long_variant(3, self.profile, base).n, 1024)
        self.assertEqual(len(load_short_psy_profiles(self.profile)), 2)

    def test_static_mdct_trig_banks(self):
        looks = load_mdct_looks(self.profile)
        self.assertEqual(looks[128].n, 128)
        self.assertEqual(looks[1024].n, 1024)


if __name__ == "__main__":
    unittest.main()
