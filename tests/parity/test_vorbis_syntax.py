"""The Vorbis bit transport and the setup syntax written on it.

The bit I/O cases pin every supported width, cross-byte anchors, growth,
reset and end-of-buffer reads against a plain-integer reference packer. The
setup cases pin the packet's resource-free parse and pack round-trip and the
subprocess check that neither pulls a profile module in. The syntax is one
layer, so it is one module; each class keeps its own test names and failure
messages.
"""

from __future__ import annotations

import json
import os
import random
import subprocess
import sys
import unittest

from pathlib import Path
from wwise_wem_reference.vorbis.bitio import BitReader, OggPack
from wwise_wem_reference.vorbis.setup import pack_setup, parse_setup

# --------------------------------------------------------------------------
# merged from tests/parity/test_bitio.py
# --------------------------------------------------------------------------

def _reference_pack(writes: list[tuple[int, int]]) -> bytes:
    value = 0
    offset = 0
    for item, width in writes:
        mask = (1 << width) - 1 if width else 0
        value |= (item & mask) << offset
        offset += width
    return value.to_bytes((offset + 7) // 8, "little")


class BitIoTests(unittest.TestCase):
    def test_every_supported_width_roundtrips_and_masks(self) -> None:
        for width in range(33):
            with self.subTest(width=width):
                value = -(1 << 40) + 0xFEDCBA9876
                packed = OggPack()
                packed.write(value, width)
                expected = value & ((1 << width) - 1) if width else 0
                self.assertEqual(packed.get_buffer(), _reference_pack([(value, width)]))
                reader = BitReader(packed.get_buffer())
                self.assertEqual(reader.read(width), expected)
                self.assertEqual(reader.tell_bits(), width)
                self.assertEqual(reader.bits_left(), (-width) % 8)

        for width in (-1, 33):
            with self.subTest(invalid_width=width):
                with self.assertRaisesRegex(ValueError, "0..32"):
                    OggPack().write(0, width)

    def test_lsb_first_cross_byte_anchor_vector(self) -> None:
        writes = [
            (0b101, 3),
            (0b101010101, 9),
            (0xAB, 8),
            (0b11111, 5),
            (0xDEADBEEF, 32),
        ]
        packed = OggPack()
        for value, width in writes:
            packed.write(value, width)
        expected = _reference_pack(writes)
        self.assertEqual(packed.get_buffer(), expected)
        self.assertEqual(expected[:4], bytes.fromhex("adba fa df"))

        reader = BitReader(expected)
        self.assertEqual(
            [reader.read(width) for _, width in writes],
            [value & ((1 << width) - 1) for value, width in writes],
        )

    def test_growth_and_reset_reuse_storage_deterministically(self) -> None:
        packed = OggPack(initial=5)
        original_storage = packed.storage
        writes = [(index * 0x9E3779B1, 32) for index in range(80)]
        for value, width in writes:
            packed.write(value, width)
        self.assertGreater(packed.storage, original_storage)
        self.assertEqual(packed.bytes_used(), 80 * 4)
        self.assertEqual(packed.get_buffer(), _reference_pack(writes))

        grown_storage = packed.storage
        packed.reset()
        self.assertEqual((packed.endbyte, packed.endbit, packed.bytes_used()), (0, 0, 0))
        self.assertEqual(packed.get_buffer(), b"")
        self.assertEqual(packed.storage, grown_storage)
        packed.write(0b101, 3)
        self.assertEqual(packed.get_buffer(), b"\x05")

    def test_reader_position_padding_and_eof_behavior(self) -> None:
        reader = BitReader(bytes.fromhex("ad7e"))
        self.assertEqual((reader.tell_bits(), reader.bits_left()), (0, 16))
        self.assertEqual(reader.read(3), 0b101)
        self.assertEqual((reader.tell_bits(), reader.bits_left()), (3, 13))
        self.assertEqual(reader.read(5), 0b10101)
        self.assertEqual((reader.tell_bits(), reader.bits_left()), (8, 8))
        self.assertEqual(reader.read(8), 0x7E)
        self.assertEqual((reader.tell_bits(), reader.bits_left()), (16, 0))
        self.assertEqual(reader.read(0), 0)
        with self.assertRaisesRegex(EOFError, "out of bits"):
            reader.read(1)
        self.assertEqual((reader.tell_bits(), reader.bits_left()), (16, 0))

        partial = BitReader(b"\x80")
        partial.read(7)
        with self.assertRaises(EOFError):
            partial.read(2)
        self.assertEqual((partial.tell_bits(), partial.bits_left()), (8, 0))

    def test_seeded_write_read_stream_is_deterministic(self) -> None:
        rng = random.Random(0x2013C0DE)
        writes: list[tuple[int, int]] = []
        for _ in range(500):
            width = rng.randrange(33)
            value = rng.getrandbits(48)
            if rng.randrange(2):
                value = -value
            writes.append((value, width))

        packed = OggPack(initial=7)
        for value, width in writes:
            packed.write(value, width)
        expected_buffer = _reference_pack(writes)
        self.assertEqual(packed.get_buffer(), expected_buffer)

        reader = BitReader(expected_buffer)
        for value, width in writes:
            mask = (1 << width) - 1 if width else 0
            self.assertEqual(reader.read(width), value & mask)
        padding = reader.bits_left()
        self.assertEqual(padding, (-sum(width for _, width in writes)) % 8)
        self.assertEqual(reader.read(padding), 0)
        with self.assertRaises(EOFError):
            reader.read(1)


# --------------------------------------------------------------------------
# merged from tests/parity/test_setup_core_boundary.py
# --------------------------------------------------------------------------

# Boundary regressions for resource-free setup packet syntax.

ROOT = Path(__file__).resolve().parents[2]
FACADE = ROOT / "src" / "wwise_wem"
REFERENCE = ROOT / "reference" / "wwise_wem_reference"


def _minimal_setup() -> dict:
    return {
        "channels": 1,
        "nbooks": 1,
        "book_ids": [0],
        "nfloors": 1,
        "floors": [
            {
                "partitions": 0,
                "partition_classes": [],
                "max_class": -1,
                "class_dims": [],
                "class_subs": [],
                "class_masterbooks": [],
                "subclass_books": [],
                "multiplier": 1,
                "rangebits": 0,
                "x_list": [],
            }
        ],
        "nresidues": 1,
        "residues": [
            {
                "type": 1,
                "begin": 0,
                "end": 0,
                "partition_size": 1,
                "classifications": 1,
                "classbook": 0,
                "cascades": [0],
                "books": [[-1] * 8],
            }
        ],
        "nmaps": 1,
        "maps": [
            {
                "submaps": 1,
                "coupling": [],
                "reserved": 0,
                "chmux": [0],
                "floors": [0],
                "residues": [0],
            }
        ],
        "nmodes": 1,
        "modes": [{"blockflag": 0, "mapping": 0}],
    }


class SetupCoreBoundaryTests(unittest.TestCase):
    def test_parse_preserves_packet_and_schema(self):
        packet = pack_setup(_minimal_setup())
        parsed = parse_setup(packet, channels=1)
        self.assertEqual(pack_setup(parsed), packet)
        self.assertNotIn("books_resolved", parsed)
        self.assertEqual(parsed["book_ids"], [0])
        self.assertTrue(parsed["parse_complete"])

    def test_fresh_core_import_and_parse_do_not_load_resource_modules(self):
        # Install a namespace package stub so this probes parse_setup's own
        # dependency boundary rather than package-root profile exports.
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
setup = importlib.import_module("wwise_wem_reference.vorbis.setup")
info = {
    "channels": 1,
    "nbooks": 1, "book_ids": [0],
    "nfloors": 1,
    "floors": [{"partitions": 0, "partition_classes": [], "max_class": -1,
                "class_dims": [], "class_subs": [], "class_masterbooks": [],
                "subclass_books": [], "multiplier": 1, "rangebits": 0,
                "x_list": []}],
    "nresidues": 1,
    "residues": [{"type": 1, "begin": 0, "end": 0, "partition_size": 1,
                  "classifications": 1, "classbook": 0, "cascades": [0],
                  "books": [[-1] * 8]}],
    "nmaps": 1,
    "maps": [{"submaps": 1, "coupling": [], "reserved": 0, "chmux": [0],
              "floors": [0], "residues": [0]}],
    "nmodes": 1, "modes": [{"blockflag": 0, "mapping": 0}],
}
packet = setup.pack_setup(info)
parsed = setup.parse_setup(packet, channels=1)
banned = [name for name in (
    "wwise_wem_reference.profiles.book_ids",
    "wwise_wem_reference.profiles.artifact",
    "wwise_wem_reference.profiles.codebooks",
) if name in sys.modules]
print(json.dumps({
    "banned": banned,
    "complete": parsed["parse_complete"],
}))
'''
        env = dict(os.environ)
        env["WWISE_PACKAGE_DIR"] = str(REFERENCE)
        completed = subprocess.run(
            [sys.executable, "-c", code],
            cwd=ROOT,
            env=env,
            capture_output=True,
            text=True,
            check=True,
        )
        result = json.loads(completed.stdout)
        self.assertTrue(result["complete"])
        self.assertEqual(result["banned"], [])


if __name__ == "__main__":
    unittest.main()
