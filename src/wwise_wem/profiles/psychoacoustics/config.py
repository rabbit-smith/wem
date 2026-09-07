#!/usr/bin/env python3
"""Immutable psychoacoustic profile models and validated resource loading."""
from __future__ import annotations

import math
from functools import lru_cache

from ...analysis.config import (
    WwisePsySeedSurface,
)
from ..resources import ResourceRef
from ...analysis.dsp.transform import _f32

TONE_LEVEL_COUNT = 8
TONE_BAND_COUNT = 17
EHMER_OFFSET = 16
TONE_REFERENCE_LEVEL_DB = 30.0
NEGATIVE_INFINITY_DB = -9999.0
SPECTRUM_PEAK_DECAY_DB_PER_SECOND = 6.0




def _float_tuple(value: object, size: int, label: str) -> tuple[float, ...]:
    if not isinstance(value, list) or len(value) != size:
        raise ValueError(f"short profile {label} must contain {size} values")
    result = tuple(_f32(float(item)) for item in value)
    if not all(math.isfinite(item) for item in result):
        raise ValueError(f"short profile {label} contains non-finite values")
    return result


def _int_tuple(value: object, size: int, label: str) -> tuple[int, ...]:
    if not isinstance(value, list) or len(value) != size or any(
        not isinstance(item, int) or isinstance(item, bool) for item in value
    ):
        raise ValueError(f"short profile {label} must contain {size} integers")
    return tuple(value)


@lru_cache(maxsize=None)
def load_short_seed_surface(ref: ResourceRef) -> WwisePsySeedSurface:
    """Load the checksum-addressed short profile and validate its complete shape."""
    if not isinstance(ref, ResourceRef):
        raise TypeError("short seed resource must be ResourceRef")
    payload = ref.read_json()
    if not isinstance(payload, dict) or payload.get("schema") != "wem.seed-surface.v6":
        raise ValueError("unexpected short seed-surface schema")
    profile = payload.get("profile")
    geometry = payload.get("geometry")
    look = payload.get("look")
    if not (isinstance(profile, dict) and isinstance(geometry, dict) and isinstance(look, dict)):
        raise ValueError("short seed surface is missing a required section")
    n = geometry.get("n")
    sample_rate = geometry.get("sample_rate")
    if n != 128 or sample_rate != 44100:
        raise ValueError("short seed surface has unsupported geometry")
    if payload.get("tone_shape") != [TONE_BAND_COUNT, TONE_LEVEL_COUNT, 58]:
        raise ValueError("short seed surface has an unexpected tone shape")

    raw_tone: list[float] = []
    tone_rle = payload.get("tone_rle")
    if not isinstance(tone_rle, list):
        raise ValueError("short seed surface lacks tone RLE")
    for entry in tone_rle:
        if not isinstance(entry, list) or len(entry) != 2:
            raise ValueError("short seed surface has malformed tone RLE")
        value, count = entry
        if not isinstance(count, int) or isinstance(count, bool) or count <= 0:
            raise ValueError("short seed surface has invalid tone RLE count")
        raw_tone.extend([_f32(float(value))] * count)
    expected_words = TONE_BAND_COUNT * TONE_LEVEL_COUNT * 58
    if len(raw_tone) != expected_words or not all(math.isfinite(value) for value in raw_tone):
        raise ValueError("short seed surface has malformed tone data")
    tone_curves = tuple(
        tuple(
            tuple(raw_tone[(band * TONE_LEVEL_COUNT + level) * 58:(band * TONE_LEVEL_COUNT + level + 1) * 58])
            for level in range(TONE_LEVEL_COUNT)
        )
        for band in range(TONE_BAND_COUNT)
    )

    return WwisePsySeedSurface(
        n=n,
        sample_rate=sample_rate,
        ath_offset=_f32(float(profile["ath_offset"])),
        ath_floor=_f32(float(profile["ath_floor"])),
        seed_ceiling=_f32(float(profile["seed_ceiling"])),
        max_curve_db=_f32(float(profile["max_curve_db"])),
        curve_offset=_f32(float(profile["curve_offset"])),
        curve_slope=_f32(float(profile["curve_slope"])),
        curve_offset_2=_f32(float(profile["curve_offset_2"])),
        curve_attenuation=_float_tuple(profile.get("curve_attenuation"), TONE_BAND_COUNT, "curve attenuation"),
        ath=_float_tuple(payload.get("ath"), n, "ATH"),
        octave=_int_tuple(payload.get("octave"), n, "octave map"),
        first_octave=int(geometry["first_octave"]),
        shift_octave=int(geometry["shift_octave"]),
        eighth_octave_lines=int(geometry["eighth_octave_lines"]),
        total_octave_lines=int(geometry["total_octave_lines"]),
        tone_curves=tone_curves,
        row=_float_tuple(look.get("row"), 133, "profile row"),
        envelope_low=_float_tuple(look.get("envelope_low"), 17, "lower envelope"),
        envelope_high=_float_tuple(look.get("envelope_high"), 17, "upper envelope"),
        mask_curve=_float_tuple(look.get("mask_curve"), n, "mask curve"),
        interval_table=_int_tuple(look.get("interval_table"), n, "interval table"),
        short_limit=int(look["short_limit"]),
        noise_fixed_window=int(look["noise_fixed_window"]),
        regular_curve_bias=_f32(float(look["regular_curve_bias"])),
        regular_curve_cap=_f32(float(look["regular_curve_cap"])),
        remap_curve_offsets=_int_tuple(look.get("remap_curve_offsets"), 40, "remap offsets"),
        remap_low_by_index=_float_tuple(look.get("remap_low_by_index"), 40, "lower remap extension"),
        remap_high_by_index=_float_tuple(look.get("remap_high_by_index"), 40, "upper remap extension"),
    )


def short_remap_parameters(surface: WwisePsySeedSurface) -> tuple[tuple[int, ...], tuple[float, ...], tuple[float, ...]]:
    """Return immutable remap lookup vectors owned by the short profile."""
    return surface.remap_curve_offsets, surface.remap_low_by_index, surface.remap_high_by_index

