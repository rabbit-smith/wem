#!/usr/bin/env python3
"""The fixed 1024-bin Wwise psychoacoustic tables, read from the artifact.

The tables are Rust constants in the kernel carrier; this module only reshapes
the stored words into the typed value objects: f32 curves as their exact
float32 values, u32 words as unsigned integers, and the flattened analysis
curves and seed tone banks nested back into their ``(row, bin)`` and
``(band, level, record)`` shapes.
"""

from __future__ import annotations

from functools import lru_cache

from ...analysis.config import LONG_PSYCH_ACOUSTIC_N, WwisePsyLongTables
from ..artifact import CompiledProfile

SCHEMA = "wem.psy-long-static.v1"
N = LONG_PSYCH_ACOUSTIC_N
PROFILE_WORDS = 256
LOOK_WORDS = 192
TONE_BANDS = 17
TONE_LEVELS = 8
TONE_RECORD_FLOATS = 58
_CURVE_ROWS = 3


def _nest(values: list, rows: int, width: int) -> tuple[tuple, ...]:
    return tuple(
        tuple(values[row * width : (row + 1) * width]) for row in range(rows)
    )


@lru_cache(maxsize=None)
def load_long_psy_tables(profile: CompiledProfile) -> WwisePsyLongTables:
    """Rebuild the fixed 1024-bin long tables the profile carries."""
    if not isinstance(profile, CompiledProfile):
        raise TypeError("long psychoacoustic tables require a CompiledProfile")

    def block(name: str) -> list:
        return profile.table(f"long_base.{name}")

    bands = _nest(
        [float(value) for value in block("seed_tone_banks")],
        TONE_BANDS * TONE_LEVELS,
        TONE_RECORD_FLOATS,
    )
    return WwisePsyLongTables(
        sample_rate=int(block("sample_rate")[0]),
        n=int(block("n")[0]),
        profile_key=str(block("profile_key")),
        analysis_profile_u32=tuple(int(v) for v in block("analysis_profile_u32")),
        analysis_interval_u32=tuple(int(v) for v in block("analysis_interval_u32")),
        analysis_curves=_nest([float(v) for v in block("analysis_curves")], _CURVE_ROWS, N),
        analysis_field_19_curve=tuple(float(v) for v in block("analysis_field_19_curve")),
        seed_outer_u32=tuple(int(v) for v in block("seed_outer_u32")),
        seed_profile_u32=tuple(int(v) for v in block("seed_profile_u32")),
        seed_base_curve=tuple(float(v) for v in block("seed_base_curve")),
        seed_group_labels_u32=tuple(int(v) for v in block("seed_group_labels_u32")),
        seed_tone_banks=tuple(
            tuple(bands[band * TONE_LEVELS + level] for level in range(TONE_LEVELS))
            for band in range(TONE_BANDS)
        ),
    )


__all__ = ["SCHEMA", "load_long_psy_tables"]
