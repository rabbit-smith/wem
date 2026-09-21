"""Contract: the vendored geometry materializer reproduces every registered
psychoacoustic surface byte-for-byte (the round pipeline lock).

The materializer is the per-instruction port of the paired build's init-time
geometry builder (analysis_geometry_builder on the paired build); this test pins the
promotion contract in the repo: if the profile data or the builder port ever
drifts apart, this suite fails loudly. The kernel-side lock (Rust == this
builder) lives in crates/wem-analysis/tests/psy_geom*_parity.rs.

The 2ch/48k SHORT surfaces are locked here as well. They were promoted from
provisional operating points once the geometry they depend on was read out of
the running paired build rather than carried from 6ch: the build materializes
``first_octave = -34`` for the (2,45000,50000) descriptor (against -42 for
44.1k), while ``total_octave_lines`` stays rate-invariant at 585. That keeps
``octave_max - first_octave = 583 < 585`` for both rates, which is the
invariant the seed floor walk requires.

Both per-profile curve sets are covered for both rates. Profile 0 reads the
``mask_pool_0`` knot bank and profile 1 reads ``mask_pool_1``; both apply the
same default quality index and a zero bias, and the two banks are byte-equal
in the paired build's two descriptor copies, so the rate enters only through
the curve lerp. Until this lock the profile-1 rows were the one SHORT surface
with no mechanism path.
"""

from __future__ import annotations

import json
import struct
import unittest
from pathlib import Path
from typing import Any, ClassVar

from wwise_wem import WwiseProfile, WwiseVersion
from wwise_wem.profiles.registry import resolve_selection
from wwise_wem_reference.geometry_materializer import (
    inputs,
    long_base,
    long_variants,
    short_seed,
)

#: The packaged profile directories are the names their selections resolve to;
#: the contract reads them off the registry rather than re-typing them.
PROFILE_ROOT = (
    Path(__file__).resolve().parents[2] / "src" / "wwise_wem" / "data" / "profiles"
)
SIX_CHANNEL_NAME = resolve_selection(
    WwiseProfile(WwiseVersion.WWISE2013, 6, 44100)
).name
TWO_CHANNEL_NAME = resolve_selection(
    WwiseProfile(WwiseVersion.WWISE2013, 2, 48000)
).name

PROFILES = PROFILE_ROOT / SIX_CHANNEL_NAME / "psychoacoustics"
KEY = (6, 40000, 70000)  # the 6ch descriptor family (paired build geometry)

PROFILES_2CH = PROFILE_ROOT / TWO_CHANNEL_NAME / "psychoacoustics"
KEY_2CH = (2, 45000, 50000)  # the 2ch/48k descriptor family


def f32_bits(x: float) -> int:
    return struct.unpack("<I", struct.pack("<f", float(x)))[0]


def u32(x: int) -> int:
    return int(x) & 0xFFFFFFFF


class GeometryMaterializerContract(unittest.TestCase):
    short_seed_reg: ClassVar[dict[str, Any]]
    short_profiles_reg: ClassVar[dict[str, Any]]
    long_base_reg: ClassVar[dict[str, Any]]
    long_modes_reg: ClassVar[dict[str, Any]]
    seed_2ch: ClassVar[dict[str, Any]]
    profiles_2ch: ClassVar[dict[str, Any]]
    long_base_2ch: ClassVar[dict[str, Any]]
    long_modes_2ch: ClassVar[dict[str, Any]]
    family: ClassVar[Any]

    @classmethod
    def setUpClass(cls) -> None:
        cls.short_seed_reg = json.loads((PROFILES / "short-seed.json").read_text())
        cls.short_profiles_reg = json.loads((PROFILES / "short-profiles.json").read_text())
        cls.long_base_reg = json.loads((PROFILES / "long-base.json").read_text())
        cls.long_modes_reg = json.loads((PROFILES / "long-modes.json").read_text())
        cls.seed_2ch = json.loads((PROFILES_2CH / "short-seed.json").read_text())
        cls.profiles_2ch = json.loads((PROFILES_2CH / "short-profiles.json").read_text())
        cls.long_base_2ch = json.loads((PROFILES_2CH / "long-base.json").read_text())
        cls.long_modes_2ch = json.loads((PROFILES_2CH / "long-modes.json").read_text())
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

    def test_short_seed_surfaces_2ch(self) -> None:
        """Lock the promoted 2ch/48k SHORT surfaces against the builder."""
        out = short_seed.build(self.family, KEY_2CH, KEY_2CH)
        self.assertEqual(out["ath"], [f32_bits(v) for v in self.seed_2ch["ath"]])
        self.assertEqual(out["octave"], [u32(v) for v in self.seed_2ch["octave"]])
        self.assertEqual(
            out["look.interval_table"],
            [u32(v) for v in self.seed_2ch["look"]["interval_table"]],
        )
        self.assertEqual(
            out["look.mask_curve"],
            [f32_bits(v) for v in self.seed_2ch["look"]["mask_curve"]],
        )
        self.assertEqual(
            out["mask_curves"],
            [f32_bits(v) for row in self.profiles_2ch["profiles"][0]["mask_curves"] for v in row],
        )

    def test_short_profile_curves_are_mechanism_derived(self) -> None:
        """Both per-profile curve sets, at both rates, from their own knot bank."""
        for key, registered in (
            (KEY, self.short_profiles_reg),
            (KEY_2CH, self.profiles_2ch),
        ):
            for index in (0, 1):
                with self.subTest(key=key, profile=index):
                    self.assertEqual(
                        short_seed.profile_mask_curves(self.family, key, key, index),
                        [
                            f32_bits(v)
                            for row in registered["profiles"][index]["mask_curves"]
                            for v in row
                        ],
                    )

    def test_2ch_octave_geometry_satisfies_the_seed_walk_invariant(self) -> None:
        """The seed floor walk indexes ``seed[pos]`` without a bound check.

        ``seed`` is allocated with ``total_octave_lines`` entries, so the octave
        surface must satisfy ``max(end) < total_octave_lines`` where
        ``end = mean(octave[i], octave[i+1]) - first_octave``. The paired build
        keeps that margin at 2 for both rates by shifting ``first_octave`` with
        the sample rate; a carried-over ``first_octave`` breaks it.
        """
        geom = self.seed_2ch["geometry"]
        octave = [int(v) for v in self.seed_2ch["octave"]]
        worst = max(
            ((octave[i] + octave[i + 1]) >> 1) - int(geom["first_octave"])
            for i in range(len(octave) - 1)
        )
        self.assertLess(worst, int(geom["total_octave_lines"]))

    def test_2ch_rate_specific_aotuv_bounds(self) -> None:
        """The 48 kHz preset must not retain the 44.1 kHz bin bounds."""
        self.assertEqual(self.seed_2ch["look"]["short_limit"], 70)
        self.assertTrue(
            all(profile["peak_cutoff"] == 83 for profile in self.profiles_2ch["profiles"])
        )
        self.assertTrue(
            all(profile["band_limits"] == [8, 6, 3] for profile in self.profiles_2ch["profiles"])
        )
        self.assertTrue(
            all(
                f32_bits(profile["side_gain"]) == 0x3F9A3D71
                for profile in self.profiles_2ch["profiles"]
            )
        )
        self.assertTrue(
            all(
                f32_bits(profile["candidate_bias_by_mode"][1]) == 0x41100000
                for profile in self.profiles_2ch["profiles"]
            )
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

    def test_long_surfaces_2ch(self) -> None:
        """Lock the promoted 2ch/48k LONG surfaces against the builder.

        These rows were read out of the running paired build at 48 kHz: five full
        4096-byte surfaces came back byte-identical to this builder's output,
        while the previously registered resampled values were absent from the
        process. The mechanism's inputs are shared with 6ch (the LONG knot banks
        sit at the same pointer-block indices in both descriptor copies), so the
        rate is the only parameter that changes.
        """
        out = long_base.build_mode(self.family, KEY_2CH, KEY_2CH, 2)
        for i in range(3):
            self.assertEqual(
                out[f"analysis.curves[{i}]"],
                [f32_bits(v) for v in self.long_base_2ch["analysis"]["curves"][i]],
            )
        self.assertEqual(
            out["analysis.field_19_curve"],
            [f32_bits(v) for v in self.long_base_2ch["analysis"]["field_19_curve"]],
        )
        self.assertEqual(
            out["analysis.interval_u32"],
            [u32(v) for v in self.long_base_2ch["analysis"]["interval_u32"]],
        )
        variants = long_variants.build(self.family, KEY_2CH, KEY_2CH)
        for mode in (2, 3):
            entry = self.long_modes_2ch["variants"][str(mode)]
            self.assertEqual(
                variants[f"variants[{mode}].curves"],
                [f32_bits(x) for row in entry["curves"] for x in row],
            )
            self.assertEqual(
                variants[f"variants[{mode}].field_19_curve"],
                [f32_bits(x) for x in entry["field_19_curve"]],
            )
            self.assertEqual(
                variants[f"variants[{mode}].interval_u32"],
                [u32(x) for x in entry["interval_u32"]],
            )
        self.assertEqual(
            variants["seed.base_curve"],
            [f32_bits(v) for v in self.long_base_2ch["seed"]["base_curve"]],
        )
        self.assertEqual(
            variants["seed.group_labels_u32"],
            [u32(v) for v in self.long_base_2ch["seed"]["group_labels_u32"]],
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
