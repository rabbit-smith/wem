#!/usr/bin/env python3
"""Pure loader and runtime adapter for long analysis mode 2/mode 3."""
from __future__ import annotations

import struct
from dataclasses import replace
from functools import lru_cache

from ...analysis.config import WwisePsyLongTables
from ..resources import ResourceRef


SCHEMA = "wem.psy-long-analysis-variants.v1"
def _f32(value: float) -> float:
    return struct.unpack("<f", struct.pack("<f", float(value)))[0]


@lru_cache(maxsize=None)
def load_long_variant(
    mode: int,
    ref: ResourceRef,
    base: WwisePsyLongTables,
) -> WwisePsyLongTables:
    """Return a complete long table with the selected analysis surface.

    The floor-seed surface is shared by both modes and inherited from the
    checked base profile rather than duplicated in the variant resource.
    """
    mode = int(mode)
    if mode not in (2, 3):
        raise ValueError("long analysis mode must be 2 or 3")
    if not isinstance(ref, ResourceRef):
        raise TypeError("long variant resource must be ResourceRef")
    if not isinstance(base, WwisePsyLongTables):
        raise TypeError("long variant base must be WwisePsyLongTables")
    payload = ref.read_json()
    if payload.get("schema") != SCHEMA or payload.get("n") != 1024 or payload.get("sample_rate") != 44100:
        raise ValueError("unexpected long analysis variant table")
    surface = payload.get("variants", {}).get(str(mode))
    if not isinstance(surface, dict):
        raise ValueError(f"long analysis table lacks mode {mode}")
    profile = surface.get("profile_u32")
    intervals = surface.get("interval_u32")
    curves = surface.get("curves")
    field19 = surface.get("field_19_curve")
    if not isinstance(profile, list) or len(profile) != 256:
        raise ValueError("malformed long variant profile")
    if not isinstance(intervals, list) or len(intervals) != 1024:
        raise ValueError("malformed long variant intervals")
    if not isinstance(curves, list) or len(curves) != 3 or any(not isinstance(row, list) or len(row) != 1024 for row in curves):
        raise ValueError("malformed long variant curves")
    if not isinstance(field19, list) or len(field19) != 1024:
        raise ValueError("malformed long variant field-19 curve")
    return replace(
        base,
        profile_key=f"1024:{mode}",
        analysis_profile_u32=tuple(int(value) & 0xFFFFFFFF for value in profile),
        analysis_interval_u32=tuple(int(value) & 0xFFFFFFFF for value in intervals),
        analysis_curves=tuple(tuple(_f32(value) for value in row) for row in curves),
        analysis_field_19_curve=tuple(_f32(value) for value in field19),
    )
