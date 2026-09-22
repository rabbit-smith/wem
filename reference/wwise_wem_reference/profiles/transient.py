#!/usr/bin/env python3
"""Immutable window and band configuration for transient detection.

The kernel's carrier records one of two transient mechanisms per profile, and
this module rebuilds whichever it finds from the compiled artifact:

* a pre-materialized static detector table (the historical shape, used by
  profiles that register one);
* the static bias/threshold record family from the paired encoder build. The
  assembly layer materializes one detector table from it per quality value:
  the record-index curve on the shared quality axis picks a (possibly
  fractional) record index, the floor record supplies every field verbatim,
  and only upper[0..3]/lower[0..3] are linearly interpolated between the
  adjacent records at the fractional part.

There is no document decoding here any more: the carrier holds the decoded
words, so the only remaining work is the selection/interpolation the reference
owns.
"""
from __future__ import annotations

import math
import struct
from dataclasses import dataclass
from functools import lru_cache

from ..analysis.config import (
    SHORT_PSYCH_ACOUSTIC_N,
    TransientBandConfig,
    TransientDetectorTables,
)
from .artifact import CompiledProfile, named_blocks
from .quality import _linear_frac, normalize_quality_factor


def _u32_f32(value: int) -> float:
    return struct.unpack("<f", struct.pack("<I", int(value) & 0xFFFFFFFF))[0]


def _bands(profile: CompiledProfile, prefix: str = "transient.bands") -> tuple[TransientBandConfig, ...]:
    """Rebuild one f32 band descriptor table from the carrier."""
    count = int(profile.table(f"{prefix}.count")[0])
    return tuple(
        TransientBandConfig(
            offset=int(profile.table(f"{prefix}.{index}.offset")[0]),
            weights=tuple(float(v) for v in profile.table(f"{prefix}.{index}.weights")),
            scale=float(profile.table(f"{prefix}.{index}.scale")[0]),
        )
        for index in range(count)
    )


@lru_cache(maxsize=None)
def load_transient_tables(profile: CompiledProfile) -> TransientDetectorTables:
    """Rebuild a pre-materialized n=128 static detector table."""
    if not isinstance(profile, CompiledProfile):
        raise TypeError("transient tables require a CompiledProfile")
    return TransientDetectorTables(
        n=profile.int("transient.n"),
        bias=float(profile.table("transient.bias")[0]),
        window=tuple(float(v) for v in profile.table("transient.window")),
        config=tuple(float(v) for v in profile.table("transient.config")),
        bands=_bands(profile),
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


def load_transient_record_family(profile: CompiledProfile) -> TransientRecordFamily:
    """Rebuild the static record library from the carrier.

    The window and band surfaces travel as stored f32 words; the record words
    travel as stored u32 bit patterns, which is what the materialization kernel
    reads them at.
    """
    if not isinstance(profile, CompiledProfile):
        raise TypeError("transient record family requires a CompiledProfile")
    names = named_blocks(profile, "transient.record.", ".marker_u32")
    if len(names) != TRANSIENT_RECORD_COUNT:
        raise ValueError(
            f"transient record-family must carry {TRANSIENT_RECORD_COUNT} records"
        )
    records = []
    for index in sorted(int(name) for name in names):
        prefix = f"transient.record.{index}"
        records.append(
            TransientRecord(
                file_off=str(profile.table(f"{prefix}.file_off")),
                marker_u32=int(profile.table(f"{prefix}.marker_u32")[0]),
                upper_u32=tuple(int(v) for v in profile.table(f"{prefix}.upper_u32")),
                lower_u32=tuple(int(v) for v in profile.table(f"{prefix}.lower_u32")),
                carry_u32=int(profile.table(f"{prefix}.carry_u32")[0]),
                bias_u32=int(profile.table(f"{prefix}.bias_u32")[0]),
                m_u32=int(profile.table(f"{prefix}.m_u32")[0]),
                tail_u32=int(profile.table(f"{prefix}.tail_u32")[0]),
                config_u32=tuple(int(v) for v in profile.table(f"{prefix}.config_u32")),
            )
        )
    return TransientRecordFamily(
        schema=TRANSIENT_RECORD_FAMILY_SCHEMA,
        n=profile.int("transient.n"),
        sample_rate=profile.int("transient.sample_rate"),
        default_record_index=float(profile.table("transient.default_record_index")[0]),
        records=tuple(records),
        index_curve=tuple(float(v) for v in profile.table("transient.record_index_curve")),
        breakpoints=tuple(
            float(v) for v in profile.table("transient.quality_axis_breakpoints")
        ),
        window_u32=tuple(_f32_bits(v) for v in profile.table("transient.window")),
        bands=_bands(profile),
    )


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
    profile: CompiledProfile,
    quality: float | None = None,
) -> TransientDetectorTables:
    """Read whichever transient mechanism the profile records.

    A pre-materialized detector table is returned as recorded (quality
    ignored, exactly as before); the record family is materialized per quality
    (the paired build's static mechanism).
    """
    if not isinstance(profile, CompiledProfile):
        raise TypeError("transient tables require a CompiledProfile")
    kind = str(profile.table("transient.kind"))
    if kind == "detector":
        return load_transient_tables(profile)
    if kind == "record-family":
        return materialize_transient_tables(load_transient_record_family(profile), quality)
    raise ValueError(f"unexpected transient mechanism {kind!r}")
