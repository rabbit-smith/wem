from __future__ import annotations

import unittest

from wwise_wem_reference.profiles.book_ids import resolve_book_id
from wwise_wem_reference.vorbis.setup import ilog, pack_setup, parse_setup
from wwise_wem_reference.profiles.transient import load_transient_tables
from wwise_wem_reference.profiles.psychoacoustics.long_tables import load_long_psy_tables
from wwise_wem_reference.profiles.psychoacoustics.long_variants import load_long_variant
from wwise_wem_reference.profiles.psychoacoustics.short_tables import load_short_psy_profiles
from wwise_wem import WwiseProfile, WwiseVersion
from wwise_wem.profiles.key import ProfileKey
from wwise_wem_reference.profiles.artifact import resolve_selection
from tests.analysis_resource_support import installed_profile
from tests.codebook_resource_support import installed_codebook_tables


SIX_CHANNEL_SELECTION = WwiseProfile(WwiseVersion.WWISE2013, 6, 44100)
SIX_CHANNEL_PROFILE = resolve_selection(SIX_CHANNEL_SELECTION)


class SetupRuntimeTests(unittest.TestCase):
    def test_profile_setup_parse_pack_roundtrip(self):
        packet = SIX_CHANNEL_PROFILE.setup_packet
        setup = parse_setup(packet, channels=6)
        self.assertTrue(setup["parse_complete"])
        self.assertEqual(
            (
                setup["nbooks"],
                setup["nfloors"],
                setup["nresidues"],
                setup["nmaps"],
                setup["nmodes"],
            ),
            (39, 2, 2, 2, 2),
        )
        self.assertEqual(pack_setup(setup), packet)

        resolved = [
            resolve_book_id(book_id, installed_codebook_tables())
            for book_id in setup["book_ids"]
        ]
        self.assertEqual(len(resolved), setup["nbooks"])

    def test_setup_truncation_and_ilog_edges(self):
        self.assertEqual([ilog(value) for value in (0, 1, 2, 3, 4, 255)], [0, 1, 2, 2, 3, 8])
        packet = SIX_CHANNEL_PROFILE.setup_packet
        for truncated in (b"", packet[:1], packet[:-1]):
            with self.subTest(length=len(truncated)):
                with self.assertRaisesRegex(EOFError, "out of bits"):
                    parse_setup(truncated, channels=6)

    def test_table_readers_validate_installed_geometry(self):
        profile = installed_profile(6, 44100)
        long_table = load_long_psy_tables(profile)
        self.assertEqual(
            (long_table.n, long_table.sample_rate, len(long_table.analysis_curves)),
            (1024, 44100, 3),
        )
        transient = load_transient_tables(profile)
        self.assertEqual((transient.n, len(transient.window), len(transient.config), len(transient.bands)), (128, 128, 26, 12))
        short = load_short_psy_profiles(profile)
        self.assertEqual(tuple(short_profile.key for short_profile in short), ("short_look_0", "short_look_1"))
        self.assertEqual(
            (
                load_long_variant(2, profile, long_table).profile_key,
                load_long_variant(3, profile, long_table).profile_key,
            ),
            ("1024:2", "1024:3"),
        )

    def test_long_variant_mode_errors(self):
        profile = installed_profile(6, 44100)
        base = load_long_psy_tables(profile)
        with self.assertRaisesRegex(ValueError, "mode must be 2 or 3"):
            load_long_variant(1, profile, base)

    def test_profile_resolution_and_setup_identity(self):
        self.assertEqual(
            resolve_selection(SIX_CHANNEL_SELECTION), SIX_CHANNEL_PROFILE
        )
        with self.assertRaisesRegex(ValueError, "no installed Wwise 2013 profile"):
            resolve_selection(WwiseProfile(WwiseVersion.WWISE2013, 2, 44100))

        # The identity is the key the carrier carries — generation, geometry
        # and layout — never a value derived from the setup packet, whose
        # bytes the resolution hands back directly.
        self.assertEqual(
            SIX_CHANNEL_PROFILE.key,
            ProfileKey(6, 44100, "2013.2", "5.1"),
        )


if __name__ == "__main__":
    unittest.main()
