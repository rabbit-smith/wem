#!/usr/bin/env python3
"""The two short-block psychoacoustic profiles, read from the artifact.

The table is a Rust constant in the kernel carrier; this module only reshapes
the stored words into the typed profile objects. Its floats are f64, exactly as
the reference stores them.
"""
from __future__ import annotations

from functools import lru_cache

from typing import cast

from ...analysis.config import ShortPsyProfile
from ..artifact import CompiledProfile

_MASK_CURVE_ROWS = 3
_MASK_CURVE_WIDTH = 128


def _flatten(values: tuple[tuple[float, ...], ...]) -> tuple[float, ...]:
    return tuple(value for row in values for value in row)


@lru_cache(maxsize=None)
def load_short_psy_profiles(profile: CompiledProfile) -> tuple[ShortPsyProfile, ...]:
    """Rebuild the short-block psychoacoustic profiles the profile carries."""
    if not isinstance(profile, CompiledProfile):
        raise TypeError("short psychoacoustic profiles require a CompiledProfile")
    count = profile.int("short_profiles.count")
    result = []
    for index in range(count):
        prefix = f"short_profiles.{index}"
        flat = tuple(float(value) for value in profile.table(f"{prefix}.mask_curves"))
        curves = tuple(
            flat[row * _MASK_CURVE_WIDTH : (row + 1) * _MASK_CURVE_WIDTH]
            for row in range(_MASK_CURVE_ROWS)
        )
        band_limits = tuple(int(value) for value in profile.table(f"{prefix}.band_limits"))
        result.append(
            ShortPsyProfile(
                key=str(profile.table(f"{prefix}.key")),
                candidate_bias_by_mode=tuple(
                    float(value)
                    for value in profile.table(f"{prefix}.candidate_bias_by_mode")
                ),
                curve_cap=float(profile.table(f"{prefix}.curve_cap")[0]),
                group_enabled=int(profile.table(f"{prefix}.group_enabled")[0]),
                candidate_bound=int(profile.table(f"{prefix}.candidate_bound")[0]),
                group_span=int(profile.table(f"{prefix}.group_span")[0]),
                band_limits=cast("tuple[int, int, int]", band_limits),
                side_gain=float(profile.table(f"{prefix}.side_gain")[0]),
                peak_cutoff=int(profile.table(f"{prefix}.peak_cutoff")[0]),
                blend_weight=float(profile.table(f"{prefix}.blend_weight")[0]),
                mask_curves=curves,
            )
        )
    return tuple(result)
