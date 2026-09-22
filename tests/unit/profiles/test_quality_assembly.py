"""End-to-end tests for the quality-override assembly wiring.

These tests prove the three wiring guarantees:

- ``quality`` omitted (``None``) leaves the assembled analysis resources
  byte-for-byte identical to the historical path (no crosstalk);
- a quality value requested from a profile that has no curves is a clear
  configuration error;
- a profile whose carrier records quality curves applies the interpolated
  overrides to the short psychoacoustic surface and records the quality value
  and the extrapolation honesty flag.

The "with curves" case builds a carrier with a synthetic quality-curves table
(the installed 6ch profile records none): the block shapes are the only thing
that changes, and the same profile -> loader -> assembly path runs without
touching the installed carrier's values.
"""
from __future__ import annotations

import unittest

from tests.analysis_resource_support import installed_profile
from wwise_wem import WwiseProfile, WwiseVersion
from wwise_wem_reference.profiles.artifact import resolve_selection
from wwise_wem_reference.profiles.assembly import (
    assemble_analysis_resources,
    assemble_encoder_profile_resources,
)
from wwise_wem_reference.profiles.quality import load_quality_curves


#: v2 shape: descriptor curve names with the semantics map routing each onto
#: its mechanism. The synthetic descriptors stand in for the paired build's
#: desc29/desc30; the overrides are rounded through the f32 boundary (the
#: short-surface storage type) on both implementations.
_SYNTHETIC_BREAKPOINTS = [0.0, 4.0, 8.0]
_SYNTHETIC_CURVES = {
    "desc29.psy_int1": [1.0, 2.0, 4.0],
    "desc30.psy_int2": [-10.0, -20.0, -40.0],
}
_SYNTHETIC_SEMANTICS = {
    "desc29.psy_int1": "short.ath_offset",
    "desc30.psy_int2": "short.ath_floor",
}


def _profile_with_curves():
    """The installed 6ch carrier plus a synthetic quality-curves table."""
    blocks: dict[str, object] = {
        "quality_curves.breakpoints": list(_SYNTHETIC_BREAKPOINTS),
    }
    blocks.update(
        {
            f"quality_curves.curve.{name}": list(values)
            for name, values in _SYNTHETIC_CURVES.items()
        }
    )
    blocks.update(
        {
            f"quality_curves.semantic.{name}": value
            for name, value in _SYNTHETIC_SEMANTICS.items()
        }
    )
    return installed_profile(6, 44100).with_tables(**blocks)


class QualityAssemblyWiringTests(unittest.TestCase):
    def test_quality_none_leaves_resources_unchanged(self):
        profile = installed_profile(6, 44100)
        resources_default = assemble_analysis_resources(profile)
        resources_explicit = assemble_analysis_resources(profile, quality=None)
        self.assertEqual(
            resources_default.short_surface.ath_offset,
            resources_explicit.short_surface.ath_offset,
        )
        self.assertEqual(
            resources_default.short_look.ath_offset,
            resources_explicit.short_look.ath_offset,
        )
        self.assertIsNone(resources_explicit.quality_value)
        self.assertFalse(resources_explicit.quality_extrapolated)

    def test_quality_without_curves_is_a_configuration_error(self):
        profile = installed_profile(6, 44100)
        self.assertIsNone(load_quality_curves(profile))
        with self.assertRaisesRegex(ValueError, "no quality-curves table"):
            assemble_analysis_resources(profile, quality=4.0)

    def test_quality_with_curves_applies_overrides_and_flags(self):
        profile = _profile_with_curves()
        # The synthetic carrier is not the installed one: the loader caches
        # are keyed on the identity *and* the table shapes.
        self.assertIsNot(profile, installed_profile(6, 44100))
        curves = load_quality_curves(profile)
        self.assertIsNotNone(curves)
        self.assertEqual(curves.breakpoints, tuple(_SYNTHETIC_BREAKPOINTS))
        self.assertEqual(set(curves.curves), set(_SYNTHETIC_CURVES))

        resources = assemble_analysis_resources(profile, quality=2.0)
        # The quality is normalized onto the breakpoint axis first
        # (qnorm = 2.0/10 + 1e-7 = 0.20000010000000001), then interpolated on
        # [0,4,8] and rounded through the f32 boundary (the short-surface
        # storage type, shared with Rust):
        #   ath_offset -> (1-f)*1 + f*2, f = 0.050000025 -> 1.0500000250000001 -> f32
        #   ath_floor  -> (1-f)*(-10) + f*(-20)          -> -10.500000250000001 -> f32
        self.assertEqual(resources.short_surface.ath_offset, 1.0500000715255737)
        self.assertEqual(resources.short_surface.ath_floor, -10.5)
        self.assertEqual(resources.quality_value, 2.0)
        self.assertFalse(resources.quality_extrapolated)
        # The rebuilt short look carries the overrides.
        self.assertEqual(resources.short_look.ath_offset, 1.0500000715255737)
        self.assertEqual(resources.short_look.ath_floor, -10.5)

        # Below-domain quality clamps to the first control point and flags
        # extrapolation (qnorm = -0.0999999 < 0).
        clamped = assemble_analysis_resources(profile, quality=-1.0)
        self.assertEqual(clamped.short_surface.ath_offset, 1.0)
        self.assertEqual(clamped.short_surface.ath_floor, -10.0)
        self.assertTrue(clamped.quality_extrapolated)
        self.assertEqual(clamped.quality_value, -1.0)

    def test_encoder_profile_resources_forward_quality(self):
        profile = _profile_with_curves()
        resources = assemble_encoder_profile_resources(profile, quality=6.0)
        # q=6 -> qnorm = 0.6000000999999999 -> f = 0.150000025:
        #   ath_offset = (1-f)*1 + f*2 = 1.150000025 -> f32 boundary
        self.assertEqual(resources.analysis.short_surface.ath_offset, 1.149999976158142)
        self.assertEqual(resources.analysis.quality_value, 6.0)
        self.assertFalse(resources.analysis.quality_extrapolated)

    def test_the_synthetic_carrier_does_not_disturb_the_installed_one(self):
        # One identity, one carrier: reading the synthetic profile must not
        # leave a cached table behind for the installed profile.
        _profile_with_curves()
        installed = resolve_selection(WwiseProfile(WwiseVersion.WWISE2013, 6, 44100))
        self.assertEqual(installed, installed_profile(6, 44100))
        self.assertIsNone(load_quality_curves(installed))


if __name__ == "__main__":
    unittest.main()
