"""Immutable typed inputs consumed by the analysis domain.

The profile layer owns decoding and validating packaged calibration files.
Analysis receives only the value objects defined here and never selects or
opens a resource itself.
"""
from __future__ import annotations

from dataclasses import dataclass
import math
import struct
from types import MappingProxyType
from typing import Mapping, Sequence

TONE_LEVEL_COUNT = 8
TONE_BAND_COUNT = 17
EHMER_OFFSET = 16
TONE_REFERENCE_LEVEL_DB = 30.0
NEGATIVE_INFINITY_DB = -9999.0
SPECTRUM_PEAK_DECAY_DB_PER_SECOND = 6.0

@dataclass(frozen=True)
class MdctLook:
    n: int
    log2n: int
    trig: tuple[float, ...]
    bitrev: tuple[int, ...]
    scale: float


@dataclass(frozen=True)
class TransientBandConfig:
    offset: int
    weights: tuple[float, ...]
    scale: float


@dataclass(frozen=True)
class TransientDetectorTables:
    n: int
    bias: float
    window: tuple[float, ...]
    config: tuple[float, ...]
    bands: tuple[TransientBandConfig, ...]


@dataclass(frozen=True)
class ShortPsyProfile:
    key: str
    candidate_bias_by_mode: tuple[float, ...]
    curve_cap: float
    group_enabled: int
    candidate_bound: int
    group_span: int
    band_limits: tuple[int, int, int]
    side_gain: float
    peak_cutoff: int
    blend_weight: float
    mask_curves: tuple[tuple[float, ...], ...]


@dataclass(frozen=True)
class WwisePsyLongTables:
    sample_rate: int
    n: int
    profile_key: str
    analysis_profile_u32: tuple[int, ...]
    analysis_interval_u32: tuple[int, ...]
    analysis_curves: tuple[tuple[float, ...], ...]
    analysis_field_19_curve: tuple[float, ...]
    seed_outer_u32: tuple[int, ...]
    seed_profile_u32: tuple[int, ...]
    seed_base_curve: tuple[float, ...]
    seed_group_labels_u32: tuple[int, ...]
    seed_tone_banks: tuple[tuple[tuple[float, ...], ...], ...]


@dataclass(frozen=True)
class WwisePsyLook:
    n: int
    sample_rate: int
    row: tuple[float, ...]
    envelope: tuple[float, ...]
    second_envelope: tuple[float, ...]
    short_limit: int
    max_curve_db: float
    ath_offset: float
    ath_floor: float
    seed_ceiling: float
    regular_curve_bias: float
    regular_curve_cap: float
    interval_table: tuple[int, ...]
    noise_fixed_window: int
    ath: tuple[float, ...]
    octave: tuple[int, ...]
    first_octave: int
    shift_octave: int
    total_octave_lines: int
    eighth_octave_lines: int
    curve_attenuation: tuple[float, ...]
    tone_curves: tuple[tuple[tuple[float, ...], ...], ...]
    mask_curves: tuple[tuple[float, ...], ...]


@dataclass(frozen=True)
class WwisePsyLongSeedLook:
    n: int
    sample_rate: int
    profile_key: str
    ath_offset: float
    ath_floor: float
    seed_ceiling: float
    max_curve_db: float
    base_curve: tuple[float, ...]
    group_labels: tuple[int, ...]
    first_octave: int
    shift_octave: int
    eighth_octave_lines: int
    total_octave_lines: int
    tone_banks: tuple[tuple[tuple[float, ...], ...], ...]


@dataclass(frozen=True)
class LongFloorEnvelopeLook:
    n: int
    mask_curve: tuple[float, ...]
    curve_bias: float
    curve_cap: float
    history_start: int
    side_gain: float = 1.0


@dataclass(frozen=True)
class WwisePsySeedSurface:
    n: int
    sample_rate: int
    ath_offset: float
    ath_floor: float
    seed_ceiling: float
    max_curve_db: float
    curve_offset: float
    curve_slope: float
    curve_offset_2: float
    curve_attenuation: tuple[float, ...]
    ath: tuple[float, ...]
    octave: tuple[int, ...]
    first_octave: int
    shift_octave: int
    eighth_octave_lines: int
    total_octave_lines: int
    tone_curves: tuple[tuple[tuple[float, ...], ...], ...]
    row: tuple[float, ...]
    envelope_low: tuple[float, ...]
    envelope_high: tuple[float, ...]
    mask_curve: tuple[float, ...]
    interval_table: tuple[int, ...]
    short_limit: int
    noise_fixed_window: int
    regular_curve_bias: float
    regular_curve_cap: float
    remap_curve_offsets: tuple[int, ...]
    remap_low_by_index: tuple[float, ...]
    remap_high_by_index: tuple[float, ...]


def _f32(value: float) -> float:
    return struct.unpack("<f", struct.pack("<f", float(value)))[0]


def _u32_f32(value: int) -> float:
    return struct.unpack("<f", struct.pack("<I", int(value) & 0xFFFFFFFF))[0]


def _signed_u32(value: int) -> int:
    value = int(value) & 0xFFFFFFFF
    return value - (1 << 32) if value & (1 << 31) else value


def make_wwise_long_seed_look(table: WwisePsyLongTables) -> WwisePsyLongSeedLook:
    outer, profile = table.seed_outer_u32, table.seed_profile_u32
    if table.n != 1024 or table.sample_rate != 44100:
        raise ValueError("long seed table has unsupported geometry")
    if outer[0] != table.n or outer[11] != table.sample_rate:
        raise ValueError("long seed geometry disagrees with the table header")
    first_octave = _signed_u32(outer[7])
    shift_octave = int(outer[8])
    eighth_octave_lines = int(outer[9])
    total_octave_lines = int(outer[10])
    if not (first_octave < 0 and 0 < shift_octave < 32 and eighth_octave_lines > 0 and 0 < total_octave_lines <= 0x100000):
        raise ValueError("long seed table has invalid octave geometry")
    labels = tuple(_signed_u32(value) for value in table.seed_group_labels_u32)
    if len(labels) != table.n or any(labels[i] > labels[i + 1] for i in range(table.n - 1)):
        raise ValueError("long seed group labels are malformed")
    if not (len(table.seed_tone_banks) == TONE_BAND_COUNT and all(len(bank) == TONE_LEVEL_COUNT for bank in table.seed_tone_banks)):
        raise ValueError("long seed table has malformed tone-bank geometry")
    return WwisePsyLongSeedLook(
        table.n, table.sample_rate, table.profile_key, _u32_f32(profile[1]),
        _u32_f32(profile[2]), _u32_f32(profile[8]), _u32_f32(profile[165]),
        table.seed_base_curve, labels, first_octave, shift_octave,
        eighth_octave_lines, total_octave_lines, table.seed_tone_banks,
    )


def make_long_floor_envelope_look(table: WwisePsyLongTables) -> LongFloorEnvelopeLook:
    if table.n != 1024 or len(table.analysis_curves) < 2:
        raise ValueError("long floor-envelope table has unsupported geometry")
    profile = table.analysis_profile_u32
    if len(profile) <= 167 or int(profile[167]) < 0:
        raise ValueError("long floor-envelope table lacks valid scalar fields")
    return LongFloorEnvelopeLook(
        table.n, table.analysis_curves[1], _u32_f32(profile[4]),
        _u32_f32(profile[27]), int(profile[167]),
    )


def make_wwise_psy_look(
    surface: WwisePsySeedSurface,
    *,
    row: Sequence[float] | None = None,
) -> WwisePsyLook:
    n, sample_rate = surface.n, surface.sample_rate
    if n != 128 or sample_rate != 44100:
        raise ValueError("short psychoacoustic look has unsupported geometry")
    selected_row = surface.row if row is None else tuple(float(value) for value in row)
    if len(selected_row) != len(surface.row):
        raise ValueError("short psychoacoustic row has the wrong length")

    def coordinate(index: int) -> tuple[int, float]:
        frequency = (float(index) + 0.5) * float(sample_rate) / float(2 * n)
        value = _f32(2.0 * (math.log(frequency) * 1.442695021629333 - 5.965784072875977))
        value = max(0.0, min(16.0, value))
        base = int(value)
        return base, _f32(value - base)

    def interpolate(offset: int) -> tuple[float, ...]:
        values = []
        for index in range(n):
            base, fraction = coordinate(index)
            values.append(_f32(fraction * selected_row[offset + 1] + (1.0 - fraction) * selected_row[offset]))
        return tuple(values)

    envelope = []
    for index in range(n):
        base, fraction = coordinate(index)
        envelope.append(_f32(fraction * surface.envelope_high[base] + (1.0 - fraction) * surface.envelope_low[base]))
    interpolated = tuple(interpolate(offset) for offset in (33, 50, 67))
    return WwisePsyLook(
        n, sample_rate, tuple(selected_row), tuple(envelope), surface.mask_curve,
        surface.short_limit, surface.max_curve_db, surface.ath_offset,
        surface.ath_floor, surface.seed_ceiling, surface.regular_curve_bias,
        surface.regular_curve_cap, surface.interval_table,
        surface.noise_fixed_window, surface.ath, surface.octave,
        surface.first_octave, surface.shift_octave, surface.total_octave_lines,
        surface.eighth_octave_lines, surface.curve_attenuation,
        surface.tone_curves, (interpolated[0], surface.mask_curve, interpolated[2]),
    )


@dataclass(frozen=True)
class AnalysisProfileResources:
    """Immutable resources injected into one analysis session.

    Psychoacoustic tables join this aggregate in the following migration
    batch; MDCT and transient configuration are already fully explicit.
    """

    mdct_looks: Mapping[int, MdctLook]
    transient: TransientDetectorTables
    short_profiles: tuple[ShortPsyProfile, ...]
    short_surface: WwisePsySeedSurface
    short_look: WwisePsyLook
    long_base: WwisePsyLongTables
    long_variants: Mapping[int, WwisePsyLongTables]
    long_floor_looks: Mapping[int, LongFloorEnvelopeLook]

    def __post_init__(self) -> None:
        object.__setattr__(
            self, "mdct_looks", MappingProxyType(dict(self.mdct_looks))
        )
        object.__setattr__(self, "short_profiles", tuple(self.short_profiles))
        object.__setattr__(self, "long_variants", MappingProxyType(dict(self.long_variants)))
        object.__setattr__(self, "long_floor_looks", MappingProxyType(dict(self.long_floor_looks)))
        if not {128, 256, 2048} <= set(self.mdct_looks):
            raise ValueError("analysis resources lack required MDCT looks")
        if len(self.short_profiles) != 2 or set(self.long_variants) != {2, 3}:
            raise ValueError("analysis resources lack psychoacoustic variants")
