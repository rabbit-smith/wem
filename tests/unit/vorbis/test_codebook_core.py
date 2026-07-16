"""Pure Vorbis codebook core and legacy façade regressions."""

from __future__ import annotations

import json
import os
import subprocess
import sys
import unittest
from pathlib import Path

import wwise_wem.vorbis.codebook as core
from wwise_wem.vorbis.bitio import BitReader, OggPack
from wwise_wem.vorbis.codebook import StaticCodebook


ROOT = Path(__file__).resolve().parents[3]
PACKAGE = ROOT / "src" / "wwise_wem"


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

package = types.ModuleType("wwise_wem")
package.__path__ = [os.environ["WWISE_PACKAGE_DIR"]]
package.__package__ = "wwise_wem"
sys.modules["wwise_wem"] = package
core = importlib.import_module("wwise_wem.vorbis.codebook")
result = core.make_codewords([1, 1])
banned = [name for name in (
    "wwise_wem.profiles.book_ids",
    "wwise_wem.profiles.resources",
    "wwise_wem.profiles.codebooks",
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



if __name__ == "__main__":
    unittest.main()
