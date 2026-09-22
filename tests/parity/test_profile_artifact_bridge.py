"""Compiled-carrier bridge: the kernel's dump equals the recorded documents.

The kernel owns the profile values now and hands them out as one canonical
little-endian stream (``wwise_wem._core.profile_tables()``); the pure-Python
reference reads its numbers from there. This suite is what makes that
substitution safe to believe: it decodes the stream and holds every table
against the recorded documents, value for value and bit pattern for bit
pattern.

It is the Python half of ``crates/wem-profiles/src/carrier_tests.rs``. Between
them the two say the carrier moved the data without moving a value; the
byte-exactness suites say the same thing end to end.

Float documents are re-read through :func:`serde_f64` — the generator's copy of
the kernel reader's decimal-to-`f64` step — because that parse is not correctly
rounded and the kernel's numbers came through it. Comparing against a
correctly rounded parse instead would report a difference the kernel does not
have.
"""

from __future__ import annotations

import base64
import json
import pathlib
import struct
import sys
import unittest

from wwise_wem import WwiseProfile, WwiseVersion
from wwise_wem.profiles.bundle import load_profile_bundle
from wwise_wem_reference.profiles.artifact import ProfileBlobError, decode

import wwise_wem._core as _core

sys.path.insert(0, str(pathlib.Path(__file__).resolve().parents[2] / "scripts"))
from generate_profile_code import serde_f64  # noqa: E402  (test-only helper)

SELECTIONS = (
    (WwiseProfile(WwiseVersion.WWISE2013, 6, 44100), "wwise2013-6ch-44100"),
    (WwiseProfile(WwiseVersion.WWISE2013, 2, 48000), "wwise2013-2ch-48000"),
)

CONTAINER_KEYS = (
    "wFormatTag",
    "nChannels",
    "nSamplesPerSec",
    "nAvgBytesPerSec",
    "nBlockAlign",
    "wBitsPerSample",
    "cbSize",
    "wReserved0",
    "dwChannelMask",
    "dwTotalPCMFrames",
    "dwFirstAudioPacketOffset",
    "dwDataPayloadSize",
    "dwUnknown_0x24",
    "dwSeekTableSize",
    "dwVorbisDataOffset",
    "uMaxPacketSize",
    "uUnknown_0x32",
    "dwUnknown_0x34",
    "dwUnknown_0x38",
    "dwUnknown_0x3C",
    "uBlocksize0Pow",
    "uBlocksize1Pow",
)

CODEBOOK_NAMES = ("t97", "t219", "t282")

KERNEL_PARSE = json.JSONDecoder(parse_float=serde_f64).decode


def _f32_word(value: float) -> int:
    return struct.unpack("<I", struct.pack("<f", value))[0]


def _f64_word(value: float) -> int:
    return struct.unpack("<Q", struct.pack("<d", value))[0]


class CompiledCarrierBridgeTests(unittest.TestCase):
    """Hold the kernel's dump against the documents it was generated from."""

    @classmethod
    def setUpClass(cls) -> None:
        cls.profiles = {profile.name: profile for profile in decode(bytes(_core.profile_tables()))}
        cls.bundles = {
            name: load_profile_bundle(profile=name) for _, name in SELECTIONS
        }

    def document(self, name: str, logical: str) -> object:
        """One recorded document, parsed as the kernel reader parses it."""
        ref = self.bundles[name].runtime_manifest.resource(logical)
        return KERNEL_PARSE(ref.read_text())

    def test_the_stream_declares_every_installed_profile(self) -> None:
        self.assertEqual(
            sorted(self.profiles),
            sorted(name for _, name in SELECTIONS),
            "the dump must carry exactly the installed profiles",
        )

    def test_identity_geometry_and_setup_packet_match(self) -> None:
        for _, name in SELECTIONS:
            with self.subTest(profile=name):
                compiled = self.profiles[name]
                bundle = self.bundles[name]
                self.assertEqual(compiled.channels, bundle.key.channels)
                self.assertEqual(compiled.sample_rate, bundle.key.sample_rate)
                self.assertEqual(compiled.generation, bundle.key.generation)
                self.assertEqual(compiled.channel_layout, bundle.key.channel_layout)
                self.assertEqual(
                    compiled.quality_setup_identity, bundle.key.quality_setup_identity
                )
                self.assertEqual(
                    compiled.block_sizes, (bundle.block_sizes[0], bundle.block_sizes[1])
                )
                self.assertEqual(compiled.setup_packet, bundle.setup_packet())
                self.assertEqual(compiled.setup_sha256, bundle.setup.sha256)
                fmt = bundle.container_metadata.to_fmt_dict()
                self.assertEqual(
                    compiled.container_fields, tuple(fmt[key] for key in CONTAINER_KEYS)
                )

    def test_mdct_banks_match_word_for_word(self) -> None:
        for _, name in SELECTIONS:
            with self.subTest(profile=name):
                compiled = self.profiles[name]
                payload = self.document(name, "transform.mdct")
                for key, entry in payload["profiles"].items():
                    raw = base64.b64decode(entry["little_endian_f32_base64"], validate=True)
                    words = struct.unpack(f"<{len(raw) // 4}I", raw)
                    carried = compiled.table(f"mdct.{int(key)}.trig")
                    self.assertEqual(
                        [_f32_word(value) for value in carried], list(words), f"n={key}"
                    )

    def test_codebook_rows_match(self) -> None:
        for _, name in SELECTIONS:
            for table in CODEBOOK_NAMES:
                logical = f"vorbis.codebooks.{table}"
                if logical not in self.bundles[name].runtime_manifest.resources:
                    continue
                with self.subTest(profile=name, table=table):
                    compiled = self.profiles[name]
                    rows = self.document(name, logical)

                    def column(field: str, default: int = 0) -> list[int]:
                        return [
                            int(row.get(field) or default) if row.get(field) is not None else default
                            for row in rows
                        ]

                    for field in ("dim", "entries"):
                        self.assertEqual(
                            compiled.table(f"codebook.{table}.{field}"), column(field), field
                        )
                    for field in ("maptype", "q_min", "q_delta", "q_quant", "q_sequencep"):
                        self.assertEqual(
                            compiled.table(f"codebook.{table}.{field}"), column(field), field
                        )
                    self.assertEqual(
                        compiled.table(f"codebook.{table}.i"),
                        [int(row["i"]) if row.get("i") is not None else 0 for row in rows],
                    )
                    self.assertEqual(
                        compiled.table(f"codebook.{table}.i_present"),
                        [int(row.get("i") is not None) for row in rows],
                    )
                    self.assertEqual(
                        compiled.table(f"codebook.{table}.quantvals"),
                        [
                            int(row["quantvals"]) if row.get("quantvals") is not None else 0
                            for row in rows
                        ],
                    )
                    for field in ("lengthlist", "quantlist"):
                        flat: list[int] = []
                        offsets = [0]
                        present = []
                        for row in rows:
                            values = row.get(field)
                            present.append(int(values is not None))
                            if values is not None:
                                flat.extend(int(value) for value in values)
                            offsets.append(len(flat))
                        self.assertEqual(compiled.table(f"codebook.{table}.{field}"), flat, field)
                        self.assertEqual(
                            compiled.table(f"codebook.{table}.{field}_offsets"), offsets, field
                        )
                        self.assertEqual(
                            compiled.table(f"codebook.{table}.{field}_present"), present, field
                        )

    def test_frozen_tables_match_word_for_word(self) -> None:
        for _, name in SELECTIONS:
            with self.subTest(profile=name):
                compiled = self.profiles[name]
                payload = self.document(name, "analysis.frozen-tables")
                pairs = payload["coordinate_ln"]
                self.assertEqual(
                    compiled.table("frozen.coordinate_ln.in_bits"),
                    [int.from_bytes(bytes.fromhex(pair[0]), "little") for pair in pairs],
                )
                self.assertEqual(
                    compiled.table("frozen.coordinate_ln.out_bits"),
                    [int.from_bytes(bytes.fromhex(pair[1]), "little") for pair in pairs],
                )
                twiddles = payload["fft_twiddles"]
                self.assertEqual(
                    compiled.table("frozen.fft_twiddles.length"),
                    [int(triple[0]) for triple in twiddles],
                )
                self.assertEqual(
                    compiled.table("frozen.fft_twiddles.cos_bits"),
                    [int.from_bytes(bytes.fromhex(triple[1]), "little") for triple in twiddles],
                )
                self.assertEqual(
                    compiled.table("frozen.fft_twiddles.sin_bits"),
                    [int.from_bytes(bytes.fromhex(triple[2]), "little") for triple in twiddles],
                )
                sizes = sorted(int(size) for size in payload["window_halves"])
                self.assertEqual(compiled.table("frozen.window_halves.size"), sizes)
                flat: list[int] = []
                offsets = [0]
                for size in sizes:
                    flat.extend(
                        int.from_bytes(bytes.fromhex(word), "little")
                        for word in payload["window_halves"][str(size)]
                    )
                    offsets.append(len(flat))
                self.assertEqual(compiled.table("frozen.window_halves.offsets"), offsets)
                self.assertEqual(
                    [_f32_word(value) for value in compiled.table("frozen.window_halves.words")],
                    flat,
                )

    def test_transient_tables_match_word_for_word(self) -> None:
        for _, name in SELECTIONS:
            with self.subTest(profile=name):
                compiled = self.profiles[name]
                payload = self.document(name, "analysis.transient")
                self.assertEqual(
                    [_f32_word(value) for value in compiled.table("transient.window")],
                    [int(word) & 0xFFFFFFFF for word in payload["window_u32"]],
                )
                bands = payload["bands"]
                self.assertEqual(
                    [
                        compiled.int(f"transient.bands.{index}.offset")
                        for index in range(len(bands))
                    ],
                    [int(band["offset"]) for band in bands],
                )
                for index, band in enumerate(bands):
                    self.assertEqual(
                        [
                            _f32_word(value)
                            for value in compiled.table(f"transient.bands.{index}.weights")
                        ],
                        [int(word) & 0xFFFFFFFF for word in band["weights_u32"]],
                        f"band {index} weights",
                    )
                    self.assertEqual(
                        _f32_word(compiled.floats(f"transient.bands.{index}.scale")[0]),
                        int(band["scale_u32"]) & 0xFFFFFFFF,
                        f"band {index} scale",
                    )
                if payload["schema"] == "wem.transient-detector-table.v1":
                    self.assertEqual(compiled.table("transient.kind"), "detector")
                    self.assertEqual(compiled.int("transient.n"), int(payload["n"]))
                    self.assertEqual(
                        _f32_word(compiled.floats("transient.bias")[0]),
                        int(payload["bias_u32"]) & 0xFFFFFFFF,
                    )
                    self.assertEqual(
                        [_f32_word(value) for value in compiled.table("transient.config")],
                        [int(word) & 0xFFFFFFFF for word in payload["config_u32"]],
                    )
                    continue

                self.assertEqual(compiled.table("transient.kind"), "record-family")
                self.assertEqual(compiled.int("transient.n"), int(payload["n"]))
                self.assertEqual(
                    compiled.int("transient.sample_rate"), int(payload["sample_rate"])
                )
                self.assertEqual(
                    _f64_word(compiled.floats("transient.default_record_index")[0]),
                    _f64_word(payload["default_record_index"]),
                )
                self.assertEqual(
                    [
                        _f64_word(value)
                        for value in compiled.table("transient.record_index_curve")
                    ],
                    [
                        _f64_word(value)
                        for value in payload["record_index_curve"]["values"]
                    ],
                )
                self.assertEqual(
                    [
                        _f64_word(value)
                        for value in compiled.table("transient.quality_axis_breakpoints")
                    ],
                    [
                        _f64_word(value)
                        for value in payload["quality_axis_breakpoints"]["values"]
                    ],
                )
                records = payload["record_family"]["records"]
                for index, record in enumerate(records):
                    self.assertEqual(
                        compiled.table(f"transient.record.{index}.file_off"),
                        record["file_off"],
                    )
                    for field in ("marker_u32", "carry_u32", "bias_u32", "m_u32", "tail_u32"):
                        self.assertEqual(
                            compiled.int(f"transient.record.{index}.{field}"),
                            int(record[field]) & 0xFFFFFFFF,
                            f"record {index} {field}",
                        )
                    for field in ("upper_u32", "lower_u32", "config_u32"):
                        self.assertEqual(
                            compiled.table(f"transient.record.{index}.{field}"),
                            [int(word) & 0xFFFFFFFF for word in record[field]],
                            f"record {index} {field}",
                        )

    def test_quality_curves_match_when_the_profile_registers_them(self) -> None:
        for _, name in SELECTIONS:
            with self.subTest(profile=name):
                compiled = self.profiles[name]
                registered = "analysis.quality-curves" in self.bundles[
                    name
                ].runtime_manifest.resources
                breakpoints = compiled.table("quality_curves.breakpoints")
                self.assertEqual(bool(breakpoints), registered)
                if not registered:
                    continue
                payload = self.document(name, "analysis.quality-curves")
                self.assertEqual(
                    [_f64_word(value) for value in breakpoints],
                    [_f64_word(value) for value in payload["breakpoints"]],
                )
                self.assertEqual(
                    sorted(
                        key.removeprefix("quality_curves.curve.")
                        for key in compiled.tables
                        if key.startswith("quality_curves.curve.")
                    ),
                    sorted(payload["curves"]),
                )
                for curve_name, values in payload["curves"].items():
                    self.assertEqual(
                        [
                            _f64_word(value)
                            for value in compiled.table(f"quality_curves.curve.{curve_name}")
                        ],
                        [_f64_word(value) for value in values],
                        curve_name,
                    )
                for curve_name, semantic in payload["semantics"].items():
                    self.assertEqual(
                        compiled.table(f"quality_curves.semantic.{curve_name}"), semantic
                    )

    def test_short_seed_surface_matches(self) -> None:
        for _, name in SELECTIONS:
            with self.subTest(profile=name):
                compiled = self.profiles[name]
                payload = self.document(name, "psychoacoustics.short-seed")
                look = payload["look"]
                profile = payload["profile"]
                for field in ("ath_offset", "ath_floor", "seed_ceiling", "max_curve_db",
                              "curve_offset", "curve_slope", "curve_offset_2"):
                    self.assertEqual(
                        _f32_word(compiled.floats(f"short_seed.{field}")[0]),
                        _f32_word(profile[field]),
                        field,
                    )
                self.assertEqual(compiled.int("short_seed.n"), int(payload["geometry"]["n"]))
                self.assertEqual(
                    compiled.int("short_seed.sample_rate"), int(payload["geometry"]["sample_rate"])
                )
                for field in ("first_octave", "shift_octave", "eighth_octave_lines",
                              "total_octave_lines"):
                    self.assertEqual(
                        compiled.int(f"short_seed.{field}"), int(payload["geometry"][field]), field
                    )
                for field in ("short_limit", "noise_fixed_window"):
                    self.assertEqual(
                        compiled.int(f"short_seed.{field}"), int(look[field]), field
                    )
                for field in ("ath", "octave", "row", "envelope_low", "envelope_high",
                              "mask_curve", "interval_table", "remap_curve_offsets",
                              "remap_low_by_index", "remap_high_by_index",
                              "curve_attenuation"):
                    source = profile[field] if field == "curve_attenuation" else payload.get(
                        field, look.get(field)
                    )
                    carried = compiled.table(f"short_seed.{field}")
                    self.assertEqual(len(carried), len(source), f"{field} length")
                    if field in ("octave", "interval_table", "remap_curve_offsets"):
                        self.assertEqual(carried, [int(value) for value in source], field)
                    else:
                        self.assertEqual(
                            [_f32_word(value) for value in carried],
                            [_f32_word(value) for value in source],
                            field,
                        )
                for field in ("regular_curve_bias", "regular_curve_cap"):
                    self.assertEqual(
                        _f32_word(compiled.floats(f"short_seed.{field}")[0]),
                        _f32_word(look[field]),
                        field,
                    )

    def test_short_profiles_match(self) -> None:
        for _, name in SELECTIONS:
            with self.subTest(profile=name):
                compiled = self.profiles[name]
                payload = self.document(name, "psychoacoustics.short-profiles")
                for index, row in enumerate(payload["profiles"]):
                    prefix = f"short_profiles.{index}"
                    self.assertEqual(compiled.table(f"{prefix}.key"), row["key"])
                    self.assertEqual(
                        compiled.int(f"{prefix}.group_enabled"), int(row["group_enabled"])
                    )
                    self.assertEqual(
                        compiled.int(f"{prefix}.candidate_bound"), int(row["candidate_bound"])
                    )
                    self.assertEqual(compiled.int(f"{prefix}.group_span"), int(row["group_span"]))
                    self.assertEqual(
                        compiled.table(f"{prefix}.band_limits"),
                        [int(value) for value in row["band_limits"]],
                    )
                    self.assertEqual(compiled.int(f"{prefix}.peak_cutoff"), int(row["peak_cutoff"]))
                    for field in ("curve_cap", "side_gain", "blend_weight"):
                        self.assertEqual(
                            _f64_word(compiled.floats(f"{prefix}.{field}")[0]),
                            _f64_word(row[field]),
                            field,
                        )
                    self.assertEqual(
                        [
                            _f64_word(value)
                            for value in compiled.table(f"{prefix}.candidate_bias_by_mode")
                        ],
                        [
                            _f64_word(value)
                            for value in row["candidate_bias_by_mode"]
                        ],
                    )
                    self.assertEqual(
                        [_f64_word(value) for value in compiled.table(f"{prefix}.mask_curves")],
                        [
                            _f64_word(value)
                            for curve in row["mask_curves"]
                            for value in curve
                        ],
                    )

    def test_long_tables_match(self) -> None:
        for _, name in SELECTIONS:
            with self.subTest(profile=name):
                compiled = self.profiles[name]
                payload = self.document(name, "psychoacoustics.long-base")
                analysis = payload["analysis"]
                seed = payload["seed"]
                self.assertEqual(compiled.int("long_base.n"), int(payload["n"]))
                self.assertEqual(
                    compiled.int("long_base.sample_rate"), int(payload["sample_rate"])
                )
                self.assertEqual(
                    compiled.table("long_base.profile_key"), payload["profile_key"]
                )
                # Carrier field name -> the recorded document's field.
                for field, source in (
                    ("analysis_profile_u32", analysis["profile_u32"]),
                    ("analysis_interval_u32", analysis["interval_u32"]),
                    ("seed_outer_u32", seed["outer_u32"]),
                    ("seed_profile_u32", seed["profile_u32"]),
                    ("seed_group_labels_u32", seed["group_labels_u32"]),
                ):
                    self.assertEqual(
                        compiled.table(f"long_base.{field}"),
                        [int(value) for value in source],
                        field,
                    )
                for field, source in (
                    ("analysis_curves", [v for curve in analysis["curves"] for v in curve]),
                    ("analysis_field_19_curve", analysis["field_19_curve"]),
                    ("seed_base_curve", seed["base_curve"]),
                    (
                        "seed_tone_banks",
                        [v for band in seed["tone_banks"] for level in band for v in level],
                    ),
                ):
                    carried = compiled.table(f"long_base.{field}")
                    self.assertEqual(len(carried), len(source), f"{field} length")
                    self.assertEqual(
                        [_f32_word(value) for value in carried],
                        [_f32_word(value) for value in source],
                        field,
                    )

                variants = self.document(name, "psychoacoustics.long-modes")
                for mode, surface in variants["variants"].items():
                    for field in ("profile_u32", "interval_u32"):
                        key = "analysis_profile_u32" if field == "profile_u32" else "analysis_interval_u32"
                        self.assertEqual(
                            compiled.table(f"long_variants.{mode}.{key}"),
                            [int(value) & 0xFFFFFFFF for value in surface[field]],
                            f"mode {mode} {key}",
                        )
                    for field, source in (
                        ("analysis_curves", [v for curve in surface["curves"] for v in curve]),
                        ("analysis_field_19_curve", surface["field_19_curve"]),
                    ):
                        carried = compiled.table(f"long_variants.{mode}.{field}")
                        self.assertEqual(
                            [_f32_word(value) for value in carried],
                            [_f32_word(value) for value in source],
                            f"mode {mode} {field}",
                        )

    def test_input_conditioner_matches(self) -> None:
        for _, name in SELECTIONS:
            with self.subTest(profile=name):
                compiled = self.profiles[name]
                registered = (
                    "analysis.input-conditioner"
                    in self.bundles[name].runtime_manifest.resources
                )
                bits = compiled.table("input_conditioner.bits")
                self.assertEqual(bool(bits), registered)
                if registered:
                    payload = self.document(name, "analysis.input-conditioner")
                    self.assertEqual(bits, [int(payload["dc_filter_coefficient_f32_bits"])])

    def test_an_unknown_stream_version_is_rejected(self) -> None:
        with self.assertRaises(ProfileBlobError):
            decode(b"WEMPROF\0" + struct.pack("<I", 999) + struct.pack("<I", 0))


if __name__ == "__main__":
    unittest.main()
