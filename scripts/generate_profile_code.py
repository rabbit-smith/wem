#!/usr/bin/env python3
"""Record the encoder profile tree as Rust typed tables.

The profile values are external facts measured from the paired Wwise build;
this script turns them into the kernel's single carrier: Rust source under
``crates/wem-profiles/src/generated/``.  The compiled artifact is what every
other language reads; nothing ships a resource tree.

What it needs
-------------
The profile tree (``index.json`` plus one directory per profile).  It is
development material and lives in the untracked tree ``corpus/profiles/`` at
the repository root — the layout the package shipped under ``data/profiles/``
until the serialized layer was deleted; ``--profiles-dir`` overrides the
location and is the only input.  The default is printed by ``--help``.

Exactness
---------
Every floating-point value is written as an integer bit pattern
(``f32::from_bits`` / ``f64::from_bits``), never as a decimal literal, and
every integer table keeps the width the kernel's loader gave it.  The output
is therefore byte-identical to what the JSON loaders decoded, and running the
script twice produces an empty diff.  No network, no timestamps, no absolute
paths: the header records a content digest over the tree.

Usage
-----
    python3 scripts/generate_profile_code.py            # write the tables
    python3 scripts/generate_profile_code.py --check     # fail if they drift
"""
from __future__ import annotations

import argparse
import base64
import hashlib
import json
import struct
import sys
from pathlib import Path
from typing import Any, Iterable, Sequence

REPO_ROOT = Path(__file__).resolve().parents[1]
DEFAULT_PROFILES_DIR = REPO_ROOT / "corpus" / "profiles"
DEFAULT_OUT_DIR = REPO_ROOT / "crates" / "wem-profiles" / "src" / "generated"

GENERATED_BY = "scripts/generate_profile_code.py"
HEADER = (
    "//! Generated encoder profile tables. Do not edit; regenerate with\n"
    "//! `python3 {script}`.\n"
    "//!\n"
    "//! Source material: the profile tree (`--help` names the location),\n"
    "//! content digest `{digest}`. Every float is stored as its IEEE bit\n"
    "//! pattern, never as a decimal literal.\n"
)

# ---------------------------------------------------------------------------
# scalar conversions (mirror the kernel loaders exactly)
# ---------------------------------------------------------------------------


def f32_bits(value: float) -> int:
    """`value as f32` in Rust / `_f32` in Python, as its stored u32 word."""
    return struct.unpack("<I", struct.pack("<f", float(value)))[0]


def f64_bits(value: float) -> int:
    """The stored u64 word of an f64 field."""
    return struct.unpack("<Q", struct.pack("<d", float(value)))[0]


def json_int(value: Any) -> int | None:
    """`json_int` in the kernel: integers pass, finite floats truncate."""
    if isinstance(value, bool) or not isinstance(value, (int, float)):
        return None
    if isinstance(value, float):
        if value != value or value in (float("inf"), float("-inf")):
            return None
        return int(value)
    return int(value)


def strict_int(value: Any) -> int | None:
    if isinstance(value, bool) or not isinstance(value, int):
        return None
    return int(value)


def u32_masked(value: Any) -> int:
    """`int(value) & 0xFFFFFFFF` for stored u32 words."""
    number = json_int(value)
    if number is None:
        raise ValueError(f"not a stored u32 word: {value!r}")
    return number & 0xFFFFFFFF


def strict_u32(value: Any) -> int:
    number = strict_int(value)
    if number is None or not 0 <= number <= 0xFFFFFFFF:
        raise ValueError(f"not a strict u32: {value!r}")
    return number


# ---------------------------------------------------------------------------
# Rust literal emitters
# ---------------------------------------------------------------------------

LINE_WIDTH = 96


def _rows(items: Sequence[str], indent: str = "    ") -> list[str]:
    """Pack items into lines no wider than LINE_WIDTH."""
    lines: list[str] = []
    current = indent
    for item in items:
        candidate = f"{current} {item}" if current.strip() else f"{indent}{item}"
        if current.strip() and len(candidate) > LINE_WIDTH:
            lines.append(current)
            current = indent + item
        else:
            current = candidate
    if current.strip():
        lines.append(current)
    return lines


def f32_slice(values: Iterable[float]) -> list[str]:
    return _rows([f"f32::from_bits(0x{f32_bits(v):08x})," for v in values])


def f32_bits_slice(words: Iterable[int]) -> list[str]:
    """An f32 array whose source values are already stored u32 words."""
    return _rows([f"f32::from_bits(0x{word:08x})," for word in words])


def f64_slice(values: Iterable[float]) -> list[str]:
    return _rows([f"f64::from_bits(0x{f64_bits(v):016x})," for v in values])


def u32_slice(values: Iterable[int]) -> list[str]:
    return _rows([f"0x{v:08x}," for v in values])


def u64_slice(values: Iterable[int]) -> list[str]:
    return _rows([f"0x{v:016x}," for v in values])


def i64_slice(values: Iterable[int]) -> list[str]:
    return _rows([f"{v}," for v in values])


def bytes_slice(values: bytes) -> list[str]:
    return _rows([f"0x{v:02x}," for v in values])


def emit_static(
    lines: list[str],
    name: str,
    rust_type: str,
    body: Sequence[str],
    *,
    trailing: str = "];",
    opening: str = "= [",
) -> None:
    lines.append("#[rustfmt::skip]")
    lines.append(f"static {name}: {rust_type} {opening}")
    lines.extend(body)
    lines.append(trailing)
    lines.append("")


def emit_pub_static(
    lines: list[str],
    name: str,
    rust_type: str,
    body: Sequence[str],
    *,
    trailing: str = "];",
    opening: str = "= [",
) -> None:
    lines.append("#[rustfmt::skip]")
    lines.append(f"pub static {name}: {rust_type} {opening}")
    lines.extend(body)
    lines.append(trailing)
    lines.append("")


def bytes_literal(name: str, payload: bytes, *, public: bool = False) -> list[str]:
    lines: list[str] = []
    emit = emit_pub_static if public else emit_static
    emit(lines, name, f"[u8; {len(payload)}]", bytes_slice(payload))
    return lines


# ---------------------------------------------------------------------------
# resource decoding (mirrors the kernel loaders; the generator validates it)
# ---------------------------------------------------------------------------


#: Correctly rounded f64 for every power of ten the parser can divide by, in
#: the index order serde_json's `POW10` table uses.
_POW10 = [float(f"1e{n}") for n in range(0, 309)]


def serde_f64(text: str) -> float:
    """Parse one JSON number exactly as the kernel's reader does.

    The kernel reads profile documents with `serde_json` *without* its
    `float_roundtrip` feature, which does not parse decimal to the nearest
    double: it converts the digit run to `f64` and then multiplies or divides
    once by a power of ten. That is lossy for long decimals (by one unit in
    the last place), and the recorded bytes were produced through it, so the
    carrier has to reproduce the same value — a correctly rounded parse would
    silently move profile numbers.

    `json.loads(..., parse_float=serde_f64)` hands this hook the literal text
    of every real number in the document.
    """
    body = text.strip()
    sign = 1.0
    if body[:1] in "+-":
        if body[0] == "-":
            sign = -1.0
        body = body[1:]

    exponent = 0
    if "e" in body or "E" in body:
        body, _, exponent_text = body.replace("E", "e").partition("e")
        exponent = int(exponent_text)

    if "." in body:
        integer_part, _, fraction_part = body.partition(".")
    else:
        integer_part, fraction_part = body, ""
    digits = (integer_part + fraction_part).lstrip("0")
    significand = int(digits) if digits else 0
    exponent -= len(fraction_part)

    value = float(significand)
    while True:
        index = abs(exponent)
        if index < len(_POW10):
            if exponent >= 0:
                value *= _POW10[index]
            else:
                value /= _POW10[index]
            break
        if value == 0.0:
            break
        if exponent >= 0:
            raise ValueError(f"number out of range: {text!r}")
        value /= 1e308
        exponent += 308
    return sign * value


def load_json(path: Path) -> Any:
    return json.loads(path.read_text(encoding="utf-8"), parse_float=serde_f64)


def hex_le_u64(text: str) -> int:
    return int.from_bytes(bytes.fromhex(text.strip()), "little")


def hex_le_u32(text: str) -> int:
    raw = bytes.fromhex(text.strip())
    return int.from_bytes(raw[:4], "little")


def rle_f32(pairs: Sequence[Sequence[Any]]) -> list[float]:
    out: list[float] = []
    for pair in pairs:
        value = struct.unpack("<f", struct.pack("<f", float(pair[0])))[0]
        count = json_int(pair[1])
        if count is None or count <= 0:
            raise ValueError(f"tone_rle count is not positive: {pair[1]!r}")
        out.extend([value] * count)
    return out


def mdct_banks(payload: dict[str, Any]) -> list[tuple[int, list[float]]]:
    banks: list[tuple[int, list[float]]] = []
    for key, entry in sorted(payload["profiles"].items(), key=lambda pair: int(pair[0])):
        n = int(key)
        raw = base64.b64decode(entry["little_endian_f32_base64"], validate=True)
        words = struct.unpack(f"<{len(raw) // 4}f", raw)
        expected = n + n // 4
        if len(words) != expected or json_int(entry["word_count"]) != expected:
            raise ValueError(f"mdct bank {n} word count differs")
        banks.append((n, list(words)))
    if [n for n, _ in banks] != [128, 256, 512, 1024, 2048]:
        raise ValueError("mdct bank set is incomplete")
    return banks


def bands_table(rows: Sequence[dict[str, Any]]) -> list[dict[str, Any]]:
    if len(rows) != 12:
        raise ValueError("transient band table must have twelve rows")
    out: list[dict[str, Any]] = []
    for row in rows:
        count = json_int(row["count"])
        weights = row["weights_u32"]
        if count is None or count < 0 or len(weights) != count:
            raise ValueError("transient band row is malformed")
        out.append(
            {
                "offset": json_int(row["offset"]),
                "weights": [u32_masked(w) for w in weights],
                "scale": u32_masked(row["scale_u32"]),
            }
        )
    return out


def frozen_table(payload: dict[str, Any]) -> dict[str, Any]:
    coordinate = [
        (hex_le_u64(pair[0]), hex_le_u64(pair[1])) for pair in payload["coordinate_ln"]
    ]
    twiddles = [
        (int(triple[0]), hex_le_u64(triple[1]), hex_le_u64(triple[2]))
        for triple in payload["fft_twiddles"]
    ]
    halves = [
        (int(size), [hex_le_u32(word) for word in words])
        for size, words in sorted(payload["window_halves"].items(), key=lambda p: int(p[0]))
    ]
    return {"coordinate_ln": coordinate, "fft_twiddles": twiddles, "window_halves": halves}


def quality_curves(payload: dict[str, Any]) -> dict[str, Any]:
    return {
        "breakpoints": [float(v) for v in payload["breakpoints"]],
        "curves": [
            (name, [float(v) for v in values])
            for name, values in sorted(payload["curves"].items())
        ],
        "semantics": [
            (name, payload["semantics"][name]) for name, _ in sorted(payload["curves"].items())
        ],
    }


def short_seed(payload: dict[str, Any]) -> dict[str, Any]:
    profile = payload["profile"]
    geometry = payload["geometry"]
    look = payload["look"]
    tone = rle_f32(payload["tone_rle"])
    if len(tone) != 17 * 8 * 58:
        raise ValueError("short seed tone table is malformed")
    return {
        "sample_rate": json_int(geometry["sample_rate"]),
        "ath_offset": f32_bits(profile["ath_offset"]),
        "ath_floor": f32_bits(profile["ath_floor"]),
        "seed_ceiling": f32_bits(profile["seed_ceiling"]),
        "max_curve_db": f32_bits(profile["max_curve_db"]),
        "curve_offset": f32_bits(profile["curve_offset"]),
        "curve_slope": f32_bits(profile["curve_slope"]),
        "curve_offset_2": f32_bits(profile["curve_offset_2"]),
        "curve_attenuation": [f32_bits(v) for v in profile["curve_attenuation"]],
        "ath": [f32_bits(v) for v in payload["ath"]],
        "octave": [json_int(v) for v in payload["octave"]],
        "first_octave": json_int(geometry["first_octave"]),
        "shift_octave": json_int(geometry["shift_octave"]),
        "eighth_octave_lines": json_int(geometry["eighth_octave_lines"]),
        "total_octave_lines": json_int(geometry["total_octave_lines"]),
        "tone_curves": [f32_bits(v) for v in tone],
        "row": [f32_bits(v) for v in look["row"]],
        "envelope_low": [f32_bits(v) for v in look["envelope_low"]],
        "envelope_high": [f32_bits(v) for v in look["envelope_high"]],
        "mask_curve": [f32_bits(v) for v in look["mask_curve"]],
        "interval_table": [json_int(v) for v in look["interval_table"]],
        "short_limit": json_int(look["short_limit"]),
        "noise_fixed_window": json_int(look["noise_fixed_window"]),
        "regular_curve_bias": f32_bits(look["regular_curve_bias"]),
        "regular_curve_cap": f32_bits(look["regular_curve_cap"]),
        "remap_curve_offsets": [json_int(v) for v in look["remap_curve_offsets"]],
        "remap_low_by_index": [f32_bits(v) for v in look["remap_low_by_index"]],
        "remap_high_by_index": [f32_bits(v) for v in look["remap_high_by_index"]],
    }


def short_profiles(payload: dict[str, Any]) -> list[dict[str, Any]]:
    rows = payload["profiles"]
    if len(rows) != 2:
        raise ValueError("short psychoacoustic profile count differs")
    out: list[dict[str, Any]] = []
    for row in rows:
        curves = [f64_bits(v) for curve in row["mask_curves"] for v in curve]
        out.append(
            {
                "key": str(row["key"]),
                "candidate_bias_by_mode": [f64_bits(v) for v in row["candidate_bias_by_mode"]],
                "curve_cap": f64_bits(row["curve_cap"]),
                "group_enabled": json_int(row["group_enabled"]),
                "candidate_bound": json_int(row["candidate_bound"]),
                "group_span": json_int(row["group_span"]),
                "band_limits": [json_int(v) for v in row["band_limits"]],
                "side_gain": f64_bits(row["side_gain"]),
                "peak_cutoff": json_int(row["peak_cutoff"]),
                "blend_weight": f64_bits(row["blend_weight"]),
                "mask_curves": curves,
            }
        )
    return out


def long_table(payload: dict[str, Any]) -> dict[str, Any]:
    analysis = payload["analysis"]
    seed = payload["seed"]
    banks = [f32_bits(v) for band in seed["tone_banks"] for level in band for v in level]
    curves = [f32_bits(v) for curve in analysis["curves"] for v in curve]
    return {
        "sample_rate": json_int(payload["sample_rate"]),
        "profile_key": str(payload["profile_key"]),
        "analysis_profile_u32": [strict_u32(v) for v in analysis["profile_u32"]],
        "analysis_interval_u32": [strict_u32(v) for v in analysis["interval_u32"]],
        "analysis_curves": curves,
        "analysis_field_19_curve": [f32_bits(v) for v in analysis["field_19_curve"]],
        "seed_outer_u32": [strict_u32(v) for v in seed["outer_u32"]],
        "seed_profile_u32": [strict_u32(v) for v in seed["profile_u32"]],
        "seed_base_curve": [f32_bits(v) for v in seed["base_curve"]],
        "seed_group_labels_u32": [strict_u32(v) for v in seed["group_labels_u32"]],
        "seed_tone_banks": banks,
    }


def long_variants(payload: dict[str, Any]) -> list[dict[str, Any]]:
    out: list[dict[str, Any]] = []
    for mode in sorted(payload["variants"], key=int):
        surface = payload["variants"][mode]
        out.append(
            {
                "mode": int(mode),
                "analysis_profile_u32": [u32_masked(v) for v in surface["profile_u32"]],
                "analysis_interval_u32": [u32_masked(v) for v in surface["interval_u32"]],
                "analysis_curves": [
                    f32_bits(v) for curve in surface["curves"] for v in curve
                ],
                "analysis_field_19_curve": [
                    f32_bits(v) for v in surface["field_19_curve"]
                ],
            }
        )
    if [entry["mode"] for entry in out] != [2, 3]:
        raise ValueError("long analysis variants must cover modes 2 and 3")
    return out


def codebook_rows(payload: Sequence[dict[str, Any]], table: str) -> list[dict[str, Any]]:
    rows: list[dict[str, Any]] = []
    for row in payload:
        if not isinstance(row, dict):
            raise ValueError(f"{table} row is not an object")
        lengthlist = row.get("lengthlist")
        quantlist = row.get("quantlist")
        rows.append(
            {
                "i": None if row.get("i") is None else json_int(row["i"]),
                "dim": json_int(row["dim"]),
                "entries": json_int(row["entries"]),
                "maptype": json_int(row.get("maptype")) or 0,
                "q_min": json_int(row.get("q_min")) or 0,
                "q_delta": json_int(row.get("q_delta")) or 0,
                "q_quant": json_int(row.get("q_quant")) or 0,
                "q_sequencep": json_int(row.get("q_sequencep")) or 0,
                "quantvals": None if row.get("quantvals") is None else json_int(row["quantvals"]),
                "lengthlist": None
                if lengthlist is None
                else [json_int(v) for v in lengthlist],
                "quantlist": None if quantlist is None else [json_int(v) for v in quantlist],
            }
        )
    return rows


# ---------------------------------------------------------------------------
# codebook module emission
# ---------------------------------------------------------------------------

CODEBOOK_COUNTS = {"t97": 97, "t219": 219, "t282": 282}


def codebook_module(table: str, rows: list[dict[str, Any]], header: str) -> str:
    expected = CODEBOOK_COUNTS[table]
    if len(rows) != expected:
        raise ValueError(f"{table} holds {len(rows)} rows, expected {expected}")
    upper = table.upper()
    lines = [header, "use crate::tables::CodebookRowTable;", ""]
    row_entries: list[str] = []
    for index, row in enumerate(rows):
        if row["lengthlist"] is not None:
            name = f"{upper}_LENGTHLIST_{index}"
            lines.extend(
                bytes_literal(name, b"")
                if False
                else _int_array(name, row["lengthlist"]),
            )
            lengthlist = f"Some(&{name})"
        else:
            lengthlist = "None"
        if row["quantlist"] is not None:
            name = f"{upper}_QUANTLIST_{index}"
            lines.extend(_int_array(name, row["quantlist"]))
            quantlist = f"Some(&{name})"
        else:
            quantlist = "None"
        quantvals = "None" if row["quantvals"] is None else f"Some({row['quantvals']})"
        index_field = "None" if row["i"] is None else f"Some({row['i']})"
        row_entries.append(
            "    CodebookRowTable { "
            f"i: {index_field}, dim: {row['dim']}, entries: {row['entries']}, "
            f"maptype: {row['maptype']}, q_min: {row['q_min']}, "
            f"q_delta: {row['q_delta']}, q_quant: {row['q_quant']}, "
            f"q_sequencep: {row['q_sequencep']}, quantvals: {quantvals}, "
            f"lengthlist: {lengthlist}, quantlist: {quantlist} }},"
        )
    lines.append("#[rustfmt::skip]")
    lines.append(f"pub static {upper}_ROWS: [CodebookRowTable; {expected}] = [")
    lines.extend(row_entries)
    lines.append("];")
    lines.append("")
    return "\n".join(lines)


def _int_array(name: str, values: Sequence[int]) -> list[str]:
    lines: list[str] = []
    emit_static(lines, name, f"[i64; {len(values)}]", i64_slice(values))
    return lines


# ---------------------------------------------------------------------------
# per-profile module emission
# ---------------------------------------------------------------------------


def profile_module(
    manifest: dict[str, Any],
    resources: dict[str, Any],
    codebook_modules: dict[str, str],
    header: str,
) -> tuple[str, list[str]]:
    lines: list[str] = []

    key = manifest["key"]
    # No stored profile name: the human label is derived from the key
    # (`ProfileKey::label`), so the carrier holds identity and values only.
    lines.append(f'pub const SETUP_SHA256: &str = "{key["quality_setup_identity"].removeprefix("sha256:")}";')
    lines.append("")
    lines.append("#[rustfmt::skip]")
    lines.append("pub static KEY: ProfileKeyParts = ProfileKeyParts {")
    lines.append(f'    channels: {key["channels"]},')
    lines.append(f'    sample_rate: {key["sample_rate"]},')
    lines.append(f'    generation: "{key["generation"]}",')
    lines.append(f'    channel_layout: "{key["channel_layout"]}",')
    lines.append(f'    quality_setup_identity: "{key["quality_setup_identity"]}",')
    lines.append("};")
    lines.append("")

    lines.extend(bytes_literal("SETUP_PACKET", resources["setup"], public=True))

    # MDCT banks.
    bank_names: list[str] = []
    for n, words in resources["mdct"]:
        array = f"MDCT_TRIG_{n}"
        bank_names.append((n, array))
        lines.append("#[rustfmt::skip]")
        lines.append(f"static {array}: [f32; {len(words)}] = [")
        lines.extend(f32_slice(words))
        lines.append("];")
        lines.append("")
    lines.append("#[rustfmt::skip]")
    lines.append(f"pub static MDCT_BANKS: [MdctBankTable; {len(bank_names)}] = [")
    lines.extend(f"    MdctBankTable {{ n: {n}, trig: &{a} }}," for n, a in bank_names)
    lines.append("];")
    lines.append("")

    # Codebook table selection.
    lines.append("#[rustfmt::skip]")
    lines.append(f"pub static CODEBOOK_TABLES: [CodebookTableRef; {len(resources['codebooks'])}] = [")
    for table in resources["codebooks"]:
        lines.append(
            f'    CodebookTableRef {{ name: "{table}", '
            f'rows: &codebooks::{table}::{table.upper()}_ROWS }},'
        )
    lines.append("];")
    lines.append("")

    # Frozen transcendental tables.
    frozen = resources["frozen"]
    lines.append("#[rustfmt::skip]")
    lines.append(f"static FROZEN_COORDINATE_LN: [(u64, u64); {len(frozen['coordinate_ln'])}] = [")
    lines.extend(
        f"    (0x{in_bits:016x}, 0x{out_bits:016x})," for in_bits, out_bits in frozen["coordinate_ln"]
    )
    lines.append("];")
    lines.append("")
    lines.append("#[rustfmt::skip]")
    lines.append(f"static FROZEN_FFT_TWIDDLES: [(i64, u64, u64); {len(frozen['fft_twiddles'])}] = [")
    lines.extend(
        f"    ({length}, 0x{cos_bits:016x}, 0x{sin_bits:016x}),"
        for length, cos_bits, sin_bits in frozen["fft_twiddles"]
    )
    lines.append("];")
    lines.append("")
    halves_entries: list[str] = []
    for size, words in frozen["window_halves"]:
        array = f"FROZEN_WINDOW_HALVES_{size}"
        lines.append("#[rustfmt::skip]")
        lines.append(f"static {array}: [f32; {len(words)}] = [")
        lines.extend(f32_bits_slice(words))
        lines.append("];")
        lines.append("")
        halves_entries.append(f"    ({size}, &{array}),")
    lines.append("#[rustfmt::skip]")
    lines.append("static FROZEN_WINDOW_HALVES: [(i64, &[f32]); 2] = [")
    lines.extend(halves_entries)
    lines.append("];")
    lines.append("")
    lines.append("#[rustfmt::skip]")
    lines.append("pub static FROZEN: FrozenTable = FrozenTable {")
    lines.append("    coordinate_ln: &FROZEN_COORDINATE_LN,")
    lines.append("    fft_twiddles: &FROZEN_FFT_TWIDDLES,")
    lines.append("    window_halves: &FROZEN_WINDOW_HALVES,")
    lines.append("};")
    lines.append("")

    # Transient tables.
    transient = resources["transient"]
    lines.extend(emit_bands("TRANSIENT_BANDS", transient["bands"]))
    lines.append("#[rustfmt::skip]")
    lines.append(f"static TRANSIENT_WINDOW: [f32; {len(transient['window'])}] = [")
    lines.extend(f32_bits_slice(transient["window"]))
    lines.append("];")
    lines.append("")
    if transient["kind"] == "detector":
        lines.append("#[rustfmt::skip]")
        lines.append(f"static TRANSIENT_CONFIG: [f32; {len(transient['config'])}] = [")
        lines.extend(f32_bits_slice(transient["config"]))
        lines.append("];")
        lines.append("")
        lines.append("#[rustfmt::skip]")
        lines.append("pub static TRANSIENT: TransientTable = TransientTable::Detector(DetectorTable {")
        lines.append(f"    n: {transient['n']},")
        lines.append(f"    bias: f32::from_bits(0x{transient['bias']:08x}),")
        lines.append("    window: &TRANSIENT_WINDOW,")
        lines.append("    config: &TRANSIENT_CONFIG,")
        lines.append("    bands: &TRANSIENT_BANDS,")
        lines.append("});")
        lines.append("")
    else:
        lines.append("#[rustfmt::skip]")
        lines.append("static RECORD_INDEX_CURVE: [f64; 13] = [")
        lines.extend(f64_slice(transient["index_curve"]))
        lines.append("];")
        lines.append("")
        lines.append("#[rustfmt::skip]")
        lines.append("static RECORD_BREAKPOINTS: [f64; 13] = [")
        lines.extend(f64_slice(transient["breakpoints"]))
        lines.append("];")
        lines.append("")
        lines.append("#[rustfmt::skip]")
        lines.append(f"static RECORDS: [RecordTable; {len(transient['records'])}] = [")
        for record in transient["records"]:
            lines.append("    RecordTable {")
            lines.append(f'        file_off: "{record["file_off"]}",')
            lines.append(f"        marker_u32: {record['marker_u32']},")
            lines.append(
                "        upper_u32: [" + ", ".join(str(v) for v in record["upper_u32"]) + "],"
            )
            lines.append(
                "        lower_u32: [" + ", ".join(str(v) for v in record["lower_u32"]) + "],"
            )
            lines.append(f"        carry_u32: {record['carry_u32']},")
            lines.append(f"        bias_u32: {record['bias_u32']},")
            lines.append(f"        m_u32: {record['m_u32']},")
            lines.append(f"        tail_u32: {record['tail_u32']},")
            lines.append(
                "        config_u32: [" + ", ".join(str(v) for v in record["config_u32"]) + "],"
            )
            lines.append("    },")
        lines.append("];")
        lines.append("")
        lines.append("#[rustfmt::skip]")
        lines.append("pub static RECORD_FAMILY: RecordFamilyTable = RecordFamilyTable {")
        lines.append(f"    n: {transient['n']},")
        lines.append(f"    sample_rate: {transient['sample_rate']},")
        lines.append(
            f"    default_record_index: f64::from_bits(0x{f64_bits(transient['default_record_index']):016x}),"
        )
        lines.append("    records: &RECORDS,")
        lines.append("    index_curve: &RECORD_INDEX_CURVE,")
        lines.append("    breakpoints: &RECORD_BREAKPOINTS,")
        lines.append("    window: &TRANSIENT_WINDOW,")
        lines.append("    bands: &TRANSIENT_BANDS,")
        lines.append("};")
        lines.append("")
        lines.append("#[rustfmt::skip]")
        lines.append("pub static TRANSIENT: TransientTable = TransientTable::RecordFamily(&RECORD_FAMILY);")
        lines.append("")

    # Quality curves (optional).
    curves = resources.get("quality_curves")
    if curves is not None:
        lines.append("#[rustfmt::skip]")
        lines.append(f"static QUALITY_CURVE_BREAKPOINTS: [f64; {len(curves['breakpoints'])}] = [")
        lines.extend(f64_slice(curves["breakpoints"]))
        lines.append("];")
        lines.append("")
        curve_names: list[str] = []
        for index, (_, values) in enumerate(curves["curves"]):
            array = f"QUALITY_CURVE_VALUES_{index}"
            curve_names.append(array)
            lines.append("#[rustfmt::skip]")
            lines.append(f"static {array}: [f64; {len(values)}] = [")
            lines.extend(f64_slice(values))
            lines.append("];")
            lines.append("")
        lines.append("#[rustfmt::skip]")
        lines.append(
            f"static QUALITY_CURVE_LIST: [(&str, &[f64]); {len(curves['curves'])}] = ["
        )
        for (name, _), array in zip(curves["curves"], curve_names):
            lines.append(f'    ("{name}", &{array}),')
        lines.append("];")
        lines.append("")
        lines.append("#[rustfmt::skip]")
        lines.append(
            f"static QUALITY_SEMANTICS: [(&str, &str); {len(curves['semantics'])}] = ["
        )
        for name, semantic in curves["semantics"]:
            lines.append(f'    ("{name}", "{semantic}"),')
        lines.append("];")
        lines.append("")
        lines.append("#[rustfmt::skip]")
        lines.append("pub static QUALITY_CURVES: QualityCurvesTable = QualityCurvesTable {")
        lines.append("    breakpoints: &QUALITY_CURVE_BREAKPOINTS,")
        lines.append("    curves: &QUALITY_CURVE_LIST,")
        lines.append("    semantics: &QUALITY_SEMANTICS,")
        lines.append("};")
        lines.append("")

    # Short psychoacoustic seed surface.
    seed = resources["short_seed"]
    for array, field in (
        ("SHORT_SEED_CURVE_ATTENUATION", "curve_attenuation"),
        ("SHORT_SEED_ATH", "ath"),
    ):
        lines.append("#[rustfmt::skip]")
        lines.append(f"static {array}: [f32; {len(seed[field])}] = [")
        lines.extend(f32_slice([struct.unpack("<f", struct.pack("<I", b))[0] for b in seed[field]]))
        lines.append("];")
        lines.append("")
    lines.append("#[rustfmt::skip]")
    lines.append(f"static SHORT_SEED_OCTAVE: [i64; {len(seed['octave'])}] = [")
    lines.extend(i64_slice(seed["octave"]))
    lines.append("];")
    lines.append("")
    for array, field in (
        ("SHORT_SEED_TONE_CURVES", "tone_curves"),
        ("SHORT_SEED_ROW", "row"),
        ("SHORT_SEED_ENVELOPE_LOW", "envelope_low"),
        ("SHORT_SEED_ENVELOPE_HIGH", "envelope_high"),
        ("SHORT_SEED_MASK_CURVE", "mask_curve"),
    ):
        lines.append("#[rustfmt::skip]")
        lines.append(f"static {array}: [f32; {len(seed[field])}] = [")
        lines.extend(f32_slice([struct.unpack("<f", struct.pack("<I", b))[0] for b in seed[field]]))
        lines.append("];")
        lines.append("")
    lines.append("#[rustfmt::skip]")
    lines.append(f"static SHORT_SEED_INTERVAL_TABLE: [i64; {len(seed['interval_table'])}] = [")
    lines.extend(i64_slice(seed["interval_table"]))
    lines.append("];")
    lines.append("")
    lines.append("#[rustfmt::skip]")
    lines.append(f"static SHORT_SEED_REMAP_OFFSETS: [i64; {len(seed['remap_curve_offsets'])}] = [")
    lines.extend(i64_slice(seed["remap_curve_offsets"]))
    lines.append("];")
    lines.append("")
    for array, field in (
        ("SHORT_SEED_REMAP_LOW", "remap_low_by_index"),
        ("SHORT_SEED_REMAP_HIGH", "remap_high_by_index"),
    ):
        lines.append("#[rustfmt::skip]")
        lines.append(f"static {array}: [f32; {len(seed[field])}] = [")
        lines.extend(f32_slice([struct.unpack("<f", struct.pack("<I", b))[0] for b in seed[field]]))
        lines.append("];")
        lines.append("")
    lines.append("#[rustfmt::skip]")
    lines.append("pub static SHORT_SEED: ShortSeedTable = ShortSeedTable {")
    lines.append("    n: 128,")
    lines.append(f"    sample_rate: {seed['sample_rate']},")
    for field in (
        "ath_offset",
        "ath_floor",
        "seed_ceiling",
        "max_curve_db",
        "curve_offset",
        "curve_slope",
        "curve_offset_2",
    ):
        lines.append(f"    {field}: f32::from_bits(0x{seed[field]:08x}),")
    lines.append("    curve_attenuation: &SHORT_SEED_CURVE_ATTENUATION,")
    lines.append("    ath: &SHORT_SEED_ATH,")
    lines.append("    octave: &SHORT_SEED_OCTAVE,")
    for field in ("first_octave", "shift_octave", "eighth_octave_lines", "total_octave_lines"):
        lines.append(f"    {field}: {seed[field]},")
    lines.append("    tone_curves: &SHORT_SEED_TONE_CURVES,")
    lines.append("    row: &SHORT_SEED_ROW,")
    lines.append("    envelope_low: &SHORT_SEED_ENVELOPE_LOW,")
    lines.append("    envelope_high: &SHORT_SEED_ENVELOPE_HIGH,")
    lines.append("    mask_curve: &SHORT_SEED_MASK_CURVE,")
    lines.append("    interval_table: &SHORT_SEED_INTERVAL_TABLE,")
    lines.append(f"    short_limit: {seed['short_limit']},")
    lines.append(f"    noise_fixed_window: {seed['noise_fixed_window']},")
    lines.append(f"    regular_curve_bias: f32::from_bits(0x{seed['regular_curve_bias']:08x}),")
    lines.append(f"    regular_curve_cap: f32::from_bits(0x{seed['regular_curve_cap']:08x}),")
    lines.append("    remap_curve_offsets: &SHORT_SEED_REMAP_OFFSETS,")
    lines.append("    remap_low_by_index: &SHORT_SEED_REMAP_LOW,")
    lines.append("    remap_high_by_index: &SHORT_SEED_REMAP_HIGH,")
    lines.append("};")
    lines.append("")

    # Short psychoacoustic profiles (f64, exactly as the loader stores them).
    short_rows = resources["short_profiles"]
    for index, row in enumerate(short_rows):
        array = f"SHORT_PROFILE_{index}_MASK_CURVES"
        lines.append("#[rustfmt::skip]")
        lines.append(f"static {array}: [f64; {len(row['mask_curves'])}] = [")
        lines.extend(
            f64_slice([struct.unpack("<d", struct.pack("<Q", b))[0] for b in row["mask_curves"]])
        )
        lines.append("];")
        lines.append("")
    lines.append("#[rustfmt::skip]")
    lines.append(f"static SHORT_PROFILES: [ShortProfileTable; {len(short_rows)}] = [")
    for index, row in enumerate(short_rows):
        lines.append("    ShortProfileTable {")
        lines.append(f'        key: "{row["key"]}",')
        lines.append(
            "        candidate_bias_by_mode: &["
            + ", ".join(f"f64::from_bits(0x{b:016x})" for b in row["candidate_bias_by_mode"])
            + "],"
        )
        lines.append(f"        curve_cap: f64::from_bits(0x{row['curve_cap']:016x}),")
        for field in ("group_enabled", "candidate_bound", "group_span"):
            lines.append(f"        {field}: {row[field]},")
        lines.append(
            "        band_limits: ["
            + ", ".join(str(v) for v in row["band_limits"])
            + "],"
        )
        lines.append(f"        side_gain: f64::from_bits(0x{row['side_gain']:016x}),")
        lines.append(f"        peak_cutoff: {row['peak_cutoff']},")
        lines.append(f"        blend_weight: f64::from_bits(0x{row['blend_weight']:016x}),")
        lines.append(f"        mask_curves: &SHORT_PROFILE_{index}_MASK_CURVES,")
        lines.append("    },")
    lines.append("];")
    lines.append("")

    # Long psychoacoustic tables.
    long_base = resources["long_base"]
    lines.extend(emit_long_arrays("LONG_BASE", long_base))
    lines.append("#[rustfmt::skip]")
    lines.append("pub static LONG_BASE: LongTable = LongTable {")
    lines.append(f"    sample_rate: {long_base['sample_rate']},")
    lines.append("    n: 1024,")
    lines.append(f'    profile_key: "{long_base["profile_key"]}",')
    for field in (
        "analysis_profile_u32",
        "analysis_interval_u32",
        "analysis_curves",
        "analysis_field_19_curve",
        "seed_outer_u32",
        "seed_profile_u32",
        "seed_base_curve",
        "seed_group_labels_u32",
        "seed_tone_banks",
    ):
        lines.append(f"    {field}: &LONG_BASE_{field.upper()},")
    lines.append("};")
    lines.append("")

    variants = resources["long_variants"]
    for variant in variants:
        lines.extend(emit_long_arrays(f"LONG_MODE_{variant['mode']}", variant))
    lines.append("#[rustfmt::skip]")
    lines.append(f"pub static LONG_VARIANTS: [LongVariantTable; {len(variants)}] = [")
    for variant in variants:
        lines.append("    LongVariantTable {")
        lines.append(f"        mode: {variant['mode']},")
        lines.append(f"        analysis_profile_u32: &LONG_MODE_{variant['mode']}_ANALYSIS_PROFILE_U32,")
        lines.append(
            f"        analysis_interval_u32: &LONG_MODE_{variant['mode']}_ANALYSIS_INTERVAL_U32,"
        )
        lines.append(f"        analysis_curves: &LONG_MODE_{variant['mode']}_ANALYSIS_CURVES,")
        lines.append(
            f"        analysis_field_19_curve: &LONG_MODE_{variant['mode']}_ANALYSIS_FIELD_19_CURVE,"
        )
        lines.append("    },")
    lines.append("];")
    lines.append("")

    # Input conditioner (optional).
    conditioner = resources.get("input_conditioner")

    # Aggregate.
    lines.append("#[rustfmt::skip]")
    lines.append("pub static TABLES: ProfileTables = ProfileTables {")
    lines.append("    key: KEY,")
    lines.append("    setup_packet: &SETUP_PACKET,")
    lines.append("    setup_sha256: SETUP_SHA256,")
    lines.append("    container_metadata: ContainerMetadata {")
    for json_field, rust_field in CONTAINER_FIELDS:
        lines.append(f"        {rust_field}: {manifest['container_metadata'][json_field]},")
    lines.append("    },")
    lines.append(f"    block_sizes: [{manifest['block_sizes'][0]}, {manifest['block_sizes'][1]}],")
    lines.append("    resources: ResourceTables {")
    lines.append("        mdct_banks: &MDCT_BANKS,")
    lines.append("        codebook_tables: &CODEBOOK_TABLES,")
    lines.append("        frozen: &FROZEN,")
    lines.append("        transient: TRANSIENT,")
    lines.append(
        "        quality_curves: None,"
        if curves is None
        else "        quality_curves: Some(&QUALITY_CURVES),"
    )
    lines.append("        short_seed: &SHORT_SEED,")
    lines.append("        short_profiles: &SHORT_PROFILES,")
    lines.append("        long_base: &LONG_BASE,")
    lines.append("        long_variants: &LONG_VARIANTS,")
    lines.append(
        "        input_conditioner: None,"
        if conditioner is None
        else f"        input_conditioner: Some(0x{conditioner:08x}),"
    )
    lines.append("    },")
    lines.append("};")
    lines.append("")

    return header + "\n" + _table_imports("\n".join(lines)) + "\n" + "\n".join(lines), []


#: Type names the generated modules borrow from `crate::tables` /
#: `crate::model`. Only the ones a module actually spells are imported, so a
#: profile that has no quality curves never carries an unused import.
TABLE_TYPES = (
    "BandTable",
    "CodebookRowTable",
    "CodebookTableRef",
    "DetectorTable",
    "FrozenTable",
    "LongTable",
    "LongVariantTable",
    "MdctBankTable",
    "ProfileKeyParts",
    "ProfileTables",
    "QualityCurvesTable",
    "RecordFamilyTable",
    "RecordTable",
    "ResourceTables",
    "ShortProfileTable",
    "ShortSeedTable",
    "TransientTable",
)


def _table_imports(body: str) -> str:
    import re

    tables = sorted(
        name
        for name in TABLE_TYPES
        if name != "CodebookRowTable" and re.search(rf"\b{name}\b", body)
    )
    model = ["ContainerMetadata"] if re.search(r"\bContainerMetadata\b", body) else []
    # rustfmt's import order: `super` first, then `crate::model`, then
    # `crate::tables`.
    lines: list[str] = ["use super::codebooks;"]
    if model:
        lines.append("use crate::model::ContainerMetadata;")
    if tables:
        lines.append("use crate::tables::{")
        lines.extend(_rows([f"{name}," for name in tables], indent="    "))
        lines.append("};")
    return "\n".join(lines)


def _snake_field(field: str) -> str:
    out: list[str] = []
    for index, char in enumerate(field):
        if char.isupper() and index:
            out.append("_")
        out.append(char.lower())
    return "".join(out)


#: `ContainerMetadata` field order and spelling, taken from the kernel's own
#: `model::ContainerMetadata::FIELDS` list. The manifest JSON keys are the
#: same as the kernel's `FIELDS` entries; the Rust fields are spelled here so
#: the generator never has to guess a name from a JSON key.
CONTAINER_FIELDS: tuple[tuple[str, str], ...] = (
    ("wFormatTag", "w_format_tag"),
    ("nChannels", "n_channels"),
    ("nSamplesPerSec", "n_samples_per_sec"),
    ("nAvgBytesPerSec", "n_avg_bytes_per_sec"),
    ("nBlockAlign", "n_block_align"),
    ("wBitsPerSample", "w_bits_per_sample"),
    ("cbSize", "cb_size"),
    ("wReserved0", "w_reserved0"),
    ("dwChannelMask", "dw_channel_mask"),
    ("dwTotalPCMFrames", "dw_total_pcm_frames"),
    ("dwFirstAudioPacketOffset", "dw_first_audio_packet_offset"),
    ("dwDataPayloadSize", "dw_data_payload_size"),
    ("dwUnknown_0x24", "dw_unknown_0x24"),
    ("dwSeekTableSize", "dw_seek_table_size"),
    ("dwVorbisDataOffset", "dw_vorbis_data_offset"),
    ("uMaxPacketSize", "u_max_packet_size"),
    ("uUnknown_0x32", "u_unknown_0x32"),
    ("dwUnknown_0x34", "dw_unknown_0x34"),
    ("dwUnknown_0x38", "dw_unknown_0x38"),
    ("dwUnknown_0x3C", "dw_unknown_0x3c"),
    ("uBlocksize0Pow", "u_blocksize0_pow"),
    ("uBlocksize1Pow", "u_blocksize1_pow"),
)


def emit_bands(name: str, bands: Sequence[dict[str, Any]]) -> list[str]:
    lines: list[str] = []
    entries: list[str] = []
    for index, band in enumerate(bands):
        array = f"{name}_{index}_WEIGHTS"
        lines.append("#[rustfmt::skip]")
        lines.append(f"static {array}: [f32; {len(band['weights'])}] = [")
        lines.extend(f32_bits_slice(band["weights"]))
        lines.append("];")
        lines.append("")
        entries.append(
            f"    BandTable {{ offset: {band['offset']}, weights: &{array}, "
            f"scale: f32::from_bits(0x{band['scale']:08x}) }},"
        )
    lines.append("#[rustfmt::skip]")
    lines.append(f"static {name}: [BandTable; {len(bands)}] = [")
    lines.extend(entries)
    lines.append("];")
    lines.append("")
    return lines


def emit_long_arrays(prefix: str, table: dict[str, Any], prefix_suffix: bool = True) -> list[str]:
    lines: list[str] = []
    for field in (
        "analysis_profile_u32",
        "analysis_interval_u32",
        "analysis_curves",
        "analysis_field_19_curve",
        "seed_outer_u32",
        "seed_profile_u32",
        "seed_base_curve",
        "seed_group_labels_u32",
        "seed_tone_banks",
    ):
        if field not in table:
            continue
        values = table[field]
        array = f"{prefix}_{field.upper()}"
        if field.endswith("_u32"):
            lines.append("#[rustfmt::skip]")
            lines.append(f"static {array}: [u32; {len(values)}] = [")
            lines.extend(u32_slice(values))
            lines.append("];")
        else:
            lines.append("#[rustfmt::skip]")
            lines.append(f"static {array}: [f32; {len(values)}] = [")
            lines.extend(
                f32_slice([struct.unpack("<f", struct.pack("<I", b))[0] for b in values])
            )
            lines.append("];")
        lines.append("")
    return lines


# ---------------------------------------------------------------------------
# driver
# ---------------------------------------------------------------------------


def tree_digest(profiles_dir: Path) -> str:
    digest = hashlib.sha256()
    for path in sorted(p for p in profiles_dir.rglob("*") if p.is_file()):
        relative = path.relative_to(profiles_dir).as_posix()
        digest.update(relative.encode("utf-8"))
        digest.update(b"\0")
        digest.update(hashlib.sha256(path.read_bytes()).hexdigest().encode("ascii"))
        digest.update(b"\n")
    return digest.hexdigest()


def resource_path(profile_dir: Path, manifest: dict[str, Any], logical: str) -> Path | None:
    entry = manifest["resources"].get(logical)
    if entry is None:
        return None
    return profile_dir / entry["path"]


def build(profiles_dir: Path, out_dir: Path) -> dict[Path, str]:
    index = load_json(profiles_dir / "index.json")
    digest = tree_digest(profiles_dir)
    header = HEADER.format(script=GENERATED_BY, digest=digest)

    files: dict[Path, str] = {}
    profile_names = sorted(index["profiles"])
    modules: list[str] = []
    codebooks_seen: dict[str, str] = {}
    profile_sources: dict[str, str] = {}

    for name in profile_names:
        entry = index["profiles"][name]
        manifest = load_json(profiles_dir / entry["manifest"])
        if manifest["name"] != name:
            raise ValueError(f"profile {name}: manifest name differs")
        profile_dir = (profiles_dir / entry["manifest"]).parent

        setup_path = resource_path(profile_dir, manifest, "vorbis.setup")
        if setup_path is None:
            raise ValueError(f"profile {name}: no setup packet")
        resources: dict[str, Any] = {
            "setup": setup_path.read_bytes(),
            "mdct": mdct_banks(load_json(resource_path(profile_dir, manifest, "transform.mdct"))),
            "codebooks": [],
        }

        for table, logical in (
            ("t97", "vorbis.codebooks.t97"),
            ("t219", "vorbis.codebooks.t219"),
            ("t282", "vorbis.codebooks.t282"),
        ):
            path = resource_path(profile_dir, manifest, logical)
            if path is None:
                continue
            rows = codebook_rows(load_json(path), table)
            module = codebook_module(table, rows, header)
            if table in codebooks_seen and codebooks_seen[table] != module:
                raise ValueError(f"codebook table {table} differs between profiles")
            codebooks_seen[table] = module
            resources["codebooks"].append(table)

        resources["frozen"] = frozen_table(
            load_json(resource_path(profile_dir, manifest, "analysis.frozen-tables"))
        )

        transient_path = resource_path(profile_dir, manifest, "analysis.transient")
        transient = load_json(transient_path)
        if transient["schema"] == "wem.transient-detector-table.v1":
            resources["transient"] = {
                "kind": "detector",
                "n": json_int(transient["n"]),
                "bias": u32_masked(transient["bias_u32"]),
                "window": [u32_masked(v) for v in transient["window_u32"]],
                "config": [u32_masked(v) for v in transient["config_u32"]],
                "bands": bands_table(transient["bands"]),
            }
        elif transient["schema"] == "wem.transient-record-family.v1":
            family = transient["record_family"]
            records = []
            for record in family["records"]:
                records.append(
                    {
                        "file_off": record["file_off"],
                        "marker_u32": u32_masked(record["marker_u32"]),
                        "upper_u32": [u32_masked(v) for v in record["upper_u32"]],
                        "lower_u32": [u32_masked(v) for v in record["lower_u32"]],
                        "carry_u32": u32_masked(record["carry_u32"]),
                        "bias_u32": u32_masked(record["bias_u32"]),
                        "m_u32": u32_masked(record["m_u32"]),
                        "tail_u32": u32_masked(record["tail_u32"]),
                        "config_u32": [u32_masked(v) for v in record["config_u32"]],
                    }
                )
            if len(records) != json_int(family["count"]):
                raise ValueError(f"profile {name}: record family count differs")
            resources["transient"] = {
                "kind": "record-family",
                "n": json_int(transient["n"]),
                "sample_rate": json_int(transient["sample_rate"]),
                "default_record_index": float(transient["default_record_index"]),
                "records": records,
                "index_curve": [float(v) for v in transient["record_index_curve"]["values"]],
                "breakpoints": [
                    float(v) for v in transient["quality_axis_breakpoints"]["values"]
                ],
                "window": [u32_masked(v) for v in transient["window_u32"]],
                "bands": bands_table(transient["bands"]),
            }
        else:
            raise ValueError(f"profile {name}: unknown transient schema")

        curves_path = resource_path(profile_dir, manifest, "analysis.quality-curves")
        if curves_path is not None:
            resources["quality_curves"] = quality_curves(load_json(curves_path))

        resources["short_seed"] = short_seed(
            load_json(resource_path(profile_dir, manifest, "psychoacoustics.short-seed"))
        )
        resources["short_profiles"] = short_profiles(
            load_json(resource_path(profile_dir, manifest, "psychoacoustics.short-profiles"))
        )
        resources["long_base"] = long_table(
            load_json(resource_path(profile_dir, manifest, "psychoacoustics.long-base"))
        )
        resources["long_variants"] = long_variants(
            load_json(resource_path(profile_dir, manifest, "psychoacoustics.long-modes"))
        )
        conditioner_path = resource_path(profile_dir, manifest, "analysis.input-conditioner")
        if conditioner_path is not None:
            resources["input_conditioner"] = strict_u32(
                load_json(conditioner_path)["dc_filter_coefficient_f32_bits"]
            )

        module = name.replace("-", "_").replace(".", "_")
        modules.append(module)
        body, _ = profile_module(manifest, resources, codebooks_seen, header)
        profile_sources[module] = body

    # Codebook modules are shared: emit one per table, deduplicated by content.
    codebook_module_names = sorted(codebooks_seen)
    codebook_mod = "\n".join(
        [
            header,
            "//!",
            "//! Static Wwise Vorbis codebook tables, one module per installed table.",
            "",
            *[f"pub mod {table};" for table in codebook_module_names],
            "",
        ]
    )
    files[out_dir / "codebooks" / "mod.rs"] = codebook_mod
    for table, text in codebooks_seen.items():
        files[out_dir / "codebooks" / f"{table}.rs"] = text

    for module, body in profile_sources.items():
        files[out_dir / f"{module}.rs"] = body

    mod_lines = [
        header,
        "//!",
        "//! The compiled carrier: one typed table set per installed profile.",
        "",
        "pub mod codebooks;",
        *[f"pub mod {module};" for module in modules],
        "",
        "use crate::tables::ProfileTables;",
        "",
        "/// Every installed profile's typed tables, in deterministic (sorted)",
        "/// order.",
        "#[rustfmt::skip]",
        f"pub static PROFILES: [&ProfileTables; {len(modules)}] = [",
        *[f"    &{module}::TABLES," for module in modules],
        "];",
        "",
    ]
    files[out_dir / "mod.rs"] = "\n".join(mod_lines)
    return files


def write(files: dict[Path, str], out_dir: Path) -> None:
    for path, text in sorted(files.items()):
        path.parent.mkdir(parents=True, exist_ok=True)
        path.write_text(text, encoding="utf-8")


def main(argv: list[str]) -> int:
    parser = argparse.ArgumentParser(
        prog="generate_profile_code.py",
        description=(
            "Record the encoder profile tree as Rust typed tables under "
            "crates/wem-profiles/src/generated/.\n"
            "It needs the profile tree: the recorded per-profile resource "
            "documents the kernel used to ship as package data. They are "
            "development material; the tree's untracked home is "
            "corpus/profiles/ at the repository root, and --profiles-dir "
            f"selects any other location (default: {DEFAULT_PROFILES_DIR})."
        ),
        formatter_class=argparse.RawDescriptionHelpFormatter,
    )
    parser.add_argument(
        "--profiles-dir",
        type=Path,
        default=DEFAULT_PROFILES_DIR,
        help=(
            "profile tree holding index.json and one directory per profile "
            f"(default: {DEFAULT_PROFILES_DIR})"
        ),
    )
    parser.add_argument(
        "--out-dir",
        type=Path,
        default=DEFAULT_OUT_DIR,
        help=f"generated Rust module tree (default: {DEFAULT_OUT_DIR})",
    )
    parser.add_argument(
        "--check",
        action="store_true",
        help="do not write; fail when the tracked tables differ from the input",
    )
    args = parser.parse_args(argv)

    profiles_dir: Path = args.profiles_dir
    out_dir: Path = args.out_dir
    if not (profiles_dir / "index.json").is_file():
        print(f"profile tree missing: {profiles_dir / 'index.json'}", file=sys.stderr)
        return 2

    files = build(profiles_dir, out_dir)

    if args.check:
        stale = [
            path
            for path, text in sorted(files.items())
            if not path.is_file() or path.read_text(encoding="utf-8") != text
        ]
        known = {path for path in out_dir.rglob("*.rs")} if out_dir.is_dir() else set()
        for extra in sorted(known - set(files)):
            stale.append(extra)
        for path in stale:
            print(f"stale generated table: {path}", file=sys.stderr)
        return 1 if stale else 0

    write(files, out_dir)
    total = sum(len(text) for text in files.values())
    print(f"{len(files)} generated modules, {total} bytes, tree digest {tree_digest(profiles_dir)}")
    return 0


if __name__ == "__main__":
    raise SystemExit(main(sys.argv[1:]))
