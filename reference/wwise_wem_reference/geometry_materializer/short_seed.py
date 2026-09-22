"""the round SHORT-family (psychoacoustic short seed) builder — analysis_geometry_builder port.

Partial port of the the paired build materializer at the build's code.
ATH/octave retain their surrounding arithmetic and use supplied CRT log/exp
value implementations. Interval uses explicit x87 64-significand operations
and the supplied CRT atan value implementation.

SOURCES:
  - constants: read-only data read via the analysis tool (see constants in this module)
  - paired_ath_source_curve (88 f32): ATH source curve (read from the analysis tool)
  - verification only: registered short-seed.json (read by the verification runner)

ARGUMENTS (analysis_geometry_builder(a1,a2,a3,a4,a5), the argument block):
  a1 = out struct (the 5 surfaces live here)
  a2 = psychoacoustic config (a1[1]; A-array at +132..+340)
  a3 = integer scale pointer (v3+717); actual value remains unresolved
  a4 = n (128 short bins)
  a5 = sample rate (44100 = 6ch static-family, 48000 = 2ch record-family)

SURFACE BINDINGS (a1 offset -> out field, confirmed from disasm stores):
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
# Constants (the paired build read-only data, f64 where noted)
# ----------------------------------------------------------------------------
LOG2E = 1.4426950216293335  # paired_log2e
PSYCHO = 5.965784072875977  # paired_psycho
LN2 = 0.6931470036506653  # paired_ln2
HALF = 0.5  # paired_half
QUARTER = 0.25  # paired_quarter
ATH_OFF = 100.0  # paired_ath_offset
TWO = 2.0  # paired_two

# paired_ath_source_curve: ATH source curve (88 f32, read from the analysis tool)
paired_ath_source_curve = [
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
# out.octave = a1[5]  (the build's code..the build's code)  -> VERIFIED 128/128 on 6ch
# ----------------------------------------------------------------------------
# x87 loop: for i in 0..a4:
#   st0 = i + 0.25                (fld i; fadd paired_quarter)
#   st0 = st0 * 0.5               (fmulp st(1),st  where st(1)=paired_half=0.5)
#   st0 = st0 * a5                (fmul var_18; var_18 = a5 from fild arg_10)
#   st0 = st0 / a4                (fdiv var_20; var_20 = a4 from fild arg_C)
#   st0 = log(st0)                (call _CIlog)
#   st0 = st0 * LOG2E             (fmul paired_log2e)
#   st0 = st0 - PSYCHO            (fsub paired_psycho)
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
    r = a5 / a4  # var_18/var_20
    out = []
    for i in range(a4):
        arg = (i + QUARTER) * 0.5 * r
        v = (f32.cilog(arg) * LOG2E - PSYCHO) * S + HALF
        out.append(_trunc_zero(v))
    return out


# ----------------------------------------------------------------------------
# out.ath = a1[4]  (the build's code..the build's code), supplied CRT exp value path
# ----------------------------------------------------------------------------
# For v42 in 0..86:
#   v12 = exp(((v42+1)*0.125 - 2.0 + PSYCHO) * LN2)   # = 2^((v42+1)/8 + 3.9658)
#   v13 = floor(2*v12*a4/a5 + 0.5)                    # segment end bin
#   slope = (paired_ath_source_curve[v42+1] - paired_ath_source_curve[v42]) / (v13 - v11)
#   fill bins [v11, v13) with f32(paired_ath_source_curve[v42] + 100 + k*slope), rounding at each
#   store. v11 advances to v13.
def ath(a4, a5):
    buf = [0.0] * a4
    v11 = 0
    # the build's code..100163ef: compare incremented index with 0x57 (87).
    # paired_ath_source_curve[87] = -18.0 verified in the paired build at the corresponding locations.
    for v42 in range(87):
        v12 = f32.ciexp(((v42 + 1) * 0.125 - TWO + PSYCHO) * LN2)
        v13 = _floor_integer(2.0 * v12 * a4 / a5 + HALF)  # full (may exceed a4)
        v46 = f32_round(paired_ath_source_curve[v42])
        if v11 < v13:
            # slope uses the FULL segment length, even when the store is clamped to a4
            slope = f32_round((paired_ath_source_curve[v42 + 1] - v46) / (v13 - v11))
            run = v46
            for _ in range(min(v13, a4) - v11):
                buf[v11] = f32_round(run + ATH_OFF)
                run = f32_round(run + slope)
                v11 += 1
    # the build's code..10016483: repeat last stored value first, then advance by
    # the last difference, with an f32 store/reload after each addition.
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
    """Packed interval endpoints, analysis_geometry_builder @ the build's code..the build's code."""
    # the build's code/10015efc: persistent cursors, initialized before ATH.
    v57, v58 = -99, 1
    # the build's code..100164af: signed integer division, NOT sample-rate fdiv.
    v26 = a5 // (2 * a4)
    v55 = v26 * v26
    v68 = v65 = 0
    out = []

    def mapped(linear, squared):
        # the build's code..10016508; repeated at 1001657d..100165c5,
        # 10016658..100166a0: f32 arguments/results; scaled first atan is f64.
        x = f32_round(linear)
        arg = f32_round(f32.mul80(squared, 1.8499999754340024e-08))
        first = f32.to_float(f32.mul80(f32_round(f32.ciatan(arg)), 2.240000009536743))
        arg = f32_round(f32.mul80(x, 0.0007399999885819852))
        second = f32_round(f32.ciatan(arg))
        # the build's code..10016537 / 100165cd..100165e3 / 100166ab..100166bf.
        return f32.add80(
            f32.add80(f32.mul80(second, 13.100000381469727), first),
            f32.mul80(x, 9.999999747378752e-05),
        )

    # a2+112/116: 0.5f/0.5f static seed the build's code (r7 byte proof: untouched).
    # a2+120/124: seed 0/0; setter-chain conditional value 3/3 (r4 audit
    # work/the round-audit.json; the byte comparison adjudicates, no fitting).
    # LONG uses the same loop with its mode-indexed integer pair.
    for v43 in range(a4):
        v92 = f32_round(mapped(v68, v43 * v65))  # the build's code: fstp f32.
        if a2_120 + v57 < v43:  # the build's code: a2+120 + v57 < v43.
            v75 = a2_120 + v57  # the build's code
            v63, v60 = v57 * v26, v57 * v55
            while True:
                # the build's code: threshold f64; 100165ec: ordered >= breaks.
                if mapped(v63, v57 * v60) >= v92 - 0.5:
                    break
                v60 += v55
                v57 += 1
                v63 += v26
                v75 += 1
                if v75 >= v43:  # the build's code: strict < continues.
                    break
        v28 = v58  # the build's code: persists after upper cursor passes a4.
        if v58 <= a4:  # the build's code.
            v61, v64 = v58 * v26, v58 * v55
            while True:
                if v28 >= v43 + a2_124:  # the build's code: a2+124.
                    # the build's code..100166d3: threshold <= map breaks.
                    if 0.5 + v92 <= mapped(v61, v58 * v64):
                        break
                v64 += v55
                v61 += v26
                v28 += 1
                v58 = v28
                if v28 > a4:  # the build's code: inclusive <= continues.
                    break
        # the build's code..10016719: packed subtraction borrows across halves.
        v68 += v26
        out.append(((v57 << 16) + v28 - 65537) & 0xFFFFFFFF)
        v65 += v55
    return out


def default_quality_index():
    """VBR default through ebd0 -> e2b0 -> ea80, not d0f0's record curve."""
    # the build's code: default 4.0f; 10008933: /10; 893b/893f: f32 before ebd0.
    q = f32_round(4.0 / 10.0)
    # the build's code/ebe4: double epsilon at the build's code, then fstps.
    q = f32_round(f32.add80(q, 1e-7))
    # the build's code/e33c..e3c6: descriptor+8, the corresponding locations.
    axis = (-0.2, -0.1, 0.0, 0.1, 0.2, 0.3, 0.4, 0.5, 0.6, 0.7, 0.8, 0.9, 1.0)
    g = next(i for i in range(12) if axis[i] <= q <= axis[i + 1])
    # the build's code/e407: both endpoints are stored as f32.
    lo, hi = f32_round(axis[g]), f32_round(axis[g + 1])
    # the build's code/e413/e41b/e41f: divide, fstps, add integer, fstpl.
    from fractions import Fraction

    fraction = f32_round(f32._round80(Fraction(q - lo) / Fraction(hi - lo)))
    # ea80 eb47/eb6b copy this index; e8bb/e8d8 load it for the mode bank.
    return f32.to_float(f32.add80(g, fraction))


def mask_knots(raw, index, bias=0.0):
    """The mode bank's 3 x 17 signed source words, interpolated and floor-clamped."""
    # 1000d940/d94c: __ftol2_sse (10023412/415: fstpl + cvttsd2si).
    g = f32.to_int(f32.to_float(index))
    # 1000d967: fisubl integer, retaining x87 precision.
    frac = f32.add80(index, -g)
    complement = f32.add80(1, -frac)  # 1000d974/976.
    rows = []
    for row in range(3):
        values = []
        for j in range(17):
            # 1000d96e/d9bc: quality stride 0xcc; daa2: row stride 0x44.
            at = g * 51 + row * 17 + j
            left, right = family._s32(raw[at]), family._s32(raw[at + 51])
            # d9d6..da98: fildl; separate fmuls; faddp; fstps.
            values.append(f32_round(f32.add80(f32.mul80(left, complement), f32.mul80(right, frac))))
        # dac9/dad6/dad8: f64 6.0 at 10184d20, first knot + 6 -> f32.
        floor = f32_round(f32.add80(values[0], 6.0))
        # dae2/dae4/daec: add bias -> f32; daee/daf5/daf7: floor clamp.
        rows.append([max(f32_round(f32.add80(v, bias)), floor) for v in values])
    return rows


def mask_curves(a4, a5, knots):
    """1001681b..1001696e: bin-center position and three knot lerps."""
    rows: list[list[float]] = [[], [], []]
    for i in range(a4):
        # 1001681f/825/829/82d: (i+0.5)*rate/(2*n), _CIlog.
        logarithm = f32.cilog((i + HALF) * a5 / (2 * a4))
        # 10016832/838/83e/840: LOG2E, PSYCHO, *2, fstps.
        position = f32_round(f32.mul80(f32.add80(f32.mul80(logarithm, LOG2E), -PSYCHO), 2))
        position = min(16.0, max(0.0, position))  # 10016844..8d2.
        g = f32.to_int(position)  # 100168d6.
        frac = f32_round(f32.add80(position, -g))  # 100168e2/8ed.
        complement = f32.add80(1, -frac)  # 100168f7/8f9.
        for row, out in zip(knots, rows):
            # 100168fb..1696e; endpoint's following load has zero weight.
            right = row[g + 1] if g < 16 else 0.0
            out.append(f32_round(f32.add80(f32.mul80(right, frac), f32.mul80(row[g], complement))))
    return rows


def build(d, key, geo):
    """Return {field: [u32]} matching the rosetta short_seed_fields encoding.

    Encodings (from rosetta.short_seed_fields):
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
    """the build's code/10010b82: a5 is an explicit geometry input, not a surface read."""
    return {(6, 40000, 70000): 44100, (2, 45000, 50000): 48000}[tuple(key)]
