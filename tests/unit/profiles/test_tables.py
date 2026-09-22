"""The compiled carrier's tables: identity, geometry and loader agreement."""

import unittest

import wwise_wem._core as _core
from wwise_wem import WwiseProfile, WwiseVersion

from tests.analysis_resource_support import installed_profile
from wwise_wem_reference.profiles.psychoacoustics.long_tables import load_long_psy_tables
from wwise_wem_reference.profiles.psychoacoustics.long_variants import load_long_variant
from wwise_wem_reference.profiles.psychoacoustics.short_tables import load_short_psy_profiles
from wwise_wem_reference.profiles.transform import load_mdct_looks
from wwise_wem_reference.profiles.transient import load_transient_tables

#: Enough frames for the kernel's stream session to emit its seq-0 packet.
MIN_FRAMES = 4096


class CarrierTableTests(unittest.TestCase):
    profile = installed_profile(6, 44100)

    def test_carried_setup_packet_is_the_kernel_packet(self):
        # The carrier used to record the packet's digest beside it and the
        # suite compared digests; the packet is compiled into the kernel, so
        # the comparison is made against the bytes themselves — the seq-0
        # packet the kernel emits for this profile's own selection.
        profile = self.profile
        selection = WwiseProfile(
            WwiseVersion.from_generation(profile.generation),
            profile.channels,
            profile.sample_rate,
        )
        packets = _core.StreamSession.for_selection(selection).push(
            bytes(MIN_FRAMES * profile.channels * 2)
        )
        self.assertTrue(packets, "the kernel emitted no seq-0 packet")
        self.assertEqual(bytes(profile.setup_packet), bytes(packets[0].data))

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
