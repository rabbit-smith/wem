from __future__ import annotations

import tempfile
import unittest
from dataclasses import replace
from pathlib import Path
from unittest.mock import patch

from wwise_wem.profiles.book_ids import resolve_book_id
from wwise_wem.vorbis.setup import ilog, pack_setup, parse_setup
from wwise_wem.profiles.transient import load_transient_tables
from wwise_wem.profiles.psychoacoustics.long_tables import (
    load_long_psy_tables,
    table_sha256,
)
from wwise_wem.profiles.psychoacoustics.long_variants import load_long_variant
from wwise_wem.profiles.psychoacoustics.short_tables import load_short_psy_profiles
from wwise_wem.profiles.registry import (
    WWISE2013_6CH_44100,
    load_wem_profile,
    resolve_wem_profile,
)
from wwise_wem.profiles.bundle import load_profile_bundle
from tests.codebook_resource_support import installed_codebook_tables


class SetupRuntimeTests(unittest.TestCase):
    def test_profile_setup_parse_pack_roundtrip(self):
        packet = WWISE2013_6CH_44100.setup_packet()
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
        packet = WWISE2013_6CH_44100.setup_packet()
        for truncated in (b"", packet[:1], packet[:-1]):
            with self.subTest(length=len(truncated)):
                with self.assertRaisesRegex(EOFError, "out of bits"):
                    parse_setup(truncated, channels=6)

    def test_table_loaders_validate_installed_geometry(self):
        manifest = load_profile_bundle(verify_all=False).runtime_manifest
        long_ref = manifest.resource("psychoacoustics.long-base")
        modes_ref = manifest.resource("psychoacoustics.long-modes")
        long_table = load_long_psy_tables(long_ref)
        self.assertEqual(
            (long_table.n, long_table.sample_rate, len(long_table.analysis_curves)),
            (1024, 44100, 3),
        )
        self.assertEqual(len(table_sha256(long_ref)), 64)
        transient = load_transient_tables(manifest.resource("analysis.transient"))
        self.assertEqual((transient.n, len(transient.window), len(transient.config), len(transient.bands)), (128, 128, 26, 12))
        short = load_short_psy_profiles(
            manifest.resource("psychoacoustics.short-profiles")
        )
        self.assertEqual(tuple(profile.key for profile in short), ("short_look_0", "short_look_1"))
        self.assertEqual(
            (
                load_long_variant(2, modes_ref, long_table).profile_key,
                load_long_variant(3, modes_ref, long_table).profile_key,
            ),
            ("1024:2", "1024:3"),
        )

    def test_long_table_schema_and_variant_mode_errors(self):
        manifest = load_profile_bundle(verify_all=False).runtime_manifest
        long_ref = manifest.resource("psychoacoustics.long-base")
        payload = long_ref.read_json()
        payload["schema"] = "wrong"
        with patch.object(type(long_ref), "read_json", return_value=payload):
            with self.assertRaisesRegex(ValueError, "unexpected.*schema"):
                load_long_psy_tables(long_ref)
        base = load_long_psy_tables(long_ref)
        with self.assertRaisesRegex(ValueError, "mode must be 2 or 3"):
            load_long_variant(
                1,
                manifest.resource("psychoacoustics.long-modes"),
                base,
            )

    def test_profile_resolution_and_checksum_errors(self):
        self.assertIs(load_wem_profile(WWISE2013_6CH_44100.name), WWISE2013_6CH_44100)
        self.assertIs(resolve_wem_profile(6, 44100), WWISE2013_6CH_44100)
        with self.assertRaisesRegex(ValueError, "unknown WEM profile"):
            load_wem_profile("missing")
        with self.assertRaisesRegex(ValueError, "no Wwise 2013.2 profile"):
            resolve_wem_profile(2, 48000)

        with tempfile.TemporaryDirectory() as directory:
            path = Path(directory) / "setup.bin"
            path.write_bytes(b"different")
            changed = replace(WWISE2013_6CH_44100, setup_path=path)
            with self.assertRaisesRegex(ValueError, "setup checksum differs"):
                changed.setup_packet()


if __name__ == "__main__":
    unittest.main()
