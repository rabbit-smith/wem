import hashlib
import json
import unittest
from pathlib import Path

import wwise_wem

from wwise_wem import WwiseProfile, WwiseVersion
from wwise_wem.profiles.registry import resolve_selection
from wwise_wem_reference.profiles.transient import load_transient_tables
from wwise_wem_reference.profiles.psychoacoustics.long_tables import load_long_psy_tables
from wwise_wem_reference.profiles.psychoacoustics.long_variants import load_long_variant
from wwise_wem_reference.profiles.psychoacoustics.short_tables import load_short_psy_profiles
from wwise_wem.profiles.bundle import load_profile_bundle
from wwise_wem_reference.profiles.transform import load_mdct_looks


SIX_CHANNEL_SELECTION = WwiseProfile(WwiseVersion.WWISE2013, 6, 44100)


class PackagedTableTests(unittest.TestCase):
    def test_runtime_manifest_hashes(self):
        # The 6ch/44100 profile's directory is the packaged name its selection
        # resolves to, never a literal restated by the test.
        profile = (
            Path(wwise_wem.__file__).parent
            / "data" / "profiles" / resolve_selection(SIX_CHANNEL_SELECTION).name
        )
        manifest = json.loads((profile / "manifest.json").read_text())
        for logical_name, resource in manifest["resources"].items():
            self.assertEqual(
                hashlib.sha256((profile / resource["path"]).read_bytes()).hexdigest(),
                resource["sha256"],
                logical_name,
            )

    def test_analysis_tables_load_and_validate(self):
        manifest = load_profile_bundle(verify_all=False).runtime_manifest
        self.assertEqual(load_transient_tables(manifest.resource("analysis.transient")).n, 128)
        base = load_long_psy_tables(
            manifest.resource("psychoacoustics.long-base")
        )
        modes = manifest.resource("psychoacoustics.long-modes")
        self.assertEqual(base.n, 1024)
        self.assertEqual(load_long_variant(2, modes, base).n, 1024)
        self.assertEqual(load_long_variant(3, modes, base).n, 1024)
        self.assertEqual(
            len(
                load_short_psy_profiles(
                    manifest.resource("psychoacoustics.short-profiles")
                )
            ),
            2,
        )

    def test_static_mdct_trig_banks(self):
        manifest = load_profile_bundle(verify_all=False).runtime_manifest
        looks = load_mdct_looks(manifest.resource("transform.mdct"))
        self.assertEqual(looks[128].n, 128)
        self.assertEqual(looks[1024].n, 1024)


if __name__ == "__main__":
    unittest.main()
