"""Encoder-side floor1 curve fitting and post quantization."""
from __future__ import annotations

import math
from dataclasses import dataclass
from typing import Sequence

from .floor import (
    FLOOR1_RANGES,
    floor1_neighbor_tables,
    postlist_from_floor,
    render_point,
)
from .._tmath import ln_f64


def _mag_to_quant(mag: float, mult: int, range_: int) -> int:
    """Map linear magnitude → floor1 quant using vorbis dB scale + mult."""
    if mag <= 0.0:
        return 0
    # todB on amplitude stored as coeff: log(x*x)*4.34294480
    db = ln_f64(mag * mag) * 4.34294480
    # vorbis_dBquant: maps ~[-140, 0] dB → [0, 1023]
    q = int(db * 7.3142857 + 1023.5)
    if q > 1023:
        q = 1023
    if q < 0:
        q = 0
    if mult == 1:
        q >>= 2
    elif mult == 2:
        q >>= 3
    elif mult == 3:
        q //= 12
    else:
        q >>= 4
    if q >= range_:
        q = range_ - 1
    return q


# Encoder-side profile for the serialized 128 x 11 floor.  It is
# not in the packed setup: the matching libvorbis floor template defines
# ``60, 30, 500, 1, 18`` for ``maxover, maxunder, maxerr, twofitweight,
# twofitatten``.  Together with the source-semantic fit fixes below, these
# values reproduce the expected post words from their raw/post curve inputs.
FLOOR1_WWISE_FIT_PARAMS: dict[str, float] = {
    "maxover": 60.0,
    "maxunder": 30.0,
    "maxerr": 500.0,
    "twofitweight": 1.0,
    "twofitatten": 18.0,
}
# The serialized 1024×27 floor is the libvorbis ``floor_all.h`` profile 7.
# It keeps the same error limits but gives A-bucket samples a weight of 3,
# rather than the 128×11 profile's 1.  The distinction changes long-frame
# endpoint fits materially.
FLOOR1_WWISE_FIT_PARAMS_LONG: dict[str, float] = {
    **FLOOR1_WWISE_FIT_PARAMS,
    "twofitweight": 3.0,
}
_DB_QUANT_SCALE = 7.314285755157471


@dataclass
class _FloorFitAcc:
    """Integer sums collected for one sorted floor interval."""

    x0: int
    x1: int
    xa: int = 0
    ya: int = 0
    x2a: int = 0
    y2a: int = 0
    xya: int = 0
    an: int = 0
    xb: int = 0
    yb: int = 0
    x2b: int = 0
    y2b: int = 0
    xyb: int = 0
    bn: int = 0


def _c_trunc_div(num: int, den: int) -> int:
    """C99 integer division toward zero for the Bresenham fit path."""
    if den == 0:
        raise ZeroDivisionError("floor1 division by zero")
    q = abs(num) // abs(den)
    return -q if (num < 0) != (den < 0) else q


def _wwise_db_quant(db: float) -> int:
    """Apply Wwise's dB quantization rule: truncate, then clamp."""
    q = int(db * _DB_QUANT_SCALE + 1023.5)
    if q < 0:
        return 0
    if q > 1023:
        return 1023
    return q


def _accumulate_fit(
    quantized_curve: Sequence[float],
    floor_curve: Sequence[float],
    x0: int,
    x1: int,
    n: int,
    params: dict[str, float],
) -> _FloorFitAcc:
    """Accumulate floor-fit statistics over an inclusive interval."""
    acc = _FloorFitAcc(x0=x0, x1=x1)
    end = min(x1, n - 1)
    if x0 > end:
        return acc
    atten = params["twofitatten"]
    for x in range(x0, end + 1):
        quantized = _wwise_db_quant(float(quantized_curve[x]))
        if not quantized:
            continue
        # The primary bucket contains fitted-floor values at or below the raw
        # MDCT curve plus attenuation. The complementary bucket is secondary.
        if float(floor_curve[x]) + atten >= float(quantized_curve[x]):
            acc.xa += x
            acc.ya += quantized
            acc.x2a += x * x
            acc.y2a += quantized * quantized
            acc.xya += x * quantized
            acc.an += 1
        else:
            acc.xb += x
            acc.yb += quantized
            acc.x2b += x * x
            acc.y2b += quantized * quantized
            acc.xyb += x * quantized
            acc.bn += 1
    return acc


def _fit_line(
    fits: Sequence[_FloorFitAcc],
    y0: int,
    y1: int,
    params: dict[str, float],
) -> tuple[int, int, bool]:
    """Fit a line using the exact weighted least-squares accumulator."""
    if not fits:
        return 0, 0, True
    xb = yb = x2b = y2b = xyb = bn = 0.0
    for acc in fits:
        # Weight the primary bucket using its sample-count denominator.
        weight = (acc.bn + acc.an) * params["twofitweight"] / (acc.an + 1) + 1.0
        xb += acc.xb + acc.xa * weight
        yb += acc.yb + acc.ya * weight
        x2b += acc.x2b + acc.x2a * weight
        y2b += acc.y2b + acc.y2a * weight
        xyb += acc.xyb + acc.xya * weight
        bn += acc.bn + acc.an * weight

    x0 = fits[0].x0
    x1 = fits[-1].x1
    if y0 >= 0:
        xb += x0
        yb += y0
        x2b += x0 * x0
        y2b += y0 * y0
        xyb += y0 * x0
        bn += 1.0
    if y1 >= 0:
        xb += x1
        yb += y1
        x2b += x1 * x1
        y2b += y1 * y1
        xyb += y1 * x1
        bn += 1.0

    denom = bn * x2b - xb * xb
    if denom <= 0.0:
        return 0, 0, True
    intercept = (yb * x2b - xyb * xb) / denom
    slope = (bn * xyb - xb * yb) / denom
    # Add 0.5 before flooring. This differs from nearest-even rounding at
    # half-integer ties, including one long-profile endpoint.
    out0 = math.floor(intercept + slope * x0 + 0.5)
    out1 = math.floor(intercept + slope * x1 + 0.5)
    out0 = min(1023, max(0, out0))
    out1 = min(1023, max(0, out1))
    return out0, out1, False


def _inspect_error(
    x0: int,
    x1: int,
    y0: int,
    y1: int,
    quantized_curve: Sequence[float],
    floor_curve: Sequence[float],
    params: dict[str, float],
) -> bool:
    """Return whether the fitted line exceeds the local error bounds."""
    adx = x1 - x0
    if adx <= 0:
        return True
    dy = y1 - y0
    base = _c_trunc_div(dy, adx)
    sy = base - 1 if dy < 0 else base + 1
    ady = abs(dy) - abs(base * adx)
    x = x0
    y = y0
    err = 0
    val = _wwise_db_quant(float(quantized_curve[x]))
    mse = (y - val) * (y - val)
    count = 1
    if float(quantized_curve[x]) <= float(floor_curve[x]) + params["twofitatten"]:
        if y + params["maxover"] < val or y - params["maxunder"] > val:
            return True

    while x + 1 < x1:
        x += 1
        err += ady
        if err >= adx:
            err -= adx
            y += sy
        else:
            y += base
        val = _wwise_db_quant(float(quantized_curve[x]))
        mse += (y - val) * (y - val)
        count += 1
        if (
            float(quantized_curve[x]) <= float(floor_curve[x]) + params["twofitatten"]
            and val
        ):
            if y + params["maxover"] < val or y - params["maxunder"] > val:
                return True

    if params["maxover"] * params["maxover"] / count > params["maxerr"]:
        return False
    if params["maxunder"] * params["maxunder"] / count > params["maxerr"]:
        return False
    # Use integer division for accumulated MSE before comparing with maxerr.
    # With maxerr=0 this intentionally
    # tolerates a total error smaller than the number of checked bins.
    return mse // count > params["maxerr"]


def _post_y(values_a: Sequence[int], values_b: Sequence[int], pos: int) -> int:
    if values_a[pos] < 0:
        return values_b[pos]
    if values_b[pos] < 0:
        return values_a[pos]
    return (values_a[pos] + values_b[pos]) >> 1


def _propagate_high_neighbor(
    local_hi: list[int], sortpos: int, high: int, post_index: int
) -> None:
    """Update high-neighbor links after inserting a split post."""
    cursor = sortpos - 1
    while cursor >= 0 and local_hi[cursor] == high:
        local_hi[cursor] = post_index
        cursor -= 1


def floor1_fit_wwise(
    fitted_floor_curve: Sequence[float],
    raw_mdct_curve: Sequence[float],
    floor: dict,
    n: int | None = None,
    fit_params: dict[str, float] | None = None,
) -> list[int] | None:
    """Fit absolute floor1 posts from psychoacoustic and raw-MDCT curves.

    ``fitted_floor_curve`` is quantized and fitted. ``raw_mdct_curve``
    determines whether each bin is within ``twofitatten`` of that curve. The
    result contains absolute post Y values, with 0x8000 marking posts
    represented by prediction; pass it through ``floor1_wrap`` before packet
    packing.
    """
    postlist = postlist_from_floor(floor)
    posts = len(postlist)
    if posts < 2:
        return None
    spectrum_n = (1 << floor["rangebits"]) if n is None else n
    required = min(spectrum_n, postlist[1])
    if (
        spectrum_n <= 0
        or len(raw_mdct_curve) < required
        or len(fitted_floor_curve) < required
    ):
        raise ValueError("floor1 log curves are shorter than the floor range")
    params = dict(
        FLOOR1_WWISE_FIT_PARAMS_LONG
        if (spectrum_n, posts) == (1024, 29)
        else FLOOR1_WWISE_FIT_PARAMS
    )
    if fit_params:
        params.update(fit_params)

    order = sorted(range(posts), key=postlist.__getitem__)
    sorted_x = [postlist[index] for index in order]
    reverse_index = [0] * posts
    for sorted_pos, index in enumerate(order):
        reverse_index[index] = sorted_pos

    fits = [
        _accumulate_fit(
            fitted_floor_curve,
            raw_mdct_curve,
            sorted_x[i],
            sorted_x[i + 1],
            spectrum_n,
            params,
        )
        for i in range(posts - 1)
    ]
    # A non-empty primary bucket is the floor's nonzero-signal test.
    if sum(acc.an for acc in fits) == 0:
        return None

    fit_a = [-200] * posts
    fit_b = [-200] * posts
    local_lo = [0] * posts
    local_hi = [1] * posts
    memo = [-1] * posts

    y0, y1, _ = _fit_line(fits, -200, -200, params)
    fit_a[0] = fit_b[0] = y0
    fit_a[1] = fit_b[1] = y1

    for post_index in range(2, posts):
        sortpos = reverse_index[post_index]
        ln = local_lo[sortpos]
        hn = local_hi[sortpos]
        if memo[ln] == hn:
            continue
        lsortpos = reverse_index[ln]
        hsortpos = reverse_index[hn]
        memo[ln] = hn
        lx = postlist[ln]
        hx = postlist[hn]
        ly = _post_y(fit_a, fit_b, ln)
        hy = _post_y(fit_a, fit_b, hn)
        if _inspect_error(
            lx, hx, ly, hy, fitted_floor_curve, raw_mdct_curve, params
        ):
            ly0, ly1, ret0 = _fit_line(
                fits[lsortpos:sortpos], -200, -200, params
            )
            hy0, hy1, ret1 = _fit_line(
                fits[sortpos:hsortpos], -200, -200, params
            )
            if ret0:
                ly0, ly1 = ly, hy0
            if ret1:
                hy0, hy1 = ly1, hy
            if ret0 and ret1:
                fit_a[post_index] = fit_b[post_index] = -200
            else:
                fit_b[ln] = ly0
                if ln == 0:
                    fit_a[ln] = ly0
                fit_a[post_index] = ly1
                fit_b[post_index] = hy0
                fit_a[hn] = hy1
                if hn == 1:
                    fit_b[hn] = hy1
                if ly1 >= 0 or hy0 >= 0:
                    _propagate_high_neighbor(local_hi, sortpos, hn, post_index)
                    j = sortpos + 1
                    while j < posts and local_lo[j] == ln:
                        local_lo[j] = post_index
                        j += 1
        else:
            fit_a[post_index] = fit_b[post_index] = -200

    output = [0] * posts
    output[0] = _post_y(fit_a, fit_b, 0)
    output[1] = _post_y(fit_a, fit_b, 1)
    loneighbor, hineighbor = floor1_neighbor_tables(postlist)
    for post_index in range(2, posts):
        ln = loneighbor[post_index - 2]
        hn = hineighbor[post_index - 2]
        predicted = render_point(
            postlist[ln],
            postlist[hn],
            output[ln],
            output[hn],
            postlist[post_index],
        )
        vx = _post_y(fit_a, fit_b, post_index)
        output[post_index] = vx if vx >= 0 and predicted != vx else predicted | 0x8000
    return output


def floor1_quantize_posts(
    fit_posts: Sequence[int], multiplier: int
) -> list[int]:
    """Apply the 1024→multiplier quantization done by floor1_encode."""
    out: list[int] = []
    for post in fit_posts:
        value = post & 0x7FFF
        if multiplier == 1:
            value >>= 2
        elif multiplier == 2:
            value >>= 3
        elif multiplier == 3:
            value //= 12
        elif multiplier == 4:
            value >>= 4
        else:
            raise ValueError(f"invalid floor1 multiplier: {multiplier}")
        out.append(value | (post & 0x8000))
    return out


def floor1_fit_simple(
    mdct_mags: Sequence[float], floor: dict, n: int
) -> list[int] | None:
    """Simple floor fit: sample log-scaled mags at postlist X, quantize.

    Returns absolute post Y values (length posts), or None if energy is too low
    (caller should set nonzero=0).
    """
    mult = floor["multiplier"]
    range_ = FLOOR1_RANGES[mult]
    postlist = postlist_from_floor(floor)
    posts: list[int] = []
    energy = 0
    mlen = len(mdct_mags)
    for x in postlist:
        # postlist X is in [0, 1<<rangebits]; spectrum uses bins 0..n-1
        xi = x
        if xi >= n:
            xi = n - 1
        if xi < 0:
            xi = 0
        if xi >= mlen:
            mag = 0.0
        else:
            mag = float(mdct_mags[xi])
        q = _mag_to_quant(mag, mult, range_)
        posts.append(q)
        energy += q
    if energy == 0:
        return None
    return posts
