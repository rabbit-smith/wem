"""The vendored geometry materializer reproduces every registered
psychoacoustic surface byte-for-byte (pipeline lock).

The materializer is the per-instruction port of the paired build's init-time
geometry builder of the paired build; this test checks that
the profile data and the builder port have not drifted apart, and fails loudly
when they have. The kernel-side check (Rust == this builder) lives in
crates/wem-analysis/tests/psy_geom*_parity.rs.

Every "registered" value below is read from the compiled carrier
(``wwise_wem._core.profile_tables()``), which is where the profile values live
now: the recorded documents this comparison used to read carried the same
numbers, and the carrier is what the kernel actually encodes with.

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

import struct
import unittest
from typing import Any, ClassVar

from wwise_wem import WwiseProfile, WwiseVersion
from wwise_wem_reference.geometry_materializer import (
    inputs,
    long_base,
    long_variants,
    short_seed,
)
from wwise_wem_reference.profiles.artifact import CompiledProfile, resolve_selection

#: The compiled profiles: every registered value below is read off the carrier
#: its selection resolves to, never re-typed here.
SIX_CHANNEL = resolve_selection(WwiseProfile(WwiseVersion.WWISE2013, 6, 44100))
TWO_CHANNEL = resolve_selection(WwiseProfile(WwiseVersion.WWISE2013, 2, 48000))

KEY = (6, 40000, 70000)  # the 6ch descriptor family (paired build geometry)
KEY_2CH = (2, 45000, 50000)  # the 2ch/48k descriptor family


def f32_bits(x: float) -> int:
    return struct.unpack("<I", struct.pack("<f", float(x)))[0]


def u32(x: int) -> int:
    return int(x) & 0xFFFFFFFF


def _bits(values: Any) -> list[int]:
    return [f32_bits(v) for v in values]


def _words(values: Any) -> list[int]:
    return [u32(v) for v in values]


def _rows(values: Any, rows: int, width: int) -> list[list[int]]:
    """One flattened carrier block as its f32 bit-pattern rows."""
    flat = _bits(values)
    return [flat[row * width : (row + 1) * width] for row in range(rows)]


def _short_seed(profile: CompiledProfile) -> dict:
    """The short seed surface, in the shape the materializer's output uses."""

    def block(name: str) -> Any:
        return profile.table(f"short_seed.{name}")

    return {
        "ath": block("ath"),
        "octave": block("octave"),
        "look": {
            "interval_table": block("interval_table"),
            "mask_curve": block("mask_curve"),
            "short_limit": int(block("short_limit")[0]),
        },
        "geometry": {
            "first_octave": int(block("first_octave")[0]),
            "total_octave_lines": int(block("total_octave_lines")[0]),
        },
    }


def _short_profile(profile: CompiledProfile, index: int) -> dict:
    prefix = f"short_profiles.{index}"
    return {
        "mask_curves": _rows(profile.table(f"{prefix}.mask_curves"), 3, 128),
        "peak_cutoff": int(profile.table(f"{prefix}.peak_cutoff")[0]),
        "band_limits": [int(v) for v in profile.table(f"{prefix}.band_limits")],
        "side_gain": profile.table(f"{prefix}.side_gain")[0],
        "candidate_bias_by_mode": profile.table(f"{prefix}.candidate_bias_by_mode"),
    }


def _long_base(profile: CompiledProfile) -> dict:
    def block(name: str) -> Any:
        return profile.table(f"long_base.{name}")

    return {
        "analysis": {
            "curves": _rows(block("analysis_curves"), 3, 1024),
            "field_19_curve": _bits(block("analysis_field_19_curve")),
            "interval_u32": _words(block("analysis_interval_u32")),
        },
        "seed": {
            "base_curve": _bits(block("seed_base_curve")),
            "group_labels_u32": _words(block("seed_group_labels_u32")),
        },
    }


def _variant(profile: CompiledProfile, mode: int) -> dict:
    prefix = f"long_variants.{mode}"
    return {
        "curves": _rows(profile.table(f"{prefix}.analysis_curves"), 3, 1024),
        "field_19_curve": _bits(profile.table(f"{prefix}.analysis_field_19_curve")),
        "interval_u32": _words(profile.table(f"{prefix}.analysis_interval_u32")),
    }


class GeometryMaterializerParityTests(unittest.TestCase):
    family: ClassVar[Any]

    @classmethod
    def setUpClass(cls) -> None:
        cls.family = inputs.load()

    def test_short_seed_surfaces(self) -> None:
        registered = _short_seed(SIX_CHANNEL)
        profiles = _short_profile(SIX_CHANNEL, 0)
        out = short_seed.build(self.family, KEY, KEY)
        self.assertEqual(out["ath"], _bits(registered["ath"]))
        self.assertEqual(out["octave"], _words(registered["octave"]))
        self.assertEqual(
            out["look.interval_table"],
            _words(registered["look"]["interval_table"]),
        )
        self.assertEqual(
            out["look.mask_curve"],
            _bits(registered["look"]["mask_curve"]),
        )
        self.assertEqual(
            out["mask_curves"],
            [value for row in profiles["mask_curves"] for value in row],
        )

    def test_short_seed_surfaces_2ch(self) -> None:
        """Lock the promoted 2ch/48k SHORT surfaces against the builder."""
        registered = _short_seed(TWO_CHANNEL)
        profiles = _short_profile(TWO_CHANNEL, 0)
        out = short_seed.build(self.family, KEY_2CH, KEY_2CH)
        self.assertEqual(out["ath"], _bits(registered["ath"]))
        self.assertEqual(out["octave"], _words(registered["octave"]))
        self.assertEqual(
            out["look.interval_table"],
            _words(registered["look"]["interval_table"]),
        )
        self.assertEqual(
            out["look.mask_curve"],
            _bits(registered["look"]["mask_curve"]),
        )
        self.assertEqual(
            out["mask_curves"],
            [value for row in profiles["mask_curves"] for value in row],
        )

    def test_short_profile_curves_are_mechanism_derived(self) -> None:
        """Both per-profile curve sets, at both rates, from their own knot bank."""
        for key, profile in ((KEY, SIX_CHANNEL), (KEY_2CH, TWO_CHANNEL)):
            for index in (0, 1):
                with self.subTest(key=key, profile=index):
                    registered = _short_profile(profile, index)
                    self.assertEqual(
                        short_seed.profile_mask_curves(self.family, key, key, index),
                        [
                            value
                            for row in registered["mask_curves"]
                            for value in row
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
        registered = _short_seed(TWO_CHANNEL)
        geom = registered["geometry"]
        octave = [int(v) for v in registered["octave"]]
        worst = max(
            ((octave[i] + octave[i + 1]) >> 1) - int(geom["first_octave"])
            for i in range(len(octave) - 1)
        )
        self.assertLess(worst, int(geom["total_octave_lines"]))

    def test_2ch_rate_specific_aotuv_bounds(self) -> None:
        """The 48 kHz preset must not retain the 44.1 kHz bin bounds."""
        registered = _short_seed(TWO_CHANNEL)
        self.assertEqual(registered["look"]["short_limit"], 70)
        for index in (0, 1):
            profile = _short_profile(TWO_CHANNEL, index)
            with self.subTest(profile=index):
                self.assertEqual(profile["peak_cutoff"], 83)
                self.assertEqual(profile["band_limits"], [8, 6, 3])
                self.assertEqual(f32_bits(profile["side_gain"]), 0x3F9A3D71)
                self.assertEqual(
                    f32_bits(profile["candidate_bias_by_mode"][1]), 0x41100000
                )

    def test_long_base_surfaces(self) -> None:
        registered = _long_base(SIX_CHANNEL)
        out = long_base.build(self.family, KEY, KEY)
        for i in range(3):
            self.assertEqual(
                out[f"analysis.curves[{i}]"], registered["analysis"]["curves"][i]
            )
        self.assertEqual(
            out["analysis.field_19_curve"],
            registered["analysis"]["field_19_curve"],
        )
        self.assertEqual(
            out["analysis.interval_u32"],
            registered["analysis"]["interval_u32"],
        )

    def test_long_variants_and_seed_surfaces(self) -> None:
        registered = _long_base(SIX_CHANNEL)
        out = long_variants.build(self.family, KEY, KEY)
        for mode in (2, 3):
            variant = _variant(SIX_CHANNEL, mode)
            self.assertEqual(
                out[f"variants[{mode}].curves"],
                [value for row in variant["curves"] for value in row],
            )
            self.assertEqual(
                out[f"variants[{mode}].field_19_curve"], variant["field_19_curve"]
            )
            self.assertEqual(
                out[f"variants[{mode}].interval_u32"], variant["interval_u32"]
            )
        self.assertEqual(out["seed.base_curve"], registered["seed"]["base_curve"])
        self.assertEqual(
            out["seed.group_labels_u32"], registered["seed"]["group_labels_u32"]
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
        registered = _long_base(TWO_CHANNEL)
        out = long_base.build_mode(self.family, KEY_2CH, KEY_2CH, 2)
        for i in range(3):
            self.assertEqual(
                out[f"analysis.curves[{i}]"], registered["analysis"]["curves"][i]
            )
        self.assertEqual(
            out["analysis.field_19_curve"],
            registered["analysis"]["field_19_curve"],
        )
        self.assertEqual(
            out["analysis.interval_u32"],
            registered["analysis"]["interval_u32"],
        )
        variants = long_variants.build(self.family, KEY_2CH, KEY_2CH)
        for mode in (2, 3):
            variant = _variant(TWO_CHANNEL, mode)
            self.assertEqual(
                variants[f"variants[{mode}].curves"],
                [value for row in variant["curves"] for value in row],
            )
            self.assertEqual(
                variants[f"variants[{mode}].field_19_curve"],
                variant["field_19_curve"],
            )
            self.assertEqual(
                variants[f"variants[{mode}].interval_u32"], variant["interval_u32"]
            )
        self.assertEqual(
            variants["seed.base_curve"], registered["seed"]["base_curve"]
        )
        self.assertEqual(
            variants["seed.group_labels_u32"], registered["seed"]["group_labels_u32"]
        )

    def test_mask_curve_alias_identity(self) -> None:
        # root-verified identity: short-seed look.mask_curve == mask_curves row 1
        registered = _short_seed(SIX_CHANNEL)
        rows = _short_profile(SIX_CHANNEL, 0)["mask_curves"]
        self.assertEqual(_bits(registered["look"]["mask_curve"]), rows[1])


if __name__ == "__main__":
    unittest.main()
