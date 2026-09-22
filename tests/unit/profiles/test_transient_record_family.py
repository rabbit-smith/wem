"""Transient record-family tests: selection rule, materialization, and the
shared cross-implementation parity pins.

The expected f64 record indices and f32 bit patterns are shared test vectors
with the Rust integration suite (crates/wem-profiles/tests/record_family.rs):
both implementations must agree bit for bit. The mechanism pins the static
record family from the paired encoder build: the record-index curve on the
shared quality axis picks a (possibly fractional) index, the floor record
supplies every field verbatim, and only upper[0..3]/lower[0..3] are linearly
interpolated between the adjacent records at the fractional part. No quality
value selects the adjudicated default record index 3.0 (record #3).
"""
from __future__ import annotations

import struct
import unittest

from tests.analysis_resource_support import installed_profile
from wwise_wem_reference.profiles.quality import (
    _linear_frac,
    normalize_quality_factor,
)
from wwise_wem_reference.profiles.transient import (
    TRANSIENT_RECORD_FAMILY_SCHEMA,
    load_transient_record_family,
    load_transient_resource,
    materialize_transient_tables,
)

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
