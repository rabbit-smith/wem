"""The Vorbis setup record and the codebooks it declares.

The setup record is parsed from the profile's own packet and packed back; the
book ids it carries are what the codebook loader resolves against the
carrier. The codebook core cases pin the synthetic Huffman and lattice
arithmetic with no resources at all, and the behavior cases pin the same
objects over the installed rows. Each class keeps its own test names and
failure messages.
"""

from __future__ import annotations

import json
import os
import subprocess
import sys
import unittest
import wwise_wem_reference.vorbis.codebook as core

from pathlib import Path
from tests.analysis_resource_support import installed_profile
from tests.codebook_resource_support import installed_codebook_tables
from wwise_wem import WwiseProfile, WwiseVersion
from wwise_wem.profiles.key import ProfileKey
from wwise_wem_reference.profiles.artifact import resolve_selection
from wwise_wem_reference.profiles.book_ids import resolve_book_id
from wwise_wem_reference.profiles.codebooks import load_setup_codebooks
from wwise_wem_reference.profiles.psychoacoustics.long_tables import (
    load_long_psy_tables,
)
from wwise_wem_reference.profiles.psychoacoustics.long_variants import load_long_variant
from wwise_wem_reference.profiles.psychoacoustics.short_tables import (
    load_short_psy_profiles,
)
from wwise_wem_reference.profiles.transient import load_transient_tables
from wwise_wem_reference.vorbis.bitio import BitReader, OggPack
from wwise_wem_reference.vorbis.codebook import StaticCodebook, make_codewords
from wwise_wem_reference.vorbis.setup import ilog, pack_setup, parse_setup

# --------------------------------------------------------------------------
# merged from tests/unit/vorbis/test_setup_runtime.py
# --------------------------------------------------------------------------

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


# --------------------------------------------------------------------------
# merged from tests/unit/vorbis/test_codebook_core.py
# --------------------------------------------------------------------------

# Pure Vorbis codebook core and legacy façade regressions.

ROOT = Path(__file__).resolve().parents[3]
PACKAGE = ROOT / "reference" / "wwise_wem_reference"


class CodebookCoreTests(unittest.TestCase):
    def test_synthetic_huffman_roundtrip_is_resource_free(self):
        static = StaticCodebook(dim=1, entries=2, lengthlist=[1, 1])
        book = core.codebook_from_static(static)
        packed = OggPack()
        book.encode(packed, 1)
        book.encode(packed, 0)
        reader = BitReader(packed.get_buffer())
        self.assertEqual((book.decode(reader), book.decode(reader)), (1, 0))
        self.assertEqual(core.make_codewords([1, 1]), [0, 1])

    def test_synthetic_maptype1_lattice_preserves_values(self):
        static = StaticCodebook(
            dim=4,
            entries=81,
            lengthlist=[7] * 81,
            maptype=1,
            q_min=-535822336,
            q_delta=1611661312,
            q_quant=2,
            q_sequencep=0,
            quantlist=[1, 0, 2],
        )
        book = core.codebook_from_static(static)
        self.assertEqual(book.quantvals, 3)
        self.assertEqual(book.vq_values(0), [0.0] * 4)
        self.assertEqual(book.vq_values(1), [-1.0, 0.0, 0.0, 0.0])
        self.assertEqual(book.best_vq([-1.0, 0.0, 0.0, 0.0]), 1)

    def test_static_pack_and_quantlist_behavior_stays_canonical(self):
        static = core.StaticCodebook(
            dim=1,
            entries=2,
            lengthlist=[1, 1],
            maptype=1,
            q_min=-535822336,
            q_delta=1611661312,
            q_quant=2,
            q_sequencep=0,
            quantlist=[1, -2],
        )
        packed = static.pack()
        self.assertEqual(packed.hex(), "21000406008000070080000b09")
        self.assertEqual(core.ilog(0), 0)
        self.assertEqual(core.ilog(255), 8)

    def test_fresh_core_import_does_not_load_resource_adapters(self):
        code = r'''
import importlib
import json
import os
import sys
import types

package = types.ModuleType("wwise_wem_reference")
package.__path__ = [os.environ["WWISE_PACKAGE_DIR"]]
package.__package__ = "wwise_wem_reference"
sys.modules["wwise_wem_reference"] = package
core = importlib.import_module("wwise_wem_reference.vorbis.codebook")
result = core.make_codewords([1, 1])
banned = [name for name in (
    "wwise_wem_reference.profiles.book_ids",
    "wwise_wem_reference.profiles.artifact",
    "wwise_wem_reference.profiles.codebooks",
) if name in sys.modules]
print(json.dumps({"result": result, "banned": banned}))
'''
        env = dict(os.environ)
        env["WWISE_PACKAGE_DIR"] = str(PACKAGE)
        completed = subprocess.run(
            [sys.executable, "-c", code],
            cwd=ROOT,
            env=env,
            capture_output=True,
            text=True,
            check=True,
        )
        result = json.loads(completed.stdout)
        self.assertEqual(result, {"result": [0, 1], "banned": []})


# --------------------------------------------------------------------------
# merged from tests/unit/vorbis/test_codebook_behavior.py
# --------------------------------------------------------------------------

SIX_CHANNEL_SELECTION = WwiseProfile(WwiseVersion.WWISE2013, 6, 44100)


class CodebookBehaviorTests(unittest.TestCase):
    def test_oggpack_lsb_and_cross_word_vectors(self) -> None:
        packed = OggPack()
        packed.write(3, 2)
        packed.write(5, 3)
        self.assertEqual((packed.endbyte, packed.endbit), (0, 5))
        self.assertEqual(packed.get_buffer(), bytes([0x17]))

        word = OggPack()
        word.write(0xDEADBEEF, 32)
        self.assertEqual(word.get_buffer(), bytes.fromhex("efbeadde"))

        crossed = OggPack()
        crossed.write(1, 1)
        crossed.write(0xFFFFFFFF, 32)
        self.assertEqual(crossed.bytes_used(), 5)

        split = OggPack()
        split.write(0x15, 5)
        split.write(0x2A, 6)
        joined = OggPack()
        joined.write(0x15 | (0x2A << 5), 11)
        self.assertEqual(split.get_buffer(), joined.get_buffer())

    def test_static_codebook_unordered_pack(self) -> None:
        lengths = [0, 2, 3, 3, 3, 3, 4, 3, 4]
        codebook = StaticCodebook(1, 9, lengths)
        self.assertFalse(codebook.is_ordered())

        expected = OggPack()
        expected.write(1, 4)
        expected.write(9, 14)
        expected.write(0, 1)
        expected.write(3, 3)
        expected.write(1, 1)
        for length in lengths:
            expected.write(bool(length), 1)
            if length:
                expected.write(length - 1, 3)
        expected.write(0, 1)
        self.assertEqual(codebook.pack(), expected.get_buffer())

    def test_setup_books_encode_decode_and_vq(self) -> None:
        profile = resolve_selection(SIX_CHANNEL_SELECTION)
        setup = parse_setup(profile.setup_packet, channels=6)
        tables = installed_codebook_tables()
        books = load_setup_codebooks(setup["book_ids"], tables)
        self.assertEqual(len(books), setup["nbooks"])

        for codebook in books:
            entries = codebook.used_entries()[:3]
            self.assertTrue(entries)
            packed = OggPack()
            for entry in entries:
                codebook.encode(packed, entry)
            reader = BitReader(packed.get_buffer())
            self.assertEqual([codebook.decode(reader) for _ in entries], entries)
            if codebook.maptype == 1:
                self.assertGreater(codebook.quantvals, 0)
                self.assertEqual(len(codebook.vq_values(entries[0])), codebook.dim)

        lattice = load_setup_codebooks([214], installed_codebook_tables())[0]
        self.assertEqual((lattice.dim, lattice.entries, lattice.quantvals), (4, 81, 3))
        self.assertEqual(lattice.vq_values(0), [0.0] * 4)
        self.assertEqual(lattice.vq_values(1), [-1.0, 0.0, 0.0, 0.0])
        self.assertEqual(make_codewords([1, 1]), [0, 1])

    def test_published_tables_and_resolver_have_no_pointer_provenance(self) -> None:
        # The carrier holds decoded rows only: no recorded document and no
        # table block grows a pointer field back.
        profile = resolve_selection(SIX_CHANNEL_SELECTION)
        forbidden = {"ptr", "lengthlist_ptr", "quantlist_ptr"}
        names = [name for name in profile.tables if name.startswith("codebook.")]
        self.assertTrue(names)
        for name in names:
            self.assertTrue(
                forbidden.isdisjoint({part for part in name.split(".")}), name
            )
        rows = installed_codebook_tables()["t97"]
        self.assertTrue(rows)
        for row in rows:
            self.assertTrue(forbidden.isdisjoint(row))
        tables = installed_codebook_tables()
        self.assertTrue(forbidden.isdisjoint(resolve_book_id(0, tables)))
        self.assertTrue(forbidden.isdisjoint(resolve_book_id(214, tables)))

    def test_modules_have_no_hidden_diagnostic_or_cli_surface(self) -> None:
        from wwise_wem_reference.profiles import book_ids, codebooks
        from wwise_wem_reference.vorbis import bitio, codebook

        forbidden = {
            book_ids: ("main",),
            codebooks: ("self_test", "_entry_roundtrip", "_pick_test_entries"),
            bitio: ("test_basic",),
            codebook: ("test_book0",),
        }
        for module, names in forbidden.items():
            for name in names:
                self.assertFalse(hasattr(module, name), f"{module.__name__}.{name}")


if __name__ == "__main__":
    unittest.main()
