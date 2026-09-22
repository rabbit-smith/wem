"""The quality axis: the interpolation kernel, its wiring, and the record family.

Three angles on one mechanism. The curve cases pin the interpolation kernel,
the schema rules and the pinned doubles the Rust kernel shares; the assembly
cases prove the wiring end to end on a synthetic carrier; the record-family
cases pin the transient tables materialized on the same axis, whose f64
indices and f32 words are shared with the Rust suite. Each class keeps its
own test names and failure messages.
"""

from __future__ import annotations

import struct
import unittest

from tests.analysis_resource_support import installed_profile
from wwise_wem import WwiseProfile, WwiseVersion
from wwise_wem_reference.profiles.artifact import resolve_selection
from wwise_wem_reference.profiles.assembly import (
    assemble_analysis_resources,
    assemble_encoder_profile_resources,
)
from wwise_wem_reference.profiles.quality import (
    QUALITY_CURVES_INTERPOLATION,
    QUALITY_CURVES_SCHEMA,
    QUALITY_NORMALIZE_ADDEND,
    QUALITY_NORMALIZE_CLAMP,
    QUALITY_SEMANTIC_NO_OP,
    QUALITY_SEMANTIC_SHORT_PREFIX,
    QUALITY_SEMANTIC_TRANSIENT_RECORD_INDEX_AXIS,
    QualityCurves,
    _linear_frac,
    load_quality_curves,
    normalize_quality_factor,
)
from wwise_wem_reference.profiles.transient import (
    TRANSIENT_RECORD_FAMILY_SCHEMA,
    load_transient_record_family,
    load_transient_resource,
    materialize_transient_tables,
)

# --------------------------------------------------------------------------
# merged from tests/unit/profiles/test_quality_curves.py
# --------------------------------------------------------------------------

# Synthetic-control-point tests for the quality-interpolation mechanism.
#
# These tests pin the interpolation kernel, the schema/validation rules, and
# the immutable value object using synthetic data only. They do not touch any
# installed profile resource, so they are independent of the 6ch calibration and
# exercise exactly the code the parallel quality-formula calibration will later
# re-target.
#
# Schema v2 adds the per-curve ``semantics`` map: every curve name must be
# routed onto the mechanism it drives (``short.<field>``, ``no-op``, or
# ``transient.record-index-axis``).

def _curves(breakpoints, samples):
    return QualityCurves(
        schema=QUALITY_CURVES_SCHEMA,
        breakpoints=breakpoints,
        curves={"short.ath_offset": samples},
        semantics={"short.ath_offset": "short.ath_offset"},
    )


class QualityCurvesKernelTests(unittest.TestCase):
    """The linear-frac kernel with synthetic control points [0,4,8]."""

    def test_mid_segment_interpolation(self):
        qc = _curves([0.0, 4.0, 8.0], [10.0, 20.0, 30.0])
        value, extrapolated = qc.evaluate_result(2.0)
        self.assertEqual(value["short.ath_offset"], 15.0)
        self.assertFalse(extrapolated)

    def test_second_segment_interpolation(self):
        qc = _curves([0.0, 4.0, 8.0], [10.0, 20.0, 30.0])
        value, extrapolated = qc.evaluate_result(6.0)
        self.assertEqual(value["short.ath_offset"], 25.0)
        self.assertFalse(extrapolated)

    def test_clamped_above_domain_reports_extrapolated(self):
        # Above the last breakpoint the (N-1, N) segment carries the value
        # via the spec N - 0.001 index clamp (not a plain samples[-1]).
        qc = _curves([0.0, 4.0, 8.0], [10.0, 20.0, 30.0])
        value, extrapolated = qc.evaluate_result(9.0)
        self.assertEqual(value["short.ath_offset"], 29.990000000000002)
        self.assertTrue(extrapolated)

    def test_clamped_below_domain_reports_extrapolated(self):
        qc = _curves([0.0, 4.0, 8.0], [10.0, 20.0, 30.0])
        value, extrapolated = qc.evaluate_result(-1.0)
        self.assertEqual(value["short.ath_offset"], 10.0)
        self.assertTrue(extrapolated)

    def test_exact_control_points_are_not_extrapolated(self):
        # The last control point is reached through the clamped index,
        # not stored verbatim: 8.0 -> (1-f)*20 + f*30, f ~= 0.999.
        qc = _curves([0.0, 4.0, 8.0], [10.0, 20.0, 30.0])
        for quality, expected in ((0.0, 10.0), (4.0, 20.0), (8.0, 29.990000000000002)):
            with self.subTest(quality=quality):
                value, extrapolated = qc.evaluate_result(quality)
                self.assertEqual(value["short.ath_offset"], expected)
                self.assertFalse(extrapolated)

    def test_evaluate_returns_only_values(self):
        qc = _curves([0.0, 4.0, 8.0], [10.0, 20.0, 30.0])
        values = qc.evaluate(2.0)
        self.assertEqual(values["short.ath_offset"], 15.0)
        self.assertIsInstance(values, dict)

    def test_linear_frac_is_the_single_kernel(self):
        # The raw kernel must agree with the object method on an in-domain point.
        value, outside = _linear_frac((0.0, 4.0, 8.0), (10.0, 20.0, 30.0), 2.0)
        self.assertEqual(value, 15.0)
        self.assertFalse(outside)


class QualityCurvesSpecParityTests(unittest.TestCase):
    """Bit-for-bit parity pins shared with the Rust kernel (quality.rs)."""

    def test_normalization_pins(self):
        # Pinned shared vectors: Py and Rust must return identical doubles.
        self.assertEqual(normalize_quality_factor(0.0), 1e-07)
        self.assertEqual(normalize_quality_factor(4.0), 0.4000001)
        self.assertEqual(normalize_quality_factor(9.0), 0.9000001)
        self.assertEqual(normalize_quality_factor(10.0), QUALITY_NORMALIZE_CLAMP)
        self.assertEqual(normalize_quality_factor(100.0), QUALITY_NORMALIZE_CLAMP)
        self.assertEqual(QUALITY_NORMALIZE_CLAMP, 0.9998999834060669)
        self.assertEqual(QUALITY_NORMALIZE_ADDEND, 1e-07)

    def test_two_step_fraction_pins(self):
        # (bp=[0.5,0.9], samples=[0,1]): the clamped-index segment and the
        # in-domain two-step fraction, pinned doubles.
        self.assertEqual(_linear_frac((0.5, 0.9), (0.0, 1.0), 0.1), (0.0, True))
        self.assertEqual(
            _linear_frac((0.5, 0.9), (0.0, 1.0), 0.7), (0.4999999999999999, False)
        )
        self.assertEqual(_linear_frac((0.5, 0.9), (0.0, 1.0), 0.9), (0.999, False))
        self.assertEqual(_linear_frac((0.5, 0.9), (0.0, 1.0), 1.0), (0.999, True))
        self.assertEqual(_linear_frac((0.5, 0.9), (0.0, 1.0), 2.0), (0.999, True))

    def test_large_index_two_step_difference(self):
        # At i > 0 the frac write/read round-trip is not the identity; pin
        # the two-step result on the 13-breakpoint two-channel control table.
        bp = (-0.2, -0.1, 0.0, 0.1, 0.2, 0.3, 0.4, 0.5, 0.6, 0.7, 0.8, 0.9, 1.0)
        samples = (
            12.9, 13.8, 14.7, 15.6, 16.5, 17.1, 18.0, 19.5, 48.0, 999.0, 999.0, 999.0,
            999.0,
        )
        value, outside = _linear_frac(bp, samples, normalize_quality_factor(4.0))
        self.assertFalse(outside)
        self.assertEqual(value, 18.0000015)

    def test_evaluate_rejects_non_finite_quality(self):
        qc = _curves([0.0, 4.0, 8.0], [10.0, 20.0, 30.0])
        with self.assertRaises(ValueError):
            qc.evaluate(float("nan"))


class QualityCurvesValidationTests(unittest.TestCase):
    """Schema, breakpoint, and curve completeness validation."""

    def test_schema_constants_are_stable(self):
        self.assertEqual(QUALITY_CURVES_SCHEMA, "wem.quality-curves.v2")
        self.assertEqual(QUALITY_CURVES_INTERPOLATION, "linear-frac")
        self.assertEqual(QUALITY_SEMANTIC_NO_OP, "no-op")
        self.assertEqual(QUALITY_SEMANTIC_SHORT_PREFIX, "short.")
        self.assertEqual(
            QUALITY_SEMANTIC_TRANSIENT_RECORD_INDEX_AXIS,
            "transient.record-index-axis",
        )

    def test_rejects_nan_curve_value(self):
        with self.assertRaisesRegex(ValueError, "finite"):
            _curves([0.0, 4.0, 8.0], [float("nan"), 20.0, 30.0])

    def test_rejects_infinite_curve_value(self):
        with self.assertRaisesRegex(ValueError, "finite"):
            _curves([0.0, 4.0, 8.0], [10.0, float("inf"), 30.0])

    def test_rejects_curve_length_mismatch(self):
        with self.assertRaisesRegex(ValueError, "must contain 3 values"):
            _curves([0.0, 4.0, 8.0], [10.0, 20.0])

    def test_rejects_non_monotonic_breakpoints(self):
        with self.assertRaisesRegex(ValueError, "strictly increasing"):
            _curves([0.0, 8.0, 4.0], [10.0, 20.0, 30.0])

    def test_rejects_equal_breakpoints(self):
        with self.assertRaisesRegex(ValueError, "strictly increasing"):
            _curves([4.0, 4.0], [10.0, 20.0])

    def test_rejects_too_few_breakpoints(self):
        with self.assertRaisesRegex(ValueError, "at least two"):
            QualityCurves(
                schema=QUALITY_CURVES_SCHEMA,
                breakpoints=[0.0],
                curves={"short.ath_offset": [10.0]},
                semantics={"short.ath_offset": "short.ath_offset"},
            )

    def test_rejects_empty_curves(self):
        with self.assertRaisesRegex(ValueError, "at least one curve"):
            QualityCurves(
                schema=QUALITY_CURVES_SCHEMA,
                breakpoints=[0.0, 4.0],
                curves={},
                semantics={},
            )

    def test_rejects_wrong_schema(self):
        with self.assertRaisesRegex(ValueError, "schema"):
            QualityCurves(
                schema="wem.quality-curves.v99",
                breakpoints=[0.0, 4.0],
                curves={"short.ath_offset": [10.0, 20.0]},
                semantics={"short.ath_offset": "short.ath_offset"},
            )

    def test_rejects_missing_semantics_entry(self):
        # v2: semantics must cover every curve name exactly.
        with self.assertRaisesRegex(ValueError, "semantics"):
            QualityCurves(
                schema=QUALITY_CURVES_SCHEMA,
                breakpoints=[0.0, 4.0],
                curves={"short.ath_offset": [10.0, 20.0]},
                semantics={},
            )

    def test_rejects_partial_semantics(self):
        with self.assertRaisesRegex(ValueError, "semantics"):
            QualityCurves(
                schema=QUALITY_CURVES_SCHEMA,
                breakpoints=[0.0, 4.0],
                curves={
                    "desc29.psy_int1": [10.0, 20.0],
                    "desc3.psy_float": [0.0, 1.0],
                },
                semantics={"desc29.psy_int1": "short.ath_offset"},
            )

    def test_rejects_extra_semantics_entry(self):
        with self.assertRaisesRegex(ValueError, "semantics"):
            QualityCurves(
                schema=QUALITY_CURVES_SCHEMA,
                breakpoints=[0.0, 4.0],
                curves={"desc29.psy_int1": [10.0, 20.0]},
                semantics={
                    "desc29.psy_int1": "short.ath_offset",
                    "desc99.psy_ghost": "no-op",
                },
            )

    def test_semantics_map_is_immutable(self):
        qc = _curves([0.0, 4.0, 8.0], [10.0, 20.0, 30.0])
        with self.assertRaises(TypeError):
            qc.semantics["short.ath_offset"] = "no-op"  # type: ignore[index]
        self.assertEqual(qc.semantics["short.ath_offset"], "short.ath_offset")

    def test_curves_are_immutable(self):
        qc = _curves([0.0, 4.0, 8.0], [10.0, 20.0, 30.0])
        with self.assertRaises(TypeError):
            qc.curves["short.ath_offset"] = (1.0, 2.0, 3.0)  # type: ignore[index]

    def test_a_profile_without_curves_reads_as_none(self):
        # The 6ch carrier records an empty breakpoint block, which is how
        # "this profile has no quality interpolation" travels now.
        profile = installed_profile(6, 44100)
        self.assertEqual(profile.table("quality_curves.breakpoints"), [])
        self.assertIsNone(load_quality_curves(profile))


# --------------------------------------------------------------------------
# merged from tests/unit/profiles/test_quality_assembly.py
# --------------------------------------------------------------------------

# End-to-end tests for the quality-override assembly wiring.
#
# These tests prove the three wiring guarantees:
#
# - ``quality`` omitted (``None``) leaves the assembled analysis resources
#   byte-for-byte identical to the historical path (no crosstalk);
# - a quality value requested from a profile that has no curves is a clear
#   configuration error;
# - a profile whose carrier records quality curves applies the interpolated
#   overrides to the short psychoacoustic surface and records the quality value
#   and the extrapolation honesty flag.
#
# The "with curves" case builds a carrier with a synthetic quality-curves table
# (the installed 6ch profile records none): the block shapes are the only thing
# that changes, and the same profile -> loader -> assembly path runs without
# touching the installed carrier's values.

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


# --------------------------------------------------------------------------
# merged from tests/unit/profiles/test_transient_record_family.py
# --------------------------------------------------------------------------

# Transient record-family tests: selection rule, materialization, and the
# shared cross-implementation parity pins.
#
# The expected f64 record indices and f32 bit patterns are shared test vectors
# with the Rust integration suite (crates/wem-profiles/tests/quality.rs):
# both implementations must agree bit for bit. The mechanism pins the static
# record family from the paired encoder build: the record-index curve on the
# shared quality axis picks a (possibly fractional) index, the floor record
# supplies every field verbatim, and only upper[0..3]/lower[0..3] are linearly
# interpolated between the adjacent records at the fractional part. No quality
# value selects the adjudicated default record index 3.0 (record #3).

#: The 2ch/48000 compiled profile: the one whose carrier records the family.
_PROFILE = installed_profile(2, 48000)


def _bits(value: float) -> int:
    return struct.unpack("<I", struct.pack("<f", float(value)))[0]


def _family():
    return load_transient_record_family(_PROFILE)


class RecordFamilyStructureTests(unittest.TestCase):
    def test_the_carrier_records_the_record_family(self):
        self.assertEqual(_PROFILE.table("transient.kind"), "record-family")
        self.assertEqual(_PROFILE.int("transient.n"), 128)
        self.assertEqual(_PROFILE.int("transient.sample_rate"), 48000)
        self.assertEqual(len(_PROFILE.table("transient.record_index_curve")), 13)
        self.assertEqual(len(_PROFILE.table("transient.quality_axis_breakpoints")), 13)
        self.assertEqual(_PROFILE.table("transient.default_record_index")[0], 3.0)
        self.assertEqual(len(_PROFILE.table("transient.window")), 128)
        self.assertEqual(_PROFILE.table("transient.bands.count")[0], 12)

    def test_family_loads_from_the_carrier(self):
        fam = _family()
        self.assertEqual(fam.schema, TRANSIENT_RECORD_FAMILY_SCHEMA)
        self.assertEqual(len(fam.records), 6)
        self.assertEqual(fam.default_record_index, 3.0)
        self.assertEqual(len(fam.window_u32), 128)
        self.assertEqual(len(fam.bands), 12)
        # Every record agrees with the stored words it was rebuilt from.
        for index, rec in enumerate(fam.records):
            prefix = f"transient.record.{index}"
            self.assertEqual(list(rec.upper_u32), _PROFILE.table(f"{prefix}.upper_u32"))
            self.assertEqual(list(rec.lower_u32), _PROFILE.table(f"{prefix}.lower_u32"))
            self.assertEqual(list(rec.config_u32), _PROFILE.table(f"{prefix}.config_u32"))
            self.assertEqual(rec.bias_u32, _PROFILE.table(f"{prefix}.bias_u32")[0])
            self.assertEqual(rec.marker_u32, 8)
            # Provenance words outside the 26-word config block must stay
            # byte-faithful to the static extraction (m = f32 -6.0, tail = 99).
            self.assertEqual(rec.m_u32, 3233808384)
            self.assertEqual(rec.tail_u32, 99)


class RecordIndexCurvePinTests(unittest.TestCase):
    """Shared f64 pins (identical in the Rust suite)."""

    PINS = (
        (-1.0, 1.0000010000000001),
        (0.0, 2.0),
        (1.0, 2.0000005),
        (2.0, 2.5000005),
        (4.0, 3.0000005),
        (5.5, 3.6000002),
        (7.0, 4.0),
        (8.0, 4.000000999999999),
        (10.0, 5.0),
    )

    def test_index_curve_pins(self):
        fam = _family()
        for quality, expected in self.PINS:
            with self.subTest(quality=quality):
                value, _outside = _linear_frac(
                    fam.breakpoints,
                    fam.index_curve,
                    normalize_quality_factor(quality),
                )
                self.assertEqual(value, expected)

    def test_default_without_quality_is_record_three(self):
        fam = _family()
        self.assertEqual(fam.default_record_index, 3.0)
        tables = materialize_transient_tables(fam, None)
        rec3 = fam.records[3]
        self.assertEqual(_bits(tables.bias), rec3.bias_u32)
        self.assertEqual(
            [_bits(word) for word in tables.config], list(rec3.config_u32)
        )


class MaterializationBitPinTests(unittest.TestCase):
    """Shared f32 bit-pattern pins for materialized tables."""

    def test_q4_lerps_only_the_first_four_bands(self):
        fam = _family()
        tables = materialize_transient_tables(fam, 4.0)
        bits = [_bits(word) for word in tables.config]
        # index 3.0000005: record 3 base + 5e-7 lerp toward record 4.
        self.assertEqual(bits[1], 1094713342)
        self.assertEqual(bits[2], 1092616191)
        self.assertEqual(bits[3], 1092616191)
        self.assertEqual(bits[4], 1092616190)
        self.assertEqual(bits[13], 3248488448)
        self.assertEqual(bits[14], 3248488447)
        self.assertEqual(bits[15], 3245342718)
        self.assertEqual(bits[16], 3245342718)
        # Floor-record fields verbatim (marker, bias, carry, tail words).
        rec3 = fam.records[3]
        self.assertEqual(bits[0], rec3.config_u32[0])
        self.assertEqual(bits[25], rec3.config_u32[25])
        for position in range(5, 13):
            self.assertEqual(bits[position], rec3.config_u32[position])
        for position in range(17, 25):
            self.assertEqual(bits[position], rec3.config_u32[position])
        self.assertEqual(_bits(tables.bias), rec3.bias_u32)

    def test_q1_and_q7_and_q10_pins(self):
        fam = _family()
        # q = 1.0 -> index 2.0000005: record 2 base + tiny lerp.
        self.assertEqual(_bits(materialize_transient_tables(fam, 1.0).config[1]), 1096810495)
        # q = 7.0 -> index 4.0 exactly: record 4 whole.
        rec4 = fam.records[4]
        t7 = [_bits(w) for w in materialize_transient_tables(fam, 7.0).config]
        self.assertEqual(t7, list(rec4.config_u32))
        # q = 10.0 -> index 5.0 exactly: record 5 whole.
        rec5 = fam.records[5]
        t10 = [_bits(w) for w in materialize_transient_tables(fam, 10.0).config]
        self.assertEqual(t10, list(rec5.config_u32))

    def test_fractional_index_uses_the_floor_record(self):
        fam = _family()
        tables = materialize_transient_tables(fam, 2.0)  # index 2.5000005
        rec2 = fam.records[2]
        self.assertEqual(_bits(tables.bias), rec2.bias_u32)
        # Interior lerp on the first interpolated word.
        a = struct.unpack("<f", struct.pack("<I", rec2.config_u32[1]))[0]
        b = struct.unpack("<f", struct.pack("<I", fam.records[3].config_u32[1]))[0]
        got = tables.config[1]
        self.assertTrue(min(a, b) < got < max(a, b), f"expected {got} between {a} and {b}")

    def test_window_and_band_pins(self):
        fam = _family()
        tables = materialize_transient_tables(fam, None)
        self.assertEqual(len(tables.window), 128)
        self.assertEqual(_bits(tables.window[127]), 0x2809ADED)
        self.assertEqual(_bits(tables.window[0]), 0x00000000)
        self.assertEqual(_bits(tables.bands[0].weights[0]), 1053028118)
        self.assertEqual(len(tables.bands), 12)
        for band, stored in zip(tables.bands, fam.bands):
            self.assertEqual(band, stored)


class DispatchAndRegressionTests(unittest.TestCase):
    def test_carrier_dispatch_on_mechanism(self):
        tables = load_transient_resource(_PROFILE, None)
        self.assertEqual(_bits(tables.bias), _family().records[3].bias_u32)
        # An explicit quality flows through the same kernel as materialize.
        fam = _family()
        q4 = load_transient_resource(_PROFILE, 4.0)
        pin = materialize_transient_tables(fam, 4.0)
        self.assertEqual(
            [_bits(w) for w in q4.config], [_bits(w) for w in pin.config]
        )

    def test_six_ch_profile_keeps_the_static_table_path(self):
        # The 6ch profile records a pre-materialized detector table; reading
        # it must return the recorded words unchanged, including at a quality
        # value (quality is ignored by that path — 6ch records no
        # quality-curves table, so a real assembly cannot hand it one; the
        # direct reader call proves the branch is quality-independent).
        from wwise_wem_reference.profiles.transient import load_transient_tables

        profile = installed_profile(6, 44100)
        self.assertEqual(profile.table("transient.kind"), "detector")
        tables = load_transient_tables(profile)
        self.assertEqual(
            [_bits(w) for w in tables.window],
            [_bits(w) for w in profile.table("transient.window")],
        )
        self.assertEqual(
            [_bits(w) for w in tables.config],
            [_bits(w) for w in profile.table("transient.config")],
        )
        self.assertEqual(_bits(tables.bias), _bits(profile.table("transient.bias")[0]))


if __name__ == "__main__":
    unittest.main()
