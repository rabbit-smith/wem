#!/usr/bin/env python3
"""The short psychoacoustic seed surface, rebuilt from the compiled artifact."""
from __future__ import annotations

from functools import lru_cache

from ...analysis.config import WwisePsySeedSurface
from ..artifact import CompiledProfile

TONE_LEVEL_COUNT = 8
TONE_BAND_COUNT = 17
EHMER_OFFSET = 16
TONE_REFERENCE_LEVEL_DB = 30.0
NEGATIVE_INFINITY_DB = -9999.0
SPECTRUM_PEAK_DECAY_DB_PER_SECOND = 6.0




@lru_cache(maxsize=None)
def load_short_seed_surface(profile: CompiledProfile) -> WwisePsySeedSurface:
    """Rebuild the short seed surface from the compiled profile artifact.

    Every word the kernel's carrier holds comes back as it was stored: f32
    tables as their exact float32 values, integer tables as integers, and the
    flattened tone table reshaped to ``(band, level, record)``.
    """
    if not isinstance(profile, CompiledProfile):
        raise TypeError("short seed surface requires a CompiledProfile")
    prefix = "short_seed."

    def column(name: str) -> list:
        return profile.table(f"{prefix}{name}")

    n = int(column("n")[0])
    raw_tone = [float(value) for value in column("tone_curves")]
    expected_words = TONE_BAND_COUNT * TONE_LEVEL_COUNT * 58
    if len(raw_tone) != expected_words:
        raise ValueError("short seed surface has malformed tone data")
    tone_curves = tuple(
        tuple(
            tuple(
                raw_tone[(band * TONE_LEVEL_COUNT + level) * 58 : (band * TONE_LEVEL_COUNT + level + 1) * 58]
            )
            for level in range(TONE_LEVEL_COUNT)
        )
        for band in range(TONE_BAND_COUNT)
    )

    return WwisePsySeedSurface(
        n=n,
        sample_rate=int(column("sample_rate")[0]),
        ath_offset=float(column("ath_offset")[0]),
        ath_floor=float(column("ath_floor")[0]),
        seed_ceiling=float(column("seed_ceiling")[0]),
        max_curve_db=float(column("max_curve_db")[0]),
        curve_offset=float(column("curve_offset")[0]),
        curve_slope=float(column("curve_slope")[0]),
        curve_offset_2=float(column("curve_offset_2")[0]),
        curve_attenuation=tuple(float(v) for v in column("curve_attenuation")),
        ath=tuple(float(v) for v in column("ath")),
        octave=tuple(int(v) for v in column("octave")),
        first_octave=int(column("first_octave")[0]),
        shift_octave=int(column("shift_octave")[0]),
        eighth_octave_lines=int(column("eighth_octave_lines")[0]),
        total_octave_lines=int(column("total_octave_lines")[0]),
        tone_curves=tone_curves,
        row=tuple(float(v) for v in column("row")),
        envelope_low=tuple(float(v) for v in column("envelope_low")),
        envelope_high=tuple(float(v) for v in column("envelope_high")),
        mask_curve=tuple(float(v) for v in column("mask_curve")),
        interval_table=tuple(int(v) for v in column("interval_table")),
        short_limit=int(column("short_limit")[0]),
        noise_fixed_window=int(column("noise_fixed_window")[0]),
        regular_curve_bias=float(column("regular_curve_bias")[0]),
        regular_curve_cap=float(column("regular_curve_cap")[0]),
        remap_curve_offsets=tuple(int(v) for v in column("remap_curve_offsets")),
        remap_low_by_index=tuple(float(v) for v in column("remap_low_by_index")),
        remap_high_by_index=tuple(float(v) for v in column("remap_high_by_index")),
    )


def short_remap_parameters(surface: WwisePsySeedSurface) -> tuple[tuple[int, ...], tuple[float, ...], tuple[float, ...]]:
    """Return immutable remap lookup vectors owned by the short profile."""
    return surface.remap_curve_offsets, surface.remap_low_by_index, surface.remap_high_by_index

