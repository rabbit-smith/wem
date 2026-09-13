"""Contract: the vendored geometry materializer reproduces every registered
6ch psychoacoustic surface byte-for-byte (the round pipeline lock).

The materializer is the per-instruction port of the paired build's init-time
geometry builder (analysis_geometry_builder on the paired build); this test pins the
promotion contract in the repo: if the profile data or the builder port ever
drifts apart, this suite fails loudly. The kernel-side lock (Rust == this
builder) lives in crates/wem-analysis/tests/psy_geom*_parity.rs.

2ch registered values remain provisional operating points pending the round
re-registration with real-Windows revalidation (the round); they are deliberately
not asserted here, and nothing in this file may start asserting them as
authority before that gate.
"""

from __future__ import annotations

import json
import struct
import unittest
from pathlib import Path

from wwise_wem_reference.geometry_materializer import (
    inputs,
    long_base,
    long_variants,
    short_seed,
)

PROFILES = (
    Path(__file__).resolve().parents[2]
    / "src"
    / "wwise_wem"
    / "data"
    / "profiles"
    / "wwise2013-6ch-44100"
    / "psychoacoustics"
)
KEY = (6, 40000, 70000)  # the 6ch descriptor family (paired build geometry)


def f32_bits(x: float) -> int:
    return struct.unpack("<I", struct.pack("<f", float(x)))[0]


def u32(x: int) -> int:
    return int(x) & 0xFFFFFFFF


class GeometryMaterializerContract(unittest.TestCase):
    @classmethod
    def setUpClass(cls) -> None:
        cls.short_seed_reg = json.loads((PROFILES / "short-seed.json").read_text())
        cls.short_profiles_reg = json.loads((PROFILES / "short-profiles.json").read_text())
        cls.long_base_reg = json.loads((PROFILES / "long-base.json").read_text())
        cls.long_modes_reg = json.loads((PROFILES / "long-modes.json").read_text())
        cls.family = inputs.load()

    def _variants(self, mode: int) -> dict:
        vv = self.long_modes_reg["variants"]
        return vv[str(mode)] if isinstance(vv, dict) else vv[mode]

    def test_short_seed_surfaces(self) -> None:
        out = short_seed.build(self.family, KEY, KEY)
        self.assertEqual(out["ath"], [f32_bits(v) for v in self.short_seed_reg["ath"]])
        self.assertEqual(out["octave"], [u32(v) for v in self.short_seed_reg["octave"]])
        self.assertEqual(
            out["look.interval_table"],
            [u32(v) for v in self.short_seed_reg["look"]["interval_table"]],
        )
        self.assertEqual(
            out["look.mask_curve"],
            [f32_bits(v) for v in self.short_seed_reg["look"]["mask_curve"]],
        )
        self.assertEqual(
            out["mask_curves"],
            [
                f32_bits(v)
                for row in self.short_profiles_reg["profiles"][0]["mask_curves"]
                for v in row
            ],
        )

    def test_long_base_surfaces(self) -> None:
        out = long_base.build(self.family, KEY, KEY)
        for i in range(3):
            self.assertEqual(
                out[f"analysis.curves[{i}]"],
                [f32_bits(v) for v in self.long_base_reg["analysis"]["curves"][i]],
            )
        self.assertEqual(
            out["analysis.field_19_curve"],
            [f32_bits(v) for v in self.long_base_reg["analysis"]["field_19_curve"]],
        )
        self.assertEqual(
            out["analysis.interval_u32"],
            [u32(v) for v in self.long_base_reg["analysis"]["interval_u32"]],
        )

    def test_long_variants_and_seed_surfaces(self) -> None:
        out = long_variants.build(self.family, KEY, KEY)
        for mode in (2, 3):
            v = self._variants(mode)
            self.assertEqual(
                out[f"variants[{mode}].curves"],
                [f32_bits(x) for row in v["curves"] for x in row],
            )
            self.assertEqual(
                out[f"variants[{mode}].field_19_curve"],
                [f32_bits(x) for x in v["field_19_curve"]],
            )
            self.assertEqual(
                out[f"variants[{mode}].interval_u32"],
                [u32(x) for x in v["interval_u32"]],
            )
        self.assertEqual(
            out["seed.base_curve"], [f32_bits(v) for v in self.long_base_reg["seed"]["base_curve"]]
        )
        self.assertEqual(
            out["seed.group_labels_u32"],
            [u32(v) for v in self.long_base_reg["seed"]["group_labels_u32"]],
        )

    def test_mask_curve_alias_identity(self) -> None:
        # root-verified identity: short-seed look.mask_curve == mask_curves row 1
        rows = self.short_profiles_reg["profiles"][0]["mask_curves"]
        self.assertEqual(
            [f32_bits(v) for v in self.short_seed_reg["look"]["mask_curve"]],
            [f32_bits(v) for v in rows[1]],
        )


if __name__ == "__main__":
    unittest.main()
