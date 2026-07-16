"""Boundary regressions for resource-free setup packet syntax."""

from __future__ import annotations

import json
import os
import subprocess
import sys
import unittest
from pathlib import Path

from wwise2013_wem.vorbis.setup import pack_setup, parse_setup


ROOT = Path(__file__).resolve().parents[2]
PACKAGE = ROOT / "src" / "wwise2013_wem"


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

package = types.ModuleType("wwise2013_wem")
package.__path__ = [os.environ["WWISE_PACKAGE_DIR"]]
package.__package__ = "wwise2013_wem"
sys.modules["wwise2013_wem"] = package
setup = importlib.import_module("wwise2013_wem.vorbis.setup")
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
    "wwise2013_wem.profiles.book_ids",
    "wwise2013_wem.profiles.resources",
    "wwise2013_wem.profiles.codebooks",
) if name in sys.modules]
print(json.dumps({
    "banned": banned,
    "complete": parsed["parse_complete"],
}))
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
        self.assertTrue(result["complete"])
        self.assertEqual(result["banned"], [])


if __name__ == "__main__":
    unittest.main()
