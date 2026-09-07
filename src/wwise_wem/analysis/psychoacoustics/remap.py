#!/usr/bin/env python3
"""Psychoacoustic smoothing, peak suppression, and spectrum remapping."""
from __future__ import annotations

import struct
from dataclasses import dataclass
from typing import Sequence

from ..config import WwisePsyLongTables, WwisePsyLook
from ..dsp.transform import _f32, _u32_f32

LONG_PSY_N = 1024

def wwise_psy_peak_suppress(
    original: Sequence[float],
    difference: Sequence[float],
    look: WwisePsyLook,
) -> list[float]:
    """Port the short/mode-0 branch of reference routine.

    ``original`` is the MDCT log curve and ``difference`` is the in-place
    MDCT-minus-FFT curve prepared by the reference routinepsychoacoustic remap``.  The function returns
    the modified difference; the caller adds the configured floor offset during the
    following quantization stage.
    """
    if len(original) != len(difference):
        raise ValueError("psycho curves must have the same length")
    if len(original) != look.n:
        raise ValueError("psycho look and curve length differ")

    n = look.n
    limit = min(look.short_limit, n)
    smooth = [0.0] * n
    peak_floor = [0.0] * n
    for i in range(min(limit + 4, n)):
        value = float(original[i])
        smooth[i] = _f32(
            value if value >= -70.0 else (value + 70.0) * 0.1 - 70.0
        )

    threshold = 9.0 if n != 256 else 15.0
    center = 4
    while center < limit:
        if not (
            float(original[center - 1]) < float(original[center])
            and float(original[center + 1]) < float(original[center])
        ):
            center += 1
            continue
        left = center - 1
        left_floor = center - 3
        cursor = left
        while cursor > left_floor and float(original[cursor]) <= float(original[cursor + 1]):
            left = cursor
            cursor -= 1
        right = center + 1
        right_limit = center + 4
        while right < right_limit and float(original[right]) <= float(original[right - 1]):
            right += 1
        right -= 1

        delta = max(
            smooth[center] - smooth[left],
            smooth[center] - smooth[right],
        )
        if delta <= threshold:
            center = right + 1
            continue
        if float(difference[center]) < float(original[center]):
            # MSVC materializes ``delta-threshold`` in a float local, then
            # multiplies by the promoted float literal whose exact double
            # value is 0x3fe3333340000000 (0.6000000238418579).
            delta = _f32(delta - threshold)
            delta = _f32(delta * 0.6000000238418579)
        for i in range(left, right + 1):
            peak_floor[i] = max(0.0, max(peak_floor[i], _f32(delta)))
        # The reference encoder resumes after the whole local peak interval rather than
        # testing nested centers inside it a second time.
        center = right + 1

    out = [_f32(value) for value in difference]
    for i in range(3, limit):
        cap = min(
            look.envelope[i],
            look.second_envelope[i] + abs(look.second_envelope[0]),
        )
        if cap < peak_floor[i]:
            peak_floor[i] = cap
        out[i] = _f32(out[i] - peak_floor[i])
    return out


@dataclass(frozen=True)
class LongRemapResult:
    """Pure local result of the 1024-bin ``psychoacoustic remap`` mode-2 front end.

    The fields deliberately retain every boundary that the reference encoder exposes to the
    following stage.  They are useful for regression, while ``remap`` is the
    encoder-facing buffer passed to ``remapped spectrum``.  No field is populated from
    a runtime-profile record: ``tables`` is the fixed 44100-Hz configuration
    image in :mod:`profiles.psychoacoustics.long_tables`.
    """

    first_smooth: list[float]
    residual: list[float]
    selector: list[float]
    base_before_peak: list[float]
    base: list[float]
    remap: list[float]


def wwise_psy_peak_suppress_long_mode2(
    base_curve: Sequence[float],
    tables: WwisePsyLongTables,
) -> list[float]:
    """Port reference routine's long / mode-2 branch.

    Unlike the short branch, mode 2 averages the *base* curve into 8-bin
    groups, identifies a local group maximum, and subtracts its excess from
    a wider interval.  The reduction is capped by both fixed look curves:

    ``min(look[19][peak_bin], look[3][1][peak_bin] + abs(look[3][1][0]))``.

    ``look[23]`` is the active span (608 for the checked 1024-bin profile),
    recovered in the immutable table as the shared long-look word at index
    23.  The rest of the 1024-bin curve remains untouched.  This routine is
    intentionally self-contained and is also geometry-checked so it is not
    silently reused with a different psycho profile.
    """
    if not isinstance(tables, WwisePsyLongTables):
        raise TypeError("tables must be WwisePsyLongTables")
    if len(base_curve) != LONG_PSY_N or tables.n != LONG_PSY_N:
        raise ValueError("long mode-2 peak suppress expects 1024 bins")

    # In the stored long look this is exactly the active-bin field in peak suppression.  The
    # same immutable look layout is also consumed by the long seed stage;
    # using it here preserves the reference encoder's 608-bin active domain without any
    # per-frame external data.
    active_bins = int(tables.seed_outer_u32[23])
    if active_bins <= 0 or active_bins > LONG_PSY_N or active_bins % 8:
        raise ValueError("long psycho table has an invalid mode-2 active span")

    # peak suppression spills each eight-bin mean to a float stack slot before its peak
    # loop.  Keep that float32 store visible; accumulation otherwise follows
    # the floating-point source order and has been checked bit-for-bit at this boundary.
    means = [
        _f32(sum(float(value) for value in base_curve[offset : offset + 8]) * 0.125)
        for offset in range(0, active_bins, 8)
    ]
    out = [_f32(value) for value in base_curve]
    cap_curve = tables.analysis_curves[1]
    cap_bias = abs(float(cap_curve[0]))

    # The reference encoder starts at mean[2], tests mean[i:i+3], and advances one 8-bin
    # group after each test.  The write interval intentionally extends beyond
    # ``active_bins`` at the final possible group, but stays inside the
    # 1024-bin allocation; clamping to the output length preserves that exact
    # observable curve while keeping this pure Python port bounds-safe.
    for i in range(2, len(means) - 2):
        if not (means[i] < means[i + 1] and means[i + 2] < means[i + 1]):
            continue
        if means[i - 1] >= means[i]:
            local_floor = means[i]
            start = (i - 1) * 8
        else:
            local_floor = means[i - 1]
            start = (i - 2) * 8
        reduction = float(means[i + 1]) - float(local_floor)
        if reduction <= 2.0:
            continue

        peak_bin = (i + 1) * 8
        cap = min(
            float(tables.analysis_field_19_curve[peak_bin]),
            float(cap_curve[peak_bin]) + cap_bias,
        )
        reduction = min(reduction - 2.0, cap)
        end = min(len(out) - 1, (i + 4) * 8)
        for bin_index in range(start, end + 1):
            out[bin_index] = _f32(float(out[bin_index]) - reduction)
    return out


def build_long_psy_remap_mode2(
    original: Sequence[float],
    tables: WwisePsyLongTables,
) -> LongRemapResult:
    """Run the local 1024-bin long ``psychoacoustic remap`` analysis front end.

    This is the pure encoder path for the observed long mode-2 profile:

    ``raw -> smoothing(+140) -> raw-first -> smoothing(0, fixed=100)``
    ``-> peak suppression(mode2) -> profile[84 + round/clamp(selector)] -> remap``.

    The fixed interval table, active span, peak caps, and 40-entry remap LUT
    all come from the installed profile's ``psychoacoustics/long-base.json``.
    This runtime path accepts only current channel buffers and packaged profile data.
    """
    if not isinstance(tables, WwisePsyLongTables):
        raise TypeError("tables must be WwisePsyLongTables")
    if len(original) != LONG_PSY_N or tables.n != LONG_PSY_N:
        raise ValueError("long psychoacoustic remap mode 2 expects 1024 bins")

    raw = [_f32(value) for value in original]
    first = wwise_psy_curve_smooth(
        raw, tables.analysis_interval_u32, offset=140.0, fixed_window=-1
    )
    residual = [_f32(value - smooth) for value, smooth in zip(raw, first)]
    selector = wwise_psy_curve_smooth(
        residual, tables.analysis_interval_u32, offset=0.0, fixed_window=100
    )
    # This explicit subtract/subtract sequence is the stack-local base passed
    # as the third argument of peak suppression.  (For the checked float32 data it equals
    # ``first`` bit-for-bit, but retaining both operations preserves the reference encoder
    # dataflow and prevents a future table/profile change from hiding it.)
    base_before_peak = [_f32(value - remainder) for value, remainder in zip(raw, residual)]
    base = wwise_psy_peak_suppress_long_mode2(base_before_peak, tables)
    remap_lut = tuple(
        _u32_f32(word) for word in tables.analysis_profile_u32[84 : 84 + 40]
    )
    if len(remap_lut) != 40:
        raise ValueError("long psycho table has no 40-entry remap LUT")
    remap = [
        _f32(float(base_value) + remap_lut[max(0, min(39, int(float(choice) + 0.5)))])
        for base_value, choice in zip(base, selector)
    ]
    return LongRemapResult(
        first_smooth=first,
        residual=residual,
        selector=selector,
        base_before_peak=base_before_peak,
        base=base,
        remap=remap,
    )


def wwise_psy_row3_curve(
    base_curve: Sequence[float],
    selector_curve: Sequence[float],
    curve_offsets: Sequence[int],
) -> list[float]:
    """Build ``psychoacoustic remap``'s offset curve from its base and selector buffers."""
    if len(base_curve) != len(selector_curve):
        raise ValueError("psycho base and selector curves must have the same length")
    return [
        _f32(
            float(base)
            + curve_offsets[
                max(0, min(39, int(float(selector) + 0.5)))
            ]
        )
        for base, selector in zip(base_curve, selector_curve)
    ]


def _wwise_signed_hi16(value: int) -> int:
    """Decode the signed high half of a profile interpolation packed interval."""
    high = (int(value) >> 16) & 0xFFFF
    return high - 0x10000 if high & 0x8000 else high


def _wwise_psy_prefix_moments(
    curve: Sequence[float], offset: float
) -> tuple[list[float], list[float], list[float], list[float], list[float]]:
    """Build the five float prefix arrays used by reference routine.

    The routine fits a straight line to ``x[i]`` with ``x[i]²`` weights.
    Therefore its stored moments are ``Σx²``, ``Σi*x²``, ``Σi²*x²``,
    ``Σx³`` and ``Σi*x³``.  Index zero has the reference encoder's half weight.  The C
    initializer writes ``X[0] = .5*x²`` but leaves ``XY[0]`` at zero.
    """
    p0: list[float] = []
    p1: list[float] = []
    p2: list[float] = []
    q0: list[float] = []
    q1: list[float] = []
    s0 = s1 = s2 = t0 = t1 = 0.0
    for i, value in enumerate(curve):
        # ``smoothing`` is compiled from Xiph's float locals on reference.  The floating-point
        # expression temporaries stay wide, but every assignment to ``y``,
        # ``w`` and the five running ``t*`` accumulators is a float32
        # boundary.  Treating those locals as Python doubles changes the
        # ill-conditioned regression by up to several 1e-3 dB on short
        # blocks even though the prefix arrays themselves are float32.
        x = max(_f32(_f32(value) + _f32(offset)), 1.0)
        x2 = _f32(x * x)
        endpoint_weight = 0.5 if i == 0 else 1.0
        weighted = _f32(endpoint_weight * x2)
        s0 = _f32(s0 + weighted)
        # bark_noise_hybridmp initializes its X term at one for the
        # half-weight endpoint, then advances to integer coordinates.
        s1 = _f32(s1 + (weighted if i == 0 else i * x2))
        # Keep the source expression order ``w*x*x``.  Its intermediate is
        # an floating-point temporary; only the assignment to tXX rounds to float32.
        s2 = _f32(s2 + (0.0 if i == 0 else x2 * i * i))
        t0 = _f32(t0 + weighted * x)
        # The source does not add w*y to tXY in its special i=0 setup.
        t1 = _f32(t1 + (0.0 if i == 0 else x2 * i * x))
        # The reference encoder stores these alloca-backed prefix entries as float32.
        p0.append(_f32(s0))
        p1.append(_f32(s1))
        p2.append(_f32(s2))
        q0.append(_f32(t0))
        q1.append(_f32(t1))
    return p0, p1, p2, q0, q1


def wwise_psy_curve_smooth(
    curve: Sequence[float],
    interval_table: Sequence[int],
    *,
    offset: float = 140.0,
    fixed_window: int = 0,
) -> list[float]:
    """Port the first, active reference routine smoothing pass.

    ``interval_table`` is the packed ``(signed_start << 16) | end`` array
    built by reference routine.  For each interval the reference encoder performs weighted
    least-squares fitting on the moment prefixes, clamps the fitted line to
    the previous floor, and stores ``fit - offset``.  This is the pass used
    by ``psychoacoustic remap`` with ``offset=140`` and the negative extension sentinel.

    A positive ``fixed_window`` additionally performs the source routine's
    fixed-width minimum fit after the Bark-window fit.  The active profile's
    second pass binds it to profile u32 field 32 (=15); making the parameter
    explicit keeps that field auditable without a duplicate line-fit port.
    """
    n = len(curve)
    if n == 0:
        return []
    if len(interval_table) < n:
        raise ValueError("psycho interval table is shorter than the curve")

    p0, p1, p2, q0, q1 = _wwise_psy_prefix_moments(curve, offset)
    out = [0.0] * n
    # Initial state from smoothing: previous slope/intercept are 0 and the
    # denominator is one, so a sentinel interval can safely extend them.
    slope_num = 0.0
    intercept_num = 0.0
    denominator = 1.0
    floor = 0.0
    xcoord = 0.0
    emitted = 0

    def fit(start: int, end: int, crossing_zero: bool) -> tuple[float, float, float]:
        if not (0 <= start < n and 0 <= end < n):
            raise ValueError("psycho interval endpoint out of range")
        if crossing_zero:
            a = _f32(p0[end] + p0[start])
            b = _f32(p1[end] - p1[start])
            c = _f32(p2[end] + p2[start])
            y = _f32(q0[end] + q0[start])
            xy = _f32(q1[end] - q1[start])
        else:
            a = _f32(p0[end] - p0[start])
            b = _f32(p1[end] - p1[start])
            c = _f32(p2[end] - p2[start])
            y = _f32(q0[end] - q0[start])
            xy = _f32(q1[end] - q1[start])
        # A/B/D/R are source ``float`` locals.  Products and sums are floating-point
        # expression temporaries, followed by one float32 assignment.
        den = _f32(a * c - b * b)
        if abs(den) < 1e-20:
            return slope_num, intercept_num, denominator
        # These are the exact numerator order in smoothing:
        # slope=(A*XY-B*Y)/den, intercept=(C*Y-B*XY)/den.
        return _f32(a * xy - b * y), _f32(c * y - b * xy), den

    for packed in interval_table:
        if emitted >= n:
            break
        start = _wwise_signed_hi16(packed)
        end = int(packed) & 0xFFFF
        # smoothing stops interval fitting when the low half reaches n; the
        # current line then fills the remaining output slots.
        if end >= n:
            break
        crossing_zero = start < 0
        start = -start if crossing_zero else start
        if start >= n:
            break
        slope_num, intercept_num, denominator = fit(
            start, end, crossing_zero
        )
        fitted = _f32((xcoord * slope_num + intercept_num) / denominator)
        fitted = max(floor, fitted)
        out[emitted] = _f32(fitted - float(offset))
        emitted += 1
        xcoord += 1.0

    while emitted < n:
        fitted = _f32((xcoord * slope_num + intercept_num) / denominator)
        fitted = max(floor, fitted)
        out[emitted] = _f32(fitted - float(offset))
        emitted += 1
        xcoord += 1.0

    fixed = int(fixed_window)
    if fixed <= 0:
        return out

    # smoothing's optional fixed-width tail.  Unlike the main Bark fit, it only
    # lowers existing values and does not clamp the candidate line at zero.
    emitted = 0
    xcoord = 0.0
    while emitted < n:
        end = emitted + fixed // 2
        start = end - fixed
        if end >= n or start >= 0:
            break
        slope_num, intercept_num, denominator = fit(-start, end, True)
        candidate = _f32(
            _f32((xcoord * slope_num + intercept_num) / denominator)
            - _f32(offset)
        )
        if candidate < out[emitted]:
            out[emitted] = candidate
        emitted += 1
        xcoord += 1.0

    while emitted < n:
        end = emitted + fixed // 2
        start = end - fixed
        if end >= n or start < 0:
            break
        slope_num, intercept_num, denominator = fit(start, end, False)
        candidate = _f32(
            _f32((xcoord * slope_num + intercept_num) / denominator)
            - _f32(offset)
        )
        if candidate < out[emitted]:
            out[emitted] = candidate
        emitted += 1
        xcoord += 1.0

    while emitted < n:
        candidate = _f32(
            _f32((xcoord * slope_num + intercept_num) / denominator)
            - _f32(offset)
        )
        if candidate < out[emitted]:
            out[emitted] = candidate
        emitted += 1
        xcoord += 1.0
    return out


def wwise_psy_residual_core(
    original: Sequence[float],
    look: WwisePsyLook,
) -> tuple[list[float], list[float], list[float]]:
    """Stage the reference routinepsychoacoustic remap`` as ``(first_smooth, selector, base)``.

    The second ``smoothing`` result is a selector for the later 40-entry lookup;
    it is not subtracted from the base curve.  The reference encoder instead reconstructs
    ``base = original - (original - first_smooth)`` and passes that base
    through ``peak suppression`` before creating its two output curves.
    """
    if len(original) != look.n:
        raise ValueError("psycho look and curve length differ")

    first = wwise_psy_curve_smooth(
        original, look.interval_table, offset=140.0
    )
    difference = [_f32(a - b) for a, b in zip(original, first)]
    selector = wwise_psy_curve_smooth(
        difference,
        look.interval_table,
        offset=0.0,
        fixed_window=look.noise_fixed_window,
    )
    base = [_f32(a - b) for a, b in zip(original, difference)]
    base = wwise_psy_peak_suppress(original, base, look)
    return first, selector, base


def wwise_psy_apply_q_extension(
    base_curve: Sequence[float],
    selector_curve: Sequence[float],
    q: float,
    *,
    low_extension: float | Sequence[float],
    high_extension: float | Sequence[float],
) -> list[float]:
    """Apply the ``psychoacoustic remap`` q-row extension remap.

    ``selector_curve`` chooses the row-table entry; ``base_curve`` supplies
    the value added to it.  The q interpolation is applied only to the first
    ``n//3`` bins, exactly as the reference encoder's first loop does.
    """
    if len(base_curve) != len(selector_curve):
        raise ValueError("psycho base and selector curves must have the same length")
    low_table: tuple[float, ...] | None = None
    high_table: tuple[float, ...] | None = None
    low_scalar = 0.0
    high_scalar = 0.0
    if isinstance(low_extension, (int, float)):
        low_scalar = float(low_extension)
    else:
        low_table = tuple(float(value) for value in low_extension)
    if isinstance(high_extension, (int, float)):
        high_scalar = float(high_extension)
    else:
        high_table = tuple(float(value) for value in high_extension)
    if low_table is not None and len(low_table) < 40:
        raise ValueError("low q-extension table needs 40 entries")
    if high_table is not None and len(high_table) < 40:
        raise ValueError("high q-extension table needs 40 entries")
    output = []
    q_limit = len(base_curve) // 3
    for i, (base, selector) in enumerate(zip(base_curve, selector_curve)):
        index = max(0, min(39, int(float(selector) + 0.5)))
        low = low_table[index] if low_table is not None else low_scalar
        high = (
            high_table[index]
            if high_table is not None
            else high_scalar
        )
        if float(q) > 0.0 and i < q_limit:
            output.append(_f32(low + float(base) - (low - high) * float(q)))
        else:
            output.append(_f32(low + float(base)))
    return output


def build_psy_remap(
    original: Sequence[float],
    q: float,
    look: WwisePsyLook,
    *,
    low_extension: float | Sequence[float] | None = None,
    high_extension: float | Sequence[float] | None = None,
    curve_offsets: Sequence[int],
) -> tuple[list[float], list[float], list[float], list[float], list[float]]:
    """Run the confirmed ``psychoacoustic remap`` front half for one short curve.

    The tuple is ``(first_smooth, selector_smooth, base, noise_mask,
    noise_mask)``.  The fourth item is the buffer passed as ``remapped spectrum``.
    It is the completed noise mask: ``base + noisecompand[selector]``.

    The v4 runtime profile binds ``remapped spectrum`` to this companded result, not
    to the profile q-extension scratch.  Keep the duplicated fifth result for
    compatibility with earlier diagnostic callers which named it
    ``offset_curve``.
    """
    first, selector, base = wwise_psy_residual_core(original, look)
    # ``q`` and the extension fields are retained in this public signature
    # because their separate scratch role is still modelled by
    # ``wwise_psy_apply_q_extension``.  They do not select remapped spectrum.
    del q, low_extension, high_extension
    noise_mask = wwise_psy_row3_curve(base, selector, curve_offsets)
    return first, selector, base, noise_mask, noise_mask


def build_long_psy_remap_variant(
    raw: Sequence[float], mode: int, table: WwisePsyLongTables
) -> LongRemapResult:
    """Build the long remap with the selected immutable analysis profile."""
    if len(raw) != 1024:
        raise ValueError("long psychoacoustic remap expects 1024 bins")
    original = [_f32(value) for value in raw]
    first = wwise_psy_curve_smooth(
        original, table.analysis_interval_u32, offset=140.0, fixed_window=-1
    )
    residual = [_f32(value - smooth) for value, smooth in zip(original, first)]
    selector = wwise_psy_curve_smooth(
        residual, table.analysis_interval_u32, offset=0.0, fixed_window=100
    )
    base_before = [_f32(value - remainder) for value, remainder in zip(original, residual)]
    base = (
        wwise_psy_peak_suppress_long_mode2(base_before, table)
        if int(mode) == 2
        else list(base_before)
    )
    lut = [
        struct.unpack("<f", struct.pack("<I", word))[0]
        for word in table.analysis_profile_u32[84:124]
    ]
    remap = [
        _f32(value + lut[max(0, min(39, int(float(choice) + 0.5)))])
        for value, choice in zip(base, selector)
    ]
    return LongRemapResult(first, residual, selector, base_before, base, remap)
