"""SHORT-family (psychoacoustic short seed) geometry, ported from the paired
build's geometry builder.

Partial port of the paired build's geometry builder. ATH/octave retain their
surrounding arithmetic and use supplied CRT log/exp value implementations.
Interval uses explicit x87 64-significand operations and the supplied CRT atan
value implementation.

SOURCES:
  - constants: the paired build's geometry constants (see this module)
  - ATH source curve (88 f32)
  - verification only: the registered short-seed surfaces (read by the
    verification runner)

ARGUMENTS (the builder's a1,a2,a3,a4,a5):
  a1 = out struct (the 5 surfaces live here)
  a2 = psychoacoustic config (a1[1]; A-array at +132..+340)
  a3 = integer scale pointer (v3+717); actual value remains unresolved
  a4 = n (128 short bins)
  a5 = sample rate (44100 = 6ch static-family, 48000 = 2ch record-family)

SURFACE BINDINGS (a1 offset -> out field, established from the builder's stores):
  a1[3] (+0x0C) -> mask_curves (3x128 f32); out.mask_curve = a1[3][1]
  a1[4] (+0x10) -> ath (128 f32)
  a1[5] (+0x14) -> octave (128 i32)
  a1[6] (+0x18) -> interval_table (128 i32)

STATUS: All five independent 6ch surfaces match byte for byte. The same
static default binding is regenerated at 48000 for the 2ch comparison.
"""

from __future__ import annotations

import sys
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parents[1]))
from . import f32_primitives as f32
from . import inputs as family

TARGET = "short_seed"

# ----------------------------------------------------------------------------
# Constants (the paired build's f64 literals)
# ----------------------------------------------------------------------------
LOG2E = 1.4426950216293335
PSYCHO = 5.965784072875977
LN2 = 0.6931470036506653
HALF = 0.5
QUARTER = 0.25
ATH_OFF = 100.0
TWO = 2.0

# ATH source curve (88 f32)
ATH_SOURCE_CURVE = [
    -31.0,
    -33.0,
    -35.0,
    -37.0,
    -39.0,
    -41.0,
    -43.0,
    -45.0,
    -47.0,
    -49.0,
    -51.0,
    -53.0,
    -55.0,
    -57.0,
    -59.0,
    -61.0,
    -63.0,
    -65.0,
    -67.0,
    -69.0,
    -71.0,
    -73.0,
    -75.0,
    -77.0,
    -79.0,
    -81.0,
    -83.0,
    -84.0,
    -85.0,
    -86.0,
    -87.0,
    -88.0,
    -89.0,
    -90.0,
    -91.0,
    -92.0,
    -93.0,
    -94.0,
    -95.0,
    -96.0,
    -96.0,
    -97.0,
    -97.0,
    -97.0,
    -98.0,
    -98.0,
    -98.0,
    -99.0,
    -98.0,
    -97.0,
    -97.0,
    -98.0,
    -99.0,
    -100.0,
    -101.0,
    -101.0,
    -102.0,
    -103.0,
    -104.0,
    -105.0,
    -106.0,
    -106.0,
    -107.0,
    -107.0,
    -105.0,
    -104.0,
    -103.0,
    -102.0,
    -101.0,
    -99.0,
    -98.0,
    -97.0,
    -96.0,
    -95.0,
    -95.0,
    -96.0,
    -97.0,
    -97.0,
    -93.0,
    -89.0,
    -80.0,
    -70.0,
    -50.0,
    -40.0,
    -30.0,
    -26.0,
    -22.0,
    -18.0,
]


def f32_bits(x):
    return f32.to_f32_bits(x)


def f32_round(x):
    """Re-round f64 -> f32 -> f64 (models x87 fstp/fld double-round)."""
    return f32.to_f32_store(x)


# ----------------------------------------------------------------------------
# out.octave = a1[5]  -> VERIFIED 128/128 on 6ch
# ----------------------------------------------------------------------------
# x87 loop: for i in 0..a4:
#   st0 = i + 0.25                (fld i; fadd the f64 0.25 literal)
#   st0 = st0 * 0.5               (fmulp st(1),st  where st(1)=0.5)
#   st0 = st0 * a5                (fmul; the f64 a5 loaded from the integer arg)
#   st0 = st0 / a4                (fdiv; the f64 a4 loaded from the integer arg)
#   st0 = log(st0)                (call _CIlog)
#   st0 = st0 * LOG2E             (fmul)
#   st0 = st0 - PSYCHO            (fsub)
#   st0 = st0 * S                 (fmulp; S = 1 << (a1[8]+1) via shl)
#   st0 = st0 + 0.5               (fld 0.5; fadd)
#   out[i] = trunc_toward_zero(st0)   (call __ftol2_sse -> mov)
def _trunc_zero(x):
    """Truncate finite builder inputs; not an x87 exception/flag emulator."""
    return f32.to_int(x)


def _floor_integer(x):
    """Floor finite builder inputs using integer truncation and correction."""
    value = f32.to_int(x)
    return value - (x < value)


def octave(a4, a5, a1_8=5):
    S = 1 << (a1_8 + 1)
    r = a5 / a4  # the two integer arguments, as f64
    out = []
    for i in range(a4):
        arg = (i + QUARTER) * 0.5 * r
        v = (f32.cilog(arg) * LOG2E - PSYCHO) * S + HALF
        out.append(_trunc_zero(v))
    return out


# ----------------------------------------------------------------------------
# out.ath = a1[4], supplied CRT exp value path
# ----------------------------------------------------------------------------
# For v42 in 0..86:
#   v12 = exp(((v42+1)*0.125 - 2.0 + PSYCHO) * LN2)   # = 2^((v42+1)/8 + 3.9658)
#   v13 = floor(2*v12*a4/a5 + 0.5)                    # segment end bin
#   slope = (ATH_SOURCE_CURVE[v42+1] - ATH_SOURCE_CURVE[v42]) / (v13 - v11)
#   fill bins [v11, v13) with f32(ATH_SOURCE_CURVE[v42] + 100 + k*slope),
#   rounding at each store. v11 advances to v13.
def ath(a4, a5):
    buf = [0.0] * a4
    v11 = 0
    # Compare the incremented index with 0x57 (87).
    # ATH_SOURCE_CURVE[87] = -18.0, verified against the paired build.
    for v42 in range(87):
        v12 = f32.ciexp(((v42 + 1) * 0.125 - TWO + PSYCHO) * LN2)
        v13 = _floor_integer(2.0 * v12 * a4 / a5 + HALF)  # full (may exceed a4)
        v46 = f32_round(ATH_SOURCE_CURVE[v42])
        if v11 < v13:
            # slope uses the FULL segment length, even when the store is clamped to a4
            slope = f32_round((ATH_SOURCE_CURVE[v42 + 1] - v46) / (v13 - v11))
            run = v46
            for _ in range(min(v13, a4) - v11):
                buf[v11] = f32_round(run + ATH_OFF)
                run = f32_round(run + slope)
                v11 += 1
    # Repeat last stored value first, then advance by the last difference,
    # with an f32 store/reload after each addition.
    if v11 < a4:
        if v11 < 2:
            raise ValueError("ATH tail requires two materialized bins")
        run = buf[v11 - 1]
        slope = f32_round(run - buf[v11 - 2])
        for i in range(v11, a4):
            buf[i] = run
            run = f32_round(run + slope)
    return buf


def interval_table(a4, a5, *, a2_120=3, a2_124=3):
    """Packed interval endpoints, ported from the geometry builder."""
    # Persistent cursors, initialized before ATH.
    v57, v58 = -99, 1
    # Signed integer division, NOT sample-rate fdiv.
    v26 = a5 // (2 * a4)
    v55 = v26 * v26
    v68 = v65 = 0
    out = []

    def mapped(linear, squared):
        # Expanded at three sites in the builder: f32 arguments/results; the
        # scaled first atan is f64.
        x = f32_round(linear)
        arg = f32_round(f32.mul80(squared, 1.8499999754340024e-08))
        first = f32.to_float(f32.mul80(f32_round(f32.ciatan(arg)), 2.240000009536743))
        arg = f32_round(f32.mul80(x, 0.0007399999885819852))
        second = f32_round(f32.ciatan(arg))
        return f32.add80(
            f32.add80(f32.mul80(second, 13.100000381469727), first),
            f32.mul80(x, 9.999999747378752e-05),
        )

    # a2+112/116: 0.5f/0.5f static seed, untouched in the registered bytes.
    # a2+120/124: seed 0/0; the setter chain yields a conditional 3/3 here.
    # The byte comparison adjudicates, so nothing is fitted.
    # LONG uses the same loop with its mode-indexed integer pair.
    for v43 in range(a4):
        v92 = f32_round(mapped(v68, v43 * v65))  # fstp f32.
        if a2_120 + v57 < v43:
            v75 = a2_120 + v57
            v63, v60 = v57 * v26, v57 * v55
            while True:
                # Threshold is f64; an ordered >= breaks.
                if mapped(v63, v57 * v60) >= v92 - 0.5:
                    break
                v60 += v55
                v57 += 1
                v63 += v26
                v75 += 1
                if v75 >= v43:  # strict < continues.
                    break
        v28 = v58  # persists after the upper cursor passes a4.
        if v58 <= a4:
            v61, v64 = v58 * v26, v58 * v55
            while True:
                if v28 >= v43 + a2_124:  # a2+124.
                    # An ordered <= against the map breaks.
                    if 0.5 + v92 <= mapped(v61, v58 * v64):
                        break
                v64 += v55
                v61 += v26
                v28 += 1
                v58 = v28
                if v28 > a4:  # inclusive <= continues.
                    break
        # Packed subtraction borrows across halves.
        v68 += v26
        out.append(((v57 << 16) + v28 - 65537) & 0xFFFFFFFF)
        v65 += v55
    return out


def default_quality_index():
    """VBR default through the profile-select chain, not the record curve."""
    # Default 4.0f, divided by 10 and f32-rounded before the select chain.
    q = f32_round(4.0 / 10.0)
    # Double epsilon, then fstps.
    q = f32_round(f32.add80(q, 1e-7))
    # The quality axis lives at descriptor+8.
    axis = (-0.2, -0.1, 0.0, 0.1, 0.2, 0.3, 0.4, 0.5, 0.6, 0.7, 0.8, 0.9, 1.0)
    g = next(i for i in range(12) if axis[i] <= q <= axis[i + 1])
    # Both endpoints are stored as f32.
    lo, hi = f32_round(axis[g]), f32_round(axis[g + 1])
    # Divide, fstps, add integer, fstpl.
    from fractions import Fraction

    fraction = f32_round(f32._round80(Fraction(q - lo) / Fraction(hi - lo)))
    # The select chain copies this index; the knot writer loads it.
    return f32.to_float(f32.add80(g, fraction))


def mask_knots(raw, index, bias=0.0):
    """The 3 x 17 signed source words, interpolated and floor-clamped."""
    # __ftol2_sse: fstpl + cvttsd2si.
    g = f32.to_int(f32.to_float(index))
    # fisubl integer, retaining x87 precision.
    frac = f32.add80(index, -g)
    complement = f32.add80(1, -frac)
    rows = []
    for row in range(3):
        values = []
        for j in range(17):
            # quality stride 0xcc bytes; row stride 0x44 bytes.
            at = g * 51 + row * 17 + j
            left, right = family._s32(raw[at]), family._s32(raw[at + 51])
            # fildl; separate fmuls; faddp; fstps.
            values.append(f32_round(f32.add80(f32.mul80(left, complement), f32.mul80(right, frac))))
        # The f64 6.0 is added to the first knot, then stored as f32.
        floor = f32_round(f32.add80(values[0], 6.0))
        # Add the bias -> f32; then the floor clamp.
        rows.append([max(f32_round(f32.add80(v, bias)), floor) for v in values])
    return rows


def mask_curves(a4, a5, knots):
    """Bin-center position and three knot lerps."""
    rows: list[list[float]] = [[], [], []]
    for i in range(a4):
        # (i+0.5)*rate/(2*n), through _CIlog.
        logarithm = f32.cilog((i + HALF) * a5 / (2 * a4))
        # LOG2E, PSYCHO, *2, fstps.
        position = f32_round(f32.mul80(f32.add80(f32.mul80(logarithm, LOG2E), -PSYCHO), 2))
        position = min(16.0, max(0.0, position))
        g = f32.to_int(position)
        frac = f32_round(f32.add80(position, -g))
        complement = f32.add80(1, -frac)
        for row, out in zip(knots, rows):
            # The endpoint's following load has zero weight.
            right = row[g + 1] if g < 16 else 0.0
            out.append(f32_round(f32.add80(f32.mul80(right, frac), f32.mul80(row[g], complement))))
    return rows


def build(d, key, geo):
    """Return {field: [u32]} matching the registered short_seed field encodings.

    Encodings:
      ath            -> f32 bits (x87 stores via fstp; f32)
      octave         -> u32 of i32 (x87 stores via mov eax after __ftol2_sse; int)
      look.mask_curve     -> f32 bits (config-dependent; a1[3][1] / mask_curves[1])
      look.interval_table -> u32 of i32 (config-dependent; a1[6])

    Both geometries (44100 and 48000) are byte-exact. The per-profile rows
    live in :func:`profile_mask_curves`; the seed surfaces above are shared.
    """
    a4 = 128
    a5 = _sample_rate(key)
    if tuple(geo) != tuple(key):
        raise ValueError("descriptor geometry does not match requested key")
    # a1[8] is the derived exponent, NOT *a3; the latter remains unresolved.
    a1_8 = 5

    out = {}
    out["ath"] = [f32_bits(x) for x in ath(a4, a5)]
    out["octave"] = [f32.to_int(x) & 0xFFFFFFFF for x in octave(a4, a5, a1_8)]
    out["look.interval_table"] = interval_table(a4, a5)
    knots = profile_knots(d, key, PROFILE_POOLS[0], a5)
    rows = mask_curves(a4, a5, knots)
    out["mask_curves"] = [f32_bits(v) for row in rows for v in row]
    out["look.mask_curve"] = [f32_bits(v) for v in rows[1]]
    return out


#: SHORT profile index -> descriptor knot-pool slot. Profile 0 hosts
#: ``look.mask_curve``/``mask_curves`` and profile 1 the second curve row set;
#: both pools are byte-identical in the paired build's two descriptor copies.
PROFILE_POOLS = ("mask_pool_0", "mask_pool_1")


def profile_knots(d, key, slot, a5):
    """Three knot rows for one SHORT profile: pool lerp at the default quality."""
    if a5 not in (44100, 48000):
        raise ValueError("unsupported geometry sample rate")
    return mask_knots(family.slot_u32(d, key, slot), default_quality_index())


def profile_mask_curves(d, key, geo, profile_index):
    """Registered per-profile SHORT floor rows as f32 bits (3 x 128).

    Profile 0 reads ``mask_pool_0``, profile 1 ``mask_pool_1``; both use the
    same default quality index and zero bias, and both reproduce the
    registered 6ch authority byte-for-byte.
    """
    if tuple(geo) != tuple(key):
        raise ValueError("descriptor geometry does not match requested key")
    if not 0 <= profile_index < len(PROFILE_POOLS):
        raise ValueError("short profile index is outside the pool table")
    a5 = _sample_rate(key)
    rows = mask_curves(128, a5, profile_knots(d, key, PROFILE_POOLS[profile_index], a5))
    return [f32_bits(v) for row in rows for v in row]


def _sample_rate(key):
    """a5 is an explicit geometry input, not a surface read."""
    return {(6, 40000, 70000): 44100, (2, 45000, 50000): 48000}[tuple(key)]
