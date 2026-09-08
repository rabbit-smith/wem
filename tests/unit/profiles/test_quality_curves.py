"""Synthetic-control-point tests for the quality-interpolation mechanism.

These tests pin the interpolation kernel, the schema/validation contract, and
the immutable value object using synthetic data only. They do not touch any
installed profile resource, so they are independent of the 6ch calibration and
exercise exactly the code the parallel quality-formula calibration will later
re-target.
"""
from __future__ import annotations

import unittest

from wwise_wem_reference.profiles.quality import (
    QUALITY_CURVES_INTERPOLATION,
    QUALITY_CURVES_RESOURCE,
    QUALITY_CURVES_SCHEMA,
    QualityCurves,
    _linear_frac,
    load_quality_curves,
)


def _curves(breakpoints, samples):
    return QualityCurves(
        schema=QUALITY_CURVES_SCHEMA,
        breakpoints=breakpoints,
        curves={"short.ath_offset": samples},
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
        qc = _curves([0.0, 4.0, 8.0], [10.0, 20.0, 30.0])
        value, extrapolated = qc.evaluate_result(9.0)
        self.assertEqual(value["short.ath_offset"], 30.0)
        self.assertTrue(extrapolated)

    def test_clamped_below_domain_reports_extrapolated(self):
        qc = _curves([0.0, 4.0, 8.0], [10.0, 20.0, 30.0])
        value, extrapolated = qc.evaluate_result(-1.0)
        self.assertEqual(value["short.ath_offset"], 10.0)
        self.assertTrue(extrapolated)

    def test_exact_control_points_are_not_extrapolated(self):
        qc = _curves([0.0, 4.0, 8.0], [10.0, 20.0, 30.0])
        for quality, expected in ((0.0, 10.0), (4.0, 20.0), (8.0, 30.0)):
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

    def test_evaluate_rejects_non_finite_quality(self):
        qc = _curves([0.0, 4.0, 8.0], [10.0, 20.0, 30.0])
        with self.assertRaises(ValueError):
            qc.evaluate(float("nan"))


class QualityCurvesValidationTests(unittest.TestCase):
    """Schema, breakpoint, and curve completeness validation."""

    def test_schema_constants_are_stable(self):
        self.assertEqual(QUALITY_CURVES_SCHEMA, "wem.quality-curves.v1")
        self.assertEqual(QUALITY_CURVES_INTERPOLATION, "linear-frac")
        self.assertEqual(QUALITY_CURVES_RESOURCE, "analysis.quality-curves")

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
            )

    def test_rejects_empty_curves(self):
        with self.assertRaisesRegex(ValueError, "at least one curve"):
            QualityCurves(
                schema=QUALITY_CURVES_SCHEMA,
                breakpoints=[0.0, 4.0],
                curves={},
            )

    def test_rejects_wrong_schema(self):
        with self.assertRaisesRegex(ValueError, "schema"):
            QualityCurves(
                schema="wem.quality-curves.v99",
                breakpoints=[0.0, 4.0],
                curves={"short.ath_offset": [10.0, 20.0]},
            )

    def test_curves_are_immutable(self):
        qc = _curves([0.0, 4.0, 8.0], [10.0, 20.0, 30.0])
        with self.assertRaises(TypeError):
            qc.curves["short.ath_offset"] = (1.0, 2.0, 3.0)  # type: ignore[index]

    def test_load_quality_curves_none_in_none_out(self):
        self.assertIsNone(load_quality_curves(None))


if __name__ == "__main__":
    unittest.main()
