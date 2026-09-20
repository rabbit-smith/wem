"""Synthetic-control-point tests for the quality-interpolation mechanism.

These tests pin the interpolation kernel, the schema/validation contract, and
the immutable value object using synthetic data only. They do not touch any
installed profile resource, so they are independent of the 6ch calibration and
exercise exactly the code the parallel quality-formula calibration will later
re-target.

Schema v2 adds the per-curve ``semantics`` map: every curve name must be
routed onto the mechanism it drives (``short.<field>``, ``no-op``, or
``transient.record-index-axis``).
"""
from __future__ import annotations

import unittest

from wwise_wem_reference.profiles.quality import (
    QUALITY_CURVES_INTERPOLATION,
    QUALITY_CURVES_RESOURCE,
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
        self.assertEqual(QUALITY_CURVES_RESOURCE, "analysis.quality-curves")
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

    def test_load_quality_curves_none_in_none_out(self):
        self.assertIsNone(load_quality_curves(None))


if __name__ == "__main__":
    unittest.main()
