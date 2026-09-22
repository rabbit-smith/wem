#!/usr/bin/env python3
"""Floor seed propagation and frame-global spectrum peak state."""
from __future__ import annotations

from dataclasses import dataclass
from typing import Sequence

from ..config import (
    TONE_BAND_COUNT,
    EHMER_OFFSET,
    SPECTRUM_PEAK_DECAY_DB_PER_SECOND,
    TONE_REFERENCE_LEVEL_DB,
    NEGATIVE_INFINITY_DB,
    TONE_LEVEL_COUNT,
    WwisePsyLongSeedLook,
    WwisePsyLongTables,
    WwisePsyLook,
    make_wwise_long_seed_look,
)
from ..._f32 import _f32

def build_long_floor_seed_from_look(
    look: WwisePsyLongSeedLook,
    logfft: Sequence[float],
    *,
    channel_specmax: float,
    global_specmax: float,
) -> list[float]:
    """Run the pure local 1024-bin ``floor-seed stage → floor-envelope stage.seed`` path.

    The implementation is the same checked ``16DB0 → 17030 → 16EA0`` port
    used by the short profile, with the long profile's static ATH curve,
    signed group labels, and 17×8×58 tone bank supplied from ``tables/``.
    No external file is opened on this path.
    """
    if len(logfft) != look.n:
        raise ValueError("long floor-seed stage logFFT geometry differs from seed look")
    ath_shift = max(
        float(look.ath_offset) + float(channel_specmax), float(look.ath_floor)
    )
    initial = [_f32(float(value) + ath_shift) for value in look.base_curve]
    return wwise_seed_floor(
        logfft,
        initial,
        look.tone_banks,
        look.group_labels,
        first_octave=look.first_octave,
        shift_octave=look.shift_octave,
        total_octave_lines=look.total_octave_lines,
        eighth_octave_lines=look.eighth_octave_lines,
        max_curve_db=look.max_curve_db,
        specmax=float(global_specmax),
        seed_ceiling=look.seed_ceiling,
    )


def build_long_floor_seed(
    logfft: Sequence[float],
    *,
    channel_specmax: float,
    global_specmax: float,
    table: WwisePsyLongTables | None = None,
) -> list[float]:
    """Convenience entry point for a 44.1 kHz / 1024-bin long block."""
    if table is None:
        raise ValueError("build_long_floor_seed requires a profile long table")
    return build_long_floor_seed_from_look(
        make_wwise_long_seed_look(table),
        logfft,
        channel_specmax=channel_specmax,
        global_specmax=global_specmax,
    )

def wwise_seed_curve(
    seed: list[float],
    curves: Sequence[Sequence[float]],
    amp: float,
    octave: int,
    total_octave_lines: int,
    eighth_octave_lines: int,
    dB_offset: float,
) -> None:
    """Port reference routine / Vorbis ``seed_curve``.

    ``curves`` is the selected band's eight-level bank.  Each entry is the
    packed post array ``[start, end, curve[0], ...]``.  The outer 17-band
    selection happens in ``wwise_seed_loop`` before reaching this helper.
    """
    choice = int((float(amp) + float(dB_offset) - TONE_REFERENCE_LEVEL_DB) * 0.1)
    choice = max(0, min(TONE_LEVEL_COUNT - 1, choice))
    if choice >= len(curves):
        raise ValueError("tone-curve level bank is shorter than the reference encoder bank")
    posts = curves[choice]
    if len(posts) < 2:
        raise ValueError("tone-curve post array needs start and end")
    post0 = int(posts[0])
    post1 = int(posts[1])
    seed_ptr = (
        int(octave)
        + (post0 - EHMER_OFFSET) * int(eighth_octave_lines)
        - (int(eighth_octave_lines) >> 1)
    )
    curve = posts[2:]
    for post in range(post0, post1):
        if 0 < seed_ptr < len(seed):
            curve_index = post
            if curve_index >= len(curve):
                raise ValueError("tone-curve posts are shorter than their end")
            seed[seed_ptr] = max(
                float(seed[seed_ptr]), _f32(float(amp) + float(curve[curve_index]))
            )
        seed_ptr += int(eighth_octave_lines)
        if seed_ptr >= int(total_octave_lines):
            break


def wwise_seed_loop(
    curves: Sequence[Sequence[Sequence[float]]],
    spectrum: Sequence[float],
    floor_curve: Sequence[float],
    seed: list[float],
    octave: Sequence[int],
    *,
    first_octave: int,
    shift_octave: int,
    total_octave_lines: int,
    eighth_octave_lines: int,
    dB_offset: float,
) -> None:
    """Port the grouped peak pass at reference routine.

    The reference encoder groups adjacent bins with the same ``octave`` value, tests the
    group's peak against ``floor_curve + 6``, then paints the selected tone
    curve into ``seed``.
    """
    n = len(spectrum)
    if len(floor_curve) != n or len(octave) != n:
        raise ValueError("seed-loop inputs must have equal bin counts")
    i = 0
    dB_offset = float(dB_offset)
    while i < n:
        peak = float(spectrum[i])
        group_octave = int(octave[i])
        while i + 1 < n and int(octave[i + 1]) == group_octave:
            i += 1
            if float(spectrum[i]) > peak:
                peak = float(spectrum[i])
        if peak + 6.0 > float(floor_curve[i]):
            band = group_octave >> int(shift_octave)
            band = max(0, min(TONE_BAND_COUNT - 1, band))
            # The Wwise helper's the tone-bank field is the per-band bank.  The
            # selected band is represented by the outer list here.
            if band >= len(curves):
                raise ValueError("tone-curve band bank is shorter than 17")
            wwise_seed_curve(
                seed,
                curves[band],
                peak,
                int(octave[i]) - int(first_octave),
                total_octave_lines,
                eighth_octave_lines,
                dB_offset,
            )
        i += 1


def wwise_seed_chase(
    seeds: list[float], eighth_octave_lines: int, total_octave_lines: int
) -> None:
    """Port the linear reference routine seed-chase pass."""
    positions: list[int] = []
    amplitudes: list[float] = []
    for i, value in enumerate(seeds[:total_octave_lines]):
        if len(positions) < 2:
            positions.append(i)
            amplitudes.append(float(value))
            continue
        while True:
            if float(value) < amplitudes[-1]:
                positions.append(i)
                amplitudes.append(float(value))
                break
            if i < positions[-1] + eighth_octave_lines:
                if (
                    len(positions) > 1
                    and amplitudes[-1] <= amplitudes[-2]
                    and i < positions[-2] + eighth_octave_lines
                ):
                    positions.pop()
                    amplitudes.pop()
                    continue
            positions.append(i)
            amplitudes.append(float(value))
            break

    pos = 0
    for stack_index, start in enumerate(positions):
        if stack_index + 1 < len(positions) and amplitudes[stack_index + 1] > amplitudes[stack_index]:
            end = positions[stack_index + 1]
        else:
            end = start + int(eighth_octave_lines) + 1
        end = min(end, int(total_octave_lines))
        for cursor in range(pos, end):
            seeds[cursor] = _f32(amplitudes[stack_index])
        # The source updates ``pos`` only through ``for(; pos < end; pos++)``.
        # A later stack interval may end behind the already-filled cursor;
        # C then leaves ``pos`` unchanged.  Assigning ``pos = end`` rewinds
        # the scan and overwrites a sparse set of boundary seeds (the exact
        pos = max(pos, end)


def wwise_max_seeds(
    seed: list[float],
    floor_curve: list[float],
    octave: Sequence[int],
    *,
    first_octave: int,
    eighth_octave_lines: int,
    total_octave_lines: int,
    seed_ceiling: float,
) -> None:
    """Port reference routine / Vorbis ``max_seeds`` in place."""
    if len(octave) != len(floor_curve):
        raise ValueError("octave and floor curves must have equal bin counts")
    wwise_seed_chase(seed, eighth_octave_lines, total_octave_lines)
    wwise_apply_max_seed_floor(
        seed,
        floor_curve,
        octave,
        first_octave=first_octave,
        eighth_octave_lines=eighth_octave_lines,
        total_octave_lines=total_octave_lines,
        seed_ceiling=seed_ceiling,
    )


def wwise_apply_max_seed_floor(
    seed: Sequence[float],
    floor_curve: list[float],
    octave: Sequence[int],
    *,
    first_octave: int,
    eighth_octave_lines: int,
    total_octave_lines: int,
    seed_ceiling: float,
) -> None:
    """Apply the final maximum-seed floor propagation in place.

    ``wwise_max_seeds`` owns the public complete operation.  Keeping the
    handoff separately testable lets a v6 runtime profile distinguish an
    upstream sparse-write mismatch from a chase-stack mismatch or the final
    octave-to-floor min walk, without changing the encoder path.
    """
    if len(octave) != len(floor_curve):
        raise ValueError("octave and floor curves must have equal bin counts")
    if len(seed) < total_octave_lines:
        raise ValueError("seed surface is shorter than total octave lines")
    if not octave:
        return
    pos = int(octave[0]) - int(first_octave) - (int(eighth_octave_lines) >> 1)
    linpos = 0
    while linpos + 1 < len(octave):
        if not 0 <= pos < total_octave_lines:
            raise ValueError("seed position falls outside total octave lines")
        min_value = float(seed[pos])
        end = ((int(octave[linpos]) + int(octave[linpos + 1])) >> 1) - int(first_octave)
        if min_value > float(seed_ceiling):
            min_value = float(seed_ceiling)
        while pos + 1 <= end:
            pos += 1
            # Same bound as the loop head above: the inner walk can step past
            # ``total_octave_lines`` before the next outer check, so guard here
            # as well. The paired build only avoids this because its geometry
            # keeps ``end`` inside the grid. Fail loudly (as the loop head does)
            # rather than raising an index error from the array access.
            if not 0 <= pos < total_octave_lines:
                raise ValueError("seed position falls outside total octave lines")
            if (
                (float(seed[pos]) > NEGATIVE_INFINITY_DB and float(seed[pos]) < min_value)
                or min_value == NEGATIVE_INFINITY_DB
            ):
                min_value = float(seed[pos])
        end = pos + int(first_octave)
        while linpos < len(octave) and int(octave[linpos]) <= end:
            if float(floor_curve[linpos]) < min_value:
                floor_curve[linpos] = _f32(min_value)
            linpos += 1
    min_value = float(seed[total_octave_lines - 1])
    while linpos < len(octave):
        if float(floor_curve[linpos]) < min_value:
            floor_curve[linpos] = _f32(min_value)
        linpos += 1


def compute_spectrum_peak(
    fft_curves: Sequence[Sequence[float]], *, initial_global: float = NEGATIVE_INFINITY_DB
) -> tuple[tuple[float, ...], float]:
    """Return the mapping pass's local and packet-global FFT maxima.

    The encoder computes every channel's packed FFT log curve first.  It
    clamps each channel maximum to 0 dB, then folds those values into the
    stream's carried ``ampmax`` before calling ``floor-seed stage`` for any channel.
    Keeping the two outputs together prevents accidentally substituting the
    raw-MDCT maximum (or one channel's maximum) for ``global_specmax``.

    ``initial_global`` is the carried value after its frame-decay step.  The
    first packet starts at ``-9999`` and therefore reduces to the maximum of
    its six local FFT maxima.
    """
    global_max = _f32(initial_global)
    local_maxima: list[float] = []
    for curve in fft_curves:
        if not curve:
            raise ValueError("FFT curve must contain at least one bin")
        local = _f32(min(0.0, max(float(value) for value in curve)))
        local_maxima.append(local)
        if local > global_max:
            global_max = local
    return tuple(local_maxima), global_max


@dataclass
class SpectrumPeakState:
    """Carried packet-global FFT peak state owned by the scheduler."""

    value: float = NEGATIVE_INFINITY_DB


def update_frame_spectrum_peak(
    fft_curves: Sequence[Sequence[float]],
    *,
    block_bins: int,
    sample_rate: int,
    state: SpectrumPeakState,
) -> tuple[tuple[float, ...], float]:
    """Decay/fold the global FFT maximum for one short or long frame.

    Before any channel reaches ``floor-seed stage``, ``frame analysis`` decays the carried peak by
    ``6 dB/s * block_bins / sample_rate`` (with the scalar spill rounded to
    float32), then folds all current channel FFT maxima.  This distinct frame
    operation avoids accidentally carrying the previous group unchanged into
    long floor-seed stage seeding.
    """
    if block_bins <= 0 or sample_rate <= 0:
        raise ValueError("specmax frame geometry must be positive")
    decay = _f32(
        SPECTRUM_PEAK_DECAY_DB_PER_SECOND
        * float(block_bins) / float(sample_rate)
    )
    decayed = _f32(float(state.value) - decay)
    local, global_max = compute_spectrum_peak(
        fft_curves, initial_global=decayed
    )
    state.value = global_max
    return local, global_max

def wwise_seed_floor(
    spectrum: Sequence[float],
    floor_curve: Sequence[float],
    curves: Sequence[Sequence[Sequence[float]]],
    octave: Sequence[int],
    *,
    first_octave: int,
    shift_octave: int,
    total_octave_lines: int,
    eighth_octave_lines: int,
    max_curve_db: float,
    specmax: float,
    seed_ceiling: float,
) -> list[float]:
    """Run the evidence-backed ``floor-seed stage`` seed front half.

    This is deliberately separate from ``psychoacoustic remap``: the call graph shows ``floor-seed stage``
    writes its initialized ATH/floor curve, seeds it from the FFT peaks, and
    then applies the chase/max pass before floor-envelope stage.  The caller still owns
    the exact ATH and tone-curve buffers.
    """
    if len(spectrum) != len(floor_curve) or len(octave) != len(spectrum):
        raise ValueError("seed-floor inputs must have equal bin counts")
    if total_octave_lines <= 0:
        raise ValueError("total octave line count must be positive")
    seed = [NEGATIVE_INFINITY_DB] * total_octave_lines
    out = [_f32(value) for value in floor_curve]
    wwise_seed_loop(
        curves,
        spectrum,
        out,
        seed,
        octave,
        first_octave=first_octave,
        shift_octave=shift_octave,
        total_octave_lines=total_octave_lines,
        eighth_octave_lines=eighth_octave_lines,
        dB_offset=float(max_curve_db) - float(specmax),
    )
    wwise_max_seeds(
        seed,
        out,
        octave,
        first_octave=first_octave,
        eighth_octave_lines=eighth_octave_lines,
        total_octave_lines=total_octave_lines,
        seed_ceiling=seed_ceiling,
    )
    return out


def _wwise_seed_initial_curve(
    look: WwisePsyLook, channel_specmax: float
) -> list[float]:
    """Build ``floor-seed stage``'s ATH work curve before grouped FFT peak seeding."""
    if len(look.ath) != look.n:
        raise ValueError("psycho look is missing ATH state")
    ath_shift = max(
        float(look.ath_offset) + float(channel_specmax), float(look.ath_floor)
    )
    return [_f32(value + ath_shift) for value in look.ath]


def wwise_seed_floor_from_look(
    look: WwisePsyLook,
    spectrum: Sequence[float],
    *,
    channel_specmax: float,
    global_specmax: float,
) -> list[float]:
    """Translate the ``floor-seed stage`` ATH initialization into Python buffers.

    ``floor-seed stage`` starts from ``look->ath + max(config[1] + channel_specmax,
    config[2])``.  It passes the work curve to the floor-seed stage with
    ``config[165] - global_specmax`` as the tone-bank level offset, then
    caps propagated seeds at ``config[8]``.
    """
    if len(spectrum) != look.n:
        raise ValueError("seed spectrum and psycho look length differ")
    if not look.tone_curves:
        raise ValueError("psycho look is missing ATH or tone-curve state")
    initial = _wwise_seed_initial_curve(look, channel_specmax)
    return wwise_seed_floor(
        spectrum,
        initial,
        look.tone_curves,
        look.octave,
        first_octave=look.first_octave,
        shift_octave=look.shift_octave,
        total_octave_lines=look.total_octave_lines,
        eighth_octave_lines=look.eighth_octave_lines,
        max_curve_db=look.max_curve_db,
        specmax=float(global_specmax),
        seed_ceiling=look.seed_ceiling,
    )
