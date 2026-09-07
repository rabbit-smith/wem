#!/usr/bin/env python3
"""Strict loader for the fixed 1024-bin Wwise psychoacoustic tables.

Runtime encoder code imports only the checked static JSON table.
"""

from __future__ import annotations

import hashlib
import math
import struct
from typing import Any

from ...analysis.config import WwisePsyLongTables
from ..resources import ResourceRef


SCHEMA = "wem.psy-long-static.v1"
N = 1024
PROFILE_WORDS = 256
LOOK_WORDS = 192
TONE_BANDS = 17
TONE_LEVELS = 8
TONE_RECORD_FLOATS = 58


def _f32(value: Any, label: str) -> float:
    if not isinstance(value, (int, float)) or isinstance(value, bool):
        raise ValueError(f"{label} must contain floats")
    value = float(value)
    if not math.isfinite(value):
        raise ValueError(f"{label} contains a non-finite float")
    return struct.unpack("<f", struct.pack("<f", value))[0]


def _u32(value: Any, label: str) -> int:
    if not isinstance(value, int) or isinstance(value, bool) or not 0 <= value <= 0xFFFFFFFF:
        raise ValueError(f"{label} must contain uint32 values")
    return value


def _floats(value: Any, count: int, label: str) -> tuple[float, ...]:
    if not isinstance(value, list) or len(value) != count:
        raise ValueError(f"{label} must contain {count} floats")
    return tuple(_f32(item, label) for item in value)


def _u32s(value: Any, count: int, label: str) -> tuple[int, ...]:
    if not isinstance(value, list) or len(value) != count:
        raise ValueError(f"{label} must contain {count} uint32 values")
    return tuple(_u32(item, label) for item in value)


def load_long_psy_tables(
    ref: ResourceRef,
) -> WwisePsyLongTables:
    """Load one validated static table."""
    if not isinstance(ref, ResourceRef):
        raise TypeError("long psychoacoustic resource must be ResourceRef")
    payload = ref.read_json()
    if not isinstance(payload, dict) or payload.get("schema") != SCHEMA:
        raise ValueError("unexpected long psychoacoustic-table schema")
    if payload.get("n") != N or payload.get("sample_rate") != 44100:
        raise ValueError("long psychoacoustic table has unexpected geometry")
    profile_key = payload.get("profile_key")
    if not isinstance(profile_key, str) or not profile_key:
        raise ValueError("long psychoacoustic table has no profile key")
    analysis, seed = payload.get("analysis"), payload.get("seed")
    if not isinstance(analysis, dict) or not isinstance(seed, dict):
        raise ValueError("long psychoacoustic table lacks analysis/seed sections")

    curves_raw = analysis.get("curves")
    if not isinstance(curves_raw, list) or len(curves_raw) != 3:
        raise ValueError("analysis.curves must contain three rows")
    curves = tuple(
        _floats(row, N, f"analysis.curves[{index}]")
        for index, row in enumerate(curves_raw)
    )

    banks_raw = seed.get("tone_banks")
    if not isinstance(banks_raw, list) or len(banks_raw) != TONE_BANDS:
        raise ValueError("seed.tone_banks must contain 17 bands")
    banks: list[tuple[tuple[float, ...], ...]] = []
    for band, levels in enumerate(banks_raw):
        if not isinstance(levels, list) or len(levels) != TONE_LEVELS:
            raise ValueError(f"seed.tone_banks[{band}] must contain 8 levels")
        banks.append(
            tuple(
                _floats(record, TONE_RECORD_FLOATS, f"seed.tone_banks[{band}][{level}]")
                for level, record in enumerate(levels)
            )
        )

    return WwisePsyLongTables(
        sample_rate=44100,
        n=N,
        profile_key=profile_key,
        analysis_profile_u32=_u32s(analysis.get("profile_u32"), PROFILE_WORDS, "analysis.profile_u32"),
        analysis_interval_u32=_u32s(analysis.get("interval_u32"), N, "analysis.interval_u32"),
        analysis_curves=curves,
        analysis_field_19_curve=_floats(analysis.get("field_19_curve"), N, "analysis.field_19_curve"),
        seed_outer_u32=_u32s(seed.get("outer_u32"), LOOK_WORDS, "seed.outer_u32"),
        seed_profile_u32=_u32s(seed.get("profile_u32"), PROFILE_WORDS, "seed.profile_u32"),
        seed_base_curve=_floats(seed.get("base_curve"), N, "seed.base_curve"),
        seed_group_labels_u32=_u32s(seed.get("group_labels_u32"), N, "seed.group_labels_u32"),
        seed_tone_banks=tuple(banks),
    )


def table_sha256(ref: ResourceRef) -> str:
    if not isinstance(ref, ResourceRef):
        raise TypeError("long psychoacoustic resource must be ResourceRef")
    payload = ref.read_bytes()
    return hashlib.sha256(payload).hexdigest()
