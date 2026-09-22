#!/usr/bin/env python3
"""Long analysis mode 2/mode 3 surfaces, read from the compiled artifact."""
from __future__ import annotations

from dataclasses import replace
from functools import lru_cache

from ...analysis.config import LONG_PSYCH_ACOUSTIC_N, WwisePsyLongTables
from ..artifact import CompiledProfile

SCHEMA = "wem.psy-long-analysis-variants.v1"
_CURVE_ROWS = 3


@lru_cache(maxsize=None)
def load_long_variant(
    mode: int,
    profile: CompiledProfile,
    base: WwisePsyLongTables,
) -> WwisePsyLongTables:
    """Return a complete long table with the selected analysis surface.

    The floor-seed surface is shared by both modes and inherited from the
    checked base profile rather than duplicated in the carrier.
    """
    if not isinstance(profile, CompiledProfile):
        raise TypeError("long variant tables require a CompiledProfile")
    if not isinstance(base, WwisePsyLongTables):
        raise TypeError("long variant base must be WwisePsyLongTables")
    mode = int(mode)
    if mode not in (2, 3):
        raise ValueError("long analysis mode must be 2 or 3")
    prefix = f"long_variants.{mode}"
    if profile.optional_table(f"{prefix}.mode") is None:
        raise ValueError(f"long analysis table lacks mode {mode}")
    curves = [float(value) for value in profile.table(f"{prefix}.analysis_curves")]
    return replace(
        base,
        profile_key=f"1024:{mode}",
        analysis_profile_u32=tuple(
            int(value) for value in profile.table(f"{prefix}.analysis_profile_u32")
        ),
        analysis_interval_u32=tuple(
            int(value) for value in profile.table(f"{prefix}.analysis_interval_u32")
        ),
        analysis_curves=tuple(
            tuple(curves[row * LONG_PSYCH_ACOUSTIC_N : (row + 1) * LONG_PSYCH_ACOUSTIC_N])
            for row in range(_CURVE_ROWS)
        ),
        analysis_field_19_curve=tuple(
            float(value) for value in profile.table(f"{prefix}.analysis_field_19_curve")
        ),
    )


__all__ = ["SCHEMA", "load_long_variant"]
