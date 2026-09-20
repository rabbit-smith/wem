#!/usr/bin/env python3
"""Immutable window and band configuration for transient detection.

Two resource schemas are served from the same logical manifest entry
(``analysis.transient``):

* ``wem.transient-detector-table.v1`` — a pre-materialized static detector
  table (the historical shape, still used by profiles that register one);
* ``wem.transient-record-family.v1`` — the static bias/threshold record
  family from the paired encoder build. The assembly layer materializes one
  detector table from it per quality value: the record-index curve on the
  shared quality axis picks a (possibly fractional) record index, the floor
  record supplies every field verbatim, and only upper[0..3]/lower[0..3] are
  linearly interpolated between the adjacent records at the fractional part.
"""
from __future__ import annotations

import math
import struct
from dataclasses import dataclass
from functools import lru_cache

from ..analysis.config import (
    CALIBRATION_SAMPLE_RATES,
    SHORT_PSYCH_ACOUSTIC_N,
    TransientBandConfig,
    TransientDetectorTables,
)
from .quality import _linear_frac, normalize_quality_factor
from wwise_wem.profiles.resources import ResourceRef


def _u32_f32(value: int) -> float:
    return struct.unpack("<f", struct.pack("<I", int(value) & 0xFFFFFFFF))[0]


@lru_cache(maxsize=None)
def load_transient_tables(ref: ResourceRef) -> TransientDetectorTables:
    """Load the checked n=128 static detector table."""
    if not isinstance(ref, ResourceRef):
        raise TypeError("transient resource must be ResourceRef")
    data = ref.read_json()
    if data.get("schema") != "wem.transient-detector-table.v1":
        raise ValueError("transient table schema changed")
    if (
        data.get("sample_rate") not in CALIBRATION_SAMPLE_RATES
        or data.get("n") != SHORT_PSYCH_ACOUSTIC_N
    ):
        raise ValueError("transient table geometry changed")
    n = data.get("n")
    window_u32 = data.get("window_u32")
    config_u32 = data.get("config_u32")
    rows = data.get("bands")
    if not isinstance(window_u32, list) or len(window_u32) != n:
        raise ValueError("transient window must contain 128 float words")
    if not isinstance(config_u32, list) or len(config_u32) != 26:
        raise ValueError("transient config must contain 26 float words")
    if not isinstance(rows, list) or len(rows) != 12:
        raise ValueError("transient descriptor table must contain twelve bands")
    bands: list[TransientBandConfig] = []
    for row in rows:
        weights_u32 = row.get("weights_u32")
        count = row.get("count")
        if (
            not isinstance(count, int)
            or not isinstance(weights_u32, list)
            or len(weights_u32) != count
            or not isinstance(row.get("offset"), int)
            or not isinstance(row.get("scale_u32"), int)
        ):
            raise ValueError("transient descriptor row is malformed")
        bands.append(TransientBandConfig(
            offset=int(row["offset"]),
            weights=tuple(_u32_f32(value) for value in weights_u32),
            scale=_u32_f32(int(row["scale_u32"])),
        ))
    return TransientDetectorTables(
        n=n,
        bias=_u32_f32(int(data["bias_u32"])),
        window=tuple(_u32_f32(value) for value in window_u32),
        config=tuple(_u32_f32(value) for value in config_u32),
        bands=tuple(bands),
    )


# ---------------------------------------------------------------------------
# wem.transient-record-family.v1: the static record library from the paired
# encoder build, materialized per quality.
#
# Selection/interpolation mechanism (instruction-pinned by the the extraction
# extraction, the internal record, the corresponding locations): the whole record
# at v5 = floor(index) is copied into the runtime block first; only
# upper[0..3] and lower[0..3] are then overwritten with a linear
# interpolation between records v5 and v5+1 at the fractional part. So all
# non-interpolated fields (bias, carry, marker, tail, upper[4..11],
# lower[4..11]) come verbatim from the FLOOR record. The floor rule matches
# the write-address evidence; it is the adjudicated selection semantics for
# non-integer indices (the STAGE0 replay confirmed FLOOR behavior is the
# consistent reading of the paired build).
# ---------------------------------------------------------------------------

TRANSIENT_RECORD_FAMILY_SCHEMA = "wem.transient-record-family.v1"
TRANSIENT_RECORD_COUNT = 6
TRANSIENT_RECORD_WORDS = 26
TRANSIENT_RECORD_INDEX_POINTS = 13
TRANSIENT_WINDOW_WORDS = SHORT_PSYCH_ACOUSTIC_N
TRANSIENT_BAND_COUNT = 12


@dataclass(frozen=True)
class TransientRecord:
    """One stored bias/threshold record (u32 bit patterns, byte-exact)."""

    file_off: str
    marker_u32: int
    upper_u32: tuple[int, ...]
    lower_u32: tuple[int, ...]
    carry_u32: int
    bias_u32: int
    m_u32: int
    tail_u32: int
    config_u32: tuple[int, ...]

    def __post_init__(self) -> None:
        if not isinstance(self.file_off, str) or not self.file_off:
            raise ValueError("transient record file_off must be a string")
        if self.marker_u32 != 8:
            raise ValueError("transient record marker must be 8")
        if len(self.upper_u32) != 12 or len(self.lower_u32) != 12:
            raise ValueError("transient record needs 12 upper and 12 lower words")
        if len(self.config_u32) != TRANSIENT_RECORD_WORDS:
            raise ValueError("transient record config must hold 26 words")
        if tuple(self.config_u32[1:13]) != self.upper_u32 or tuple(
            self.config_u32[13:25]
        ) != self.lower_u32:
            raise ValueError("transient record fields disagree with config words")
        if self.config_u32[0] != self.marker_u32 or self.config_u32[25] != self.carry_u32:
            raise ValueError("transient record marker/carry disagree with config words")


@dataclass(frozen=True)
class TransientRecordFamily:
    """The static record library plus its immutable detector surfaces."""

    schema: str
    n: int
    sample_rate: int
    default_record_index: float
    records: tuple[TransientRecord, ...]
    index_curve: tuple[float, ...]
    breakpoints: tuple[float, ...]
    window_u32: tuple[int, ...]
    bands: tuple[TransientBandConfig, ...]

    def __post_init__(self) -> None:
        object.__setattr__(self, "records", tuple(self.records))
        object.__setattr__(self, "index_curve", tuple(float(v) for v in self.index_curve))
        object.__setattr__(self, "breakpoints", tuple(float(v) for v in self.breakpoints))
        object.__setattr__(self, "window_u32", tuple(int(v) for v in self.window_u32))
        object.__setattr__(self, "bands", tuple(self.bands))


def load_transient_record_family(ref: ResourceRef) -> TransientRecordFamily:
    """Load and validate the record-family resource."""
    if not isinstance(ref, ResourceRef):
        raise TypeError("transient resource must be ResourceRef")
    data = ref.read_json()
    if data.get("schema") != TRANSIENT_RECORD_FAMILY_SCHEMA:
        raise ValueError("transient record-family schema changed")
    if (
        data.get("sample_rate") not in CALIBRATION_SAMPLE_RATES
        or data.get("n") != SHORT_PSYCH_ACOUSTIC_N
    ):
        raise ValueError("transient record-family geometry changed")

    def strict_u32_list(container: dict, key: str, length: int) -> list[int]:
        values = container.get(key)
        if not isinstance(values, list) or len(values) != length or any(
            isinstance(v, bool) or not isinstance(v, int) for v in values
        ):
            raise ValueError(f"transient record-family {key} is malformed")
        return [int(v) for v in values]

    def f64_list(key: str, length: int) -> list[float]:
        block = data.get(key)
        if not isinstance(block, dict):
            raise ValueError(f"transient record-family {key} is malformed")
        values = block.get("values")
        if not isinstance(values, list) or len(values) != length or any(
            isinstance(v, bool) or not isinstance(v, (int, float)) for v in values
        ):
            raise ValueError(f"transient record-family {key} is malformed")
        result = [float(v) for v in values]
        if not all(math.isfinite(v) for v in result):
            raise ValueError(f"transient record-family {key} must be finite")
        return result

    family = data.get("record_family")
    if not isinstance(family, dict) or family.get("count") != TRANSIENT_RECORD_COUNT:
        raise ValueError("transient record-family must carry six records")
    raw_records = family.get("records")
    if not isinstance(raw_records, list) or len(raw_records) != TRANSIENT_RECORD_COUNT:
        raise ValueError("transient record-family must carry six records")
    records: list[TransientRecord] = []
    for index, raw in enumerate(raw_records):
        if not isinstance(raw, dict):
            raise ValueError(f"transient record {index} is malformed")
        try:
            records.append(
                TransientRecord(
                    file_off=str(raw["file_off"]),
                    marker_u32=int(raw["marker_u32"]),
                    upper_u32=tuple(strict_u32_list(raw, "upper_u32", 12)),
                    lower_u32=tuple(strict_u32_list(raw, "lower_u32", 12)),
                    carry_u32=int(raw["carry_u32"]),
                    bias_u32=int(raw["bias_u32"]),
                    m_u32=int(raw["m_u32"]),
                    tail_u32=int(raw["tail_u32"]),
                    config_u32=tuple(strict_u32_list(raw, "config_u32", TRANSIENT_RECORD_WORDS)),
                )
            )
        except (KeyError, TypeError, ValueError) as error:
            raise ValueError(f"transient record {index} is malformed") from error

    index_curve = f64_list("record_index_curve", TRANSIENT_RECORD_INDEX_POINTS)
    breakpoints = f64_list("quality_axis_breakpoints", TRANSIENT_RECORD_INDEX_POINTS)
    if any(
        breakpoints[i] >= breakpoints[i + 1]
        for i in range(len(breakpoints) - 1)
    ):
        raise ValueError("transient record-family breakpoints must be increasing")
    if any(not (0.0 <= v <= float(TRANSIENT_RECORD_COUNT - 1)) for v in index_curve):
        raise ValueError("transient record-index curve leaves the record domain")

    window_u32 = strict_u32_list(data, "window_u32", TRANSIENT_WINDOW_WORDS)
    rows = data.get("bands")
    if not isinstance(rows, list) or len(rows) != TRANSIENT_BAND_COUNT:
        raise ValueError("transient record-family bands are malformed")
    bands: list[TransientBandConfig] = []
    for index, row in enumerate(rows):
        if not isinstance(row, dict):
            raise ValueError(f"transient record-family band {index} is malformed")
        weights = row.get("weights_u32")
        count = row.get("count")
        if (
            isinstance(count, bool)
            or not isinstance(count, int)
            or not isinstance(weights, list)
            or len(weights) != count
            or isinstance(row.get("offset"), bool)
            or not isinstance(row.get("offset"), int)
            or isinstance(row.get("scale_u32"), bool)
            or not isinstance(row.get("scale_u32"), int)
            or any(isinstance(value, bool) or not isinstance(value, int) for value in weights)
        ):
            raise ValueError(f"transient record-family band {index} is malformed")
        bands.append(
            TransientBandConfig(
                offset=int(row["offset"]),
                weights=tuple(_u32_f32(value) for value in weights),
                scale=_u32_f32(int(row["scale_u32"])),
            )
        )

    raw_default = data.get("default_record_index")
    if isinstance(raw_default, bool) or not isinstance(raw_default, (int, float)):
        raise ValueError("transient record-family default_record_index is malformed")
    default_index = float(raw_default)
    if not math.isfinite(default_index) or not (
        0.0 <= default_index <= float(TRANSIENT_RECORD_COUNT - 1)
    ):
        raise ValueError("transient record-family default_record_index is out of domain")

    return TransientRecordFamily(
        schema=TRANSIENT_RECORD_FAMILY_SCHEMA,
        n=int(data["n"]),
        sample_rate=int(data["sample_rate"]),
        default_record_index=default_index,
        records=tuple(records),
        index_curve=tuple(index_curve),
        breakpoints=tuple(breakpoints),
        window_u32=tuple(window_u32),
        bands=tuple(bands),
    )


def _f32(value: float) -> float:
    return struct.unpack("<f", struct.pack("<f", float(value)))[0]


def _f32_bits(value: float) -> int:
    return struct.unpack("<I", struct.pack("<f", float(value)))[0]


def materialize_transient_tables(
    family: TransientRecordFamily,
    quality: float | None,
) -> TransientDetectorTables:
    """Materialize one detector table from the record family for one quality.

    ``quality`` is the raw Wwise quality factor (0-10 convention) or ``None``
    for the profile's historical default gear: ``None`` materializes the
    family's ``default_record_index`` record whole (the adjudicated default
    is record 3, the High band of the paired build). A quality value is
    normalized onto the shared quality axis, read on the family's own
    record-index curve with the shared two-step linear kernel, and mapped
    onto the family by the pinned selection rule:

    * ``v5 = floor(index)`` (clamped to the last record at the top);
    * every field except upper[0..3]/lower[0..3] comes verbatim from
      record ``v5`` (bias, carry, marker, tail, upper[4..11], lower[4..11]);
    * upper[0..3]/lower[0..3] are linearly interpolated between records
      ``v5`` and ``v5+1`` at the fractional part (f64 lerp, f32 rounding).

    The window and band surfaces are quality-independent static f32 words from
    the paired build. Materialization only selects/interpolates record fields.
    """
    if quality is None:
        index = family.default_record_index
    else:
        if not math.isfinite(quality):
            raise ValueError("quality must be a finite number")
        index, _outside = _linear_frac(
            family.breakpoints, family.index_curve, normalize_quality_factor(quality)
        )
    index = max(0.0, min(float(len(family.records) - 1), index))
    v5 = int(math.floor(index))
    if v5 >= len(family.records) - 1:
        v5 = len(family.records) - 1
        frac = 0.0
    else:
        frac = index - v5

    base = family.records[v5]
    # Rebuild the float config view directly from the u32 words:
    config_words: list[int] = list(base.config_u32)
    if frac > 0.0:
        nxt = family.records[v5 + 1].config_u32
        for j in range(4):
            for position in (1 + j, 13 + j):
                a = _u32_f32(config_words[position])
                b = _u32_f32(nxt[position])
                config_words[position] = _f32_bits((1.0 - frac) * a + frac * b)

    return TransientDetectorTables(
        n=family.n,
        bias=_u32_f32(base.bias_u32),
        window=tuple(_u32_f32(word) for word in family.window_u32),
        config=tuple(_u32_f32(word) for word in config_words),
        bands=family.bands,
    )


def load_transient_resource(
    ref: ResourceRef,
    quality: float | None = None,
) -> TransientDetectorTables:
    """Dispatch on the resource schema and load/materialize the tables.

    ``wem.transient-detector-table.v1`` keeps the historical path exactly
    (profiles that register a pre-materialized table, quality ignored);
    ``wem.transient-record-family.v1`` materializes per quality (the paired
    build's static mechanism).
    """
    data = ref.read_json()
    schema = data.get("schema")
    if schema == "wem.transient-detector-table.v1":
        return load_transient_tables(ref)
    if schema == TRANSIENT_RECORD_FAMILY_SCHEMA:
        return materialize_transient_tables(
            load_transient_record_family(ref), quality
        )
    raise ValueError("unexpected transient resource schema")
