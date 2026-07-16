#!/usr/bin/env python3
"""Pure-runtime loader for the two short-block psychoacoustic profiles."""
from __future__ import annotations

from functools import lru_cache

from ...analysis.config import ShortPsyProfile
from ..resources import ResourceRef


@lru_cache(maxsize=None)
def load_short_psy_profiles(ref: ResourceRef) -> tuple[ShortPsyProfile, ...]:
    if not isinstance(ref, ResourceRef):
        raise TypeError("short psychoacoustic resource must be ResourceRef")
    data = ref.read_json()
    if data.get("schema") != "wem.short-psy-profiles.v1" or data.get("n") != 128:
        raise ValueError("short psychoacoustic table schema/geometry changed")
    rows = data.get("profiles")
    if not isinstance(rows, list) or len(rows) != 2:
        raise ValueError("short psychoacoustic table must contain two profiles")
    result = []
    for row in rows:
        curves = row.get("mask_curves")
        if not isinstance(curves, list) or len(curves) != 3 or any(len(curve) != 128 for curve in curves):
            raise ValueError("short psychoacoustic profile has malformed mask curves")
        result.append(ShortPsyProfile(
            key=str(row["key"]),
            candidate_bias_by_mode=tuple(float(value) for value in row["candidate_bias_by_mode"]),
            curve_cap=float(row["curve_cap"]),
            group_enabled=int(row["group_enabled"]),
            candidate_bound=int(row["candidate_bound"]),
            group_span=int(row["group_span"]),
            band_limits=tuple(int(value) for value in row["band_limits"]),
            side_gain=float(row["side_gain"]),
            peak_cutoff=int(row["peak_cutoff"]),
            blend_weight=float(row["blend_weight"]),
            mask_curves=tuple(tuple(float(value) for value in curve) for curve in curves),
        ))
    return tuple(result)
