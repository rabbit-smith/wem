#!/usr/bin/env python3
"""Float32-compatible MDCT, analysis windows, and block extraction."""
from __future__ import annotations

import math
import struct
from typing import Sequence

from ..config import MdctLook, TransientDetectorTables

def _f32(value: float) -> float:
    return struct.unpack("<f", struct.pack("<f", float(value)))[0]


def _u32_f32(value: int) -> float:
    """Interpret a stored unsigned 32-bit word as IEEE-754 float32."""
    return struct.unpack("<f", struct.pack("<I", int(value) & 0xFFFFFFFF))[0]


# The analysis path reads a float32 window table.  The analytic expression used below
# reproduces its 256-point block except the endpoint, where the reference encoder's stored
# value is seven float32 ULPs below a host ``sin`` result.  Keeping that table
# bit makes both edges of the short packet-1 window reproducible.
WWISE_SHORT_WINDOW_ENDPOINT_F32 = struct.unpack("<f", bytes.fromhex("050c7838"))[0]

# The 2048-point Vorbis window is likewise table-backed.  For its first 96
# coefficients the analytic expression agrees at every multiplier-relevant
# bit except the 19 entries below.  The values were recovered from the six
# independent PCM/frame products in the reference short→long window at spectrum transform:
# each word is the unique float32 product factor that reproduces all six
# stored frame samples.  Keeping the whole prefix, rather than only the 19
# differing entries, gives the table boundary a compact and auditable form.
WWISE_LONG_WINDOW_HEAD_U32 = (
    0x35780FAB, 0x370B8718, 0x37C1C9E1, 0x383DE96B, 0x389CF780, 0x38EA7ABB, 0x3923BF18, 0x395A00D5,
    0x398C0138, 0x39AEE1E5, 0x39D5A258, 0x3A00213C, 0x3A176118, 0x3A3090AF, 0x3A4BAFF3, 0x3A68BED4,
    0x3A83DE9F, 0x3A94558F, 0x3AA5C430, 0x3AB82A77, 0x3ACB885A, 0x3ADFDDCC, 0x3AF52ABF, 0x3B05B794,
    0x3B11557C, 0x3B1D6F10, 0x3B2A0449, 0x3B37151F, 0x3B44A18A, 0x3B52A981, 0x3B612CFC, 0x3B702BF1,
    0x3B7FA658, 0x3B87CE13, 0x3B9006A9, 0x3B987CE9, 0x3BA130CC, 0x3BAA224F, 0x3BB3516A, 0x3BBCBE1A,
    0x3BC66856, 0x3BD0501A, 0x3BDA755F, 0x3BE4D81F, 0x3BEF7853, 0x3BFA55F4, 0x3C02B87E, 0x3C0864B1,
    0x3C0E2F91, 0x3C141919, 0x3C1A2146, 0x3C204813, 0x3C268D7E, 0x3C2CF181, 0x3C337419, 0x3C3A1541,
    0x3C40D4F6, 0x3C47B332, 0x3C4EAFF3, 0x3C55CB32, 0x3C5D04EB, 0x3C645D1A, 0x3C6BD3BA, 0x3C7368C6,
    0x3C7B1C3A, 0x3C817707, 0x3C856F21, 0x3C897666, 0x3C8D8CD4, 0x3C91B269, 0x3C95E721, 0x3C9A2AFB,
    0x3C9E7DF3, 0x3CA2E006, 0x3CA75132, 0x3CABD173, 0x3CB060C7, 0x3CB4FF2B, 0x3CB9AC9A, 0x3CBE6913,
    0x3CC33492, 0x3CC80F14, 0x3CCCF895, 0x3CD1F113, 0x3CD6F889, 0x3CDC0EF5, 0x3CE13453, 0x3CE668A0,
    0x3CEBABD7, 0x3CF0FDF6, 0x3CF65EF9, 0x3CFBCEDC, 0x3D00A6CD, 0x3D036D99, 0x3D063BCF, 0x3D09116D,
)



def make_mdct_look(
    n: int, *, static_trig: Sequence[float] | None = None
) -> MdctLook:
    if n < 64 or n & (n - 1):
        raise ValueError("MDCT size must be a power of two >= 64")
    log2n = n.bit_length() - 1
    if static_trig is not None:
        if len(static_trig) != n + n // 4:
            raise ValueError("static MDCT trig bank has the wrong length")
        trig = list(static_trig)
    else:
        # Retain the reference libvorbis construction for geometries that
        # the Wwise binary has no static bank for.
        trig = [0.0] * (n + n // 4)
        for i in range(n // 4):
            trig[2 * i] = _f32(math.cos(math.pi / n * (4 * i)))
            trig[2 * i + 1] = _f32(-math.sin(math.pi / n * (4 * i)))
            trig[n // 2 + 2 * i] = _f32(
                math.cos(math.pi / (2 * n) * (2 * i + 1))
            )
            trig[n // 2 + 2 * i + 1] = _f32(
                math.sin(math.pi / (2 * n) * (2 * i + 1))
            )
        for i in range(n // 8):
            trig[n + 2 * i] = _f32(
                math.cos(math.pi / n * (4 * i + 2)) * 0.5
            )
            trig[n + 2 * i + 1] = _f32(
                -math.sin(math.pi / n * (4 * i + 2)) * 0.5
            )

    mask = (1 << (log2n - 1)) - 1
    msb = 1 << (log2n - 2)
    bitrev: list[int] = []
    for i in range(n // 8):
        acc = 0
        for j in range(log2n):
            if (msb >> j) & i:
                acc |= 1 << j
        bitrev.extend((((~acc) & mask) - 1, acc))
    return MdctLook(n, log2n, tuple(trig), tuple(bitrev), _f32(4.0 / n))


def vorbis_window(n: int) -> list[float]:
    """Symmetric same-size Vorbis window from the converter's f32 table."""
    if n < 2 or n & 1:
        raise ValueError("window size must be a positive even number")
    half = n // 2
    left = [
        _f32(math.sin(math.pi / 2.0 * math.sin(math.pi * (i + 0.5) / n) ** 2))
        for i in range(half)
    ]
    if n == 256:
        left[0] = WWISE_SHORT_WINDOW_ENDPOINT_F32
    elif n == 2048:
        left[: len(WWISE_LONG_WINDOW_HEAD_U32)] = [
            _u32_f32(value) for value in WWISE_LONG_WINDOW_HEAD_U32
        ]
    return left + left[::-1]


def wwise_psy_window(n: int, tables: TransientDetectorTables) -> list[float]:
    """Return the the psychoacoustic psycho-analysis window (sin(pi*i/(n-1)))²."""
    if n < 2:
        raise ValueError("psycho window needs at least two samples")
    # ``transient analysis`` does not synthesize this surface: it multiplies by the
    # 128 stored float words at look+36.  The analytic form is close but
    # differs at 86 words, enough to drift its detector histories.
    if n == 128:
        if tables.n != n:
            raise ValueError("transient window geometry differs from MDCT size")
        return list(tables.window)
    return [
        _f32(math.sin(math.pi * i / (n - 1)) ** 2) for i in range(n)
    ]


def wwise_psy_mdct(
    samples: Sequence[float],
    *,
    look: MdctLook,
    tables: TransientDetectorTables,
) -> list[float]:
    """Run the ``transient analysis → spectrum transform`` psycho spectrum front-end."""
    n = len(samples)
    if look.n != n:
        raise ValueError("transient MDCT look geometry differs from samples")
    windowed = [_f32(x * w) for x, w in zip(samples, wwise_psy_window(n, tables))]
    return mdct_forward(look, windowed)


def apply_vorbis_window(
    samples: Sequence[float],
    blocksizes: Sequence[int],
    previous: int,
    current: int,
    following: int,
) -> list[float]:
    """Apply the encoder-side Vorbis hybrid window to one block.

    This is the straight-line behavior of the reference encoder helper:
    zero the non-overlap regions, multiply the left transition by the
    previous block window, and multiply the right transition in reverse by
    the following block window.
    """
    n = blocksizes[current]
    left_n = blocksizes[previous]
    right_n = blocksizes[following]
    if len(samples) < n:
        raise ValueError(f"need {n} samples, got {len(samples)}")
    if any(size < 2 or size & 1 for size in blocksizes):
        raise ValueError("block sizes must be positive even numbers")
    if not all(0 <= index < len(blocksizes) for index in (previous, current, following)):
        raise ValueError("window state index out of range")

    left_begin = n // 4 - left_n // 4
    left_end = left_begin + left_n // 2
    right_begin = n // 2 + n // 4 - right_n // 4
    right_end = right_begin + right_n // 2
    if not (0 <= left_begin <= left_end <= right_begin <= right_end <= n):
        raise ValueError("incompatible block/window sizes")

    windows = {size: vorbis_window(size) for size in {left_n, right_n}}
    out = list(samples[:n])
    for i in range(left_begin):
        out[i] = 0.0
    for i in range(left_begin, left_end):
        out[i] = _f32(out[i] * windows[left_n][i - left_begin])
    for i in range(right_begin, right_end):
        out[i] = _f32(out[i] * windows[right_n][right_n // 2 - 1 - (i - right_begin)])
    for i in range(right_end, n):
        out[i] = 0.0
    return out


def extract_analysis_block(
    buffered: Sequence[float], cursor: int, current_size: int
) -> list[float]:
    """Return the block view exposed by reference routine.

    The feeder keeps ``cursor`` at the center of the current window; the
    encoder consumes ``current_size`` samples around that center.
    """
    if current_size < 2 or current_size & 1:
        raise ValueError("current block size must be a positive even number")
    start = cursor - current_size // 2
    end = start + current_size
    if start < 0 or end > len(buffered):
        raise ValueError("buffer does not contain the requested analysis block")
    return list(buffered[start:end])




def _b8(x: list[float], base: int) -> None:
    # These stages use C ``float`` locals for every butterfly difference
    # (visible as fstp dword stack slots).  The products below remain floating-point
    # expressions until their destination stores, but these inputs do not.
    r0 = _f32(x[base + 6] + x[base + 2])
    r1 = _f32(x[base + 6] - x[base + 2])
    r2 = _f32(x[base + 4] + x[base])
    r3 = _f32(x[base + 4] - x[base])
    x[base + 6] = _f32(r0 + r2)
    x[base + 4] = _f32(r0 - r2)
    r0 = _f32(x[base + 5] - x[base + 1])
    r2 = _f32(x[base + 7] - x[base + 3])
    x[base] = _f32(r1 + r0)
    x[base + 2] = _f32(r1 - r0)
    r0 = _f32(x[base + 5] + x[base + 1])
    r1 = _f32(x[base + 7] + x[base + 3])
    x[base + 3] = _f32(r2 + r3)
    x[base + 1] = _f32(r2 - r3)
    x[base + 7] = _f32(r1 + r0)
    x[base + 5] = _f32(r1 - r0)


def _b16(x: list[float], base: int) -> None:
    r0 = _f32(x[base + 1] - x[base + 9])
    r1 = _f32(x[base] - x[base + 8])
    x[base + 8] = _f32(x[base + 8] + x[base])
    x[base + 9] = _f32(x[base + 9] + x[base + 1])
    x[base] = _f32((r0 + r1) * 0.7071067690849304)
    x[base + 1] = _f32((r0 - r1) * 0.7071067690849304)
    r0 = _f32(x[base + 3] - x[base + 11])
    r1 = _f32(x[base + 10] - x[base + 2])
    x[base + 10] = _f32(x[base + 10] + x[base + 2])
    x[base + 11] = _f32(x[base + 11] + x[base + 3])
    x[base + 2] = _f32(r0)
    x[base + 3] = _f32(r1)
    r0 = _f32(x[base + 12] - x[base + 4])
    r1 = _f32(x[base + 13] - x[base + 5])
    x[base + 12] = _f32(x[base + 12] + x[base + 4])
    x[base + 13] = _f32(x[base + 13] + x[base + 5])
    x[base + 4] = _f32((r0 - r1) * 0.7071067690849304)
    x[base + 5] = _f32((r0 + r1) * 0.7071067690849304)
    r0 = _f32(x[base + 14] - x[base + 6])
    r1 = _f32(x[base + 15] - x[base + 7])
    x[base + 14] = _f32(x[base + 14] + x[base + 6])
    x[base + 15] = _f32(x[base + 15] + x[base + 7])
    x[base + 6] = _f32(r0)
    x[base + 7] = _f32(r1)
    _b8(x, base)
    _b8(x, base + 8)


def _b32(x: list[float], base: int) -> None:
    # Constants mirror libvorbis butterfly32; its s3 branch is unused here.
    c1 = 0.9238795042037964
    s1 = 0.3826834261417389
    c3 = 0.3826834261417389
    s2 = 0.7071067690849304
    r0 = _f32(x[base + 30] - x[base + 14])
    r1 = _f32(x[base + 31] - x[base + 15])
    x[base + 30] = _f32(x[base + 30] + x[base + 14])
    x[base + 31] = _f32(x[base + 31] + x[base + 15])
    x[base + 14] = _f32(r0)
    x[base + 15] = _f32(r1)
    r0 = _f32(x[base + 28] - x[base + 12])
    r1 = _f32(x[base + 29] - x[base + 13])
    x[base + 28] = _f32(x[base + 28] + x[base + 12])
    x[base + 29] = _f32(x[base + 29] + x[base + 13])
    x[base + 12] = _f32(r0 * c1 - r1 * s1)
    x[base + 13] = _f32(r0 * s1 + r1 * c1)
    r0 = _f32(x[base + 26] - x[base + 10])
    r1 = _f32(x[base + 27] - x[base + 11])
    x[base + 26] = _f32(x[base + 26] + x[base + 10])
    x[base + 27] = _f32(x[base + 27] + x[base + 11])
    x[base + 10] = _f32((r0 - r1) * s2)
    x[base + 11] = _f32((r0 + r1) * s2)
    r0 = _f32(x[base + 24] - x[base + 8])
    r1 = _f32(x[base + 25] - x[base + 9])
    x[base + 24] = _f32(x[base + 24] + x[base + 8])
    x[base + 25] = _f32(x[base + 25] + x[base + 9])
    x[base + 8] = _f32(r0 * s1 - r1 * c1)
    x[base + 9] = _f32(r1 * s1 + r0 * c1)
    r0 = _f32(x[base + 22] - x[base + 6])
    r1 = _f32(x[base + 7] - x[base + 23])
    x[base + 22] = _f32(x[base + 22] + x[base + 6])
    x[base + 23] = _f32(x[base + 23] + x[base + 7])
    x[base + 6] = _f32(r1)
    x[base + 7] = _f32(r0)
    r0 = _f32(x[base + 4] - x[base + 20])
    r1 = _f32(x[base + 5] - x[base + 21])
    x[base + 20] = _f32(x[base + 20] + x[base + 4])
    x[base + 21] = _f32(x[base + 21] + x[base + 5])
    x[base + 4] = _f32(r1 * c1 + r0 * s1)
    x[base + 5] = _f32(r1 * s1 - r0 * c1)
    r0 = _f32(x[base + 2] - x[base + 18])
    r1 = _f32(x[base + 3] - x[base + 19])
    x[base + 18] = _f32(x[base + 18] + x[base + 2])
    x[base + 19] = _f32(x[base + 19] + x[base + 3])
    x[base + 2] = _f32((r1 + r0) * s2)
    x[base + 3] = _f32((r1 - r0) * s2)
    r0 = _f32(x[base] - x[base + 16])
    r1 = _f32(x[base + 1] - x[base + 17])
    x[base + 16] = _f32(x[base + 16] + x[base])
    x[base + 17] = _f32(x[base + 17] + x[base + 1])
    x[base] = _f32(r1 * c3 + r0 * c1)
    x[base + 1] = _f32(r1 * c1 - r0 * c3)
    _b16(x, base)
    _b16(x, base + 16)


def _first(trig: Sequence[float], x: list[float], points: int) -> None:
    x1 = points - 8
    x2 = (points >> 1) - 8
    t = 0
    while x2 >= 0:
        for off in (6, 4, 2, 0):
            # This stage spills both differences before loading the trig pair.
            r0 = _f32(x[x1 + off] - x[x2 + off])
            r1 = _f32(x[x1 + off + 1] - x[x2 + off + 1])
            x[x1 + off] = _f32(x[x1 + off] + x[x2 + off])
            x[x1 + off + 1] = _f32(x[x1 + off + 1] + x[x2 + off + 1])
            x[x2 + off] = _f32(r1 * trig[t + 1] + r0 * trig[t])
            x[x2 + off + 1] = _f32(r1 * trig[t] - r0 * trig[t + 1])
            t += 4
        x1 -= 8
        x2 -= 8


def _generic(
    trig: Sequence[float], x: list[float], base: int, points: int, stride: int
) -> None:
    x1 = base + points - 8
    x2 = base + (points >> 1) - 8
    t = 0
    while x2 >= base:
        for off in (6, 4, 2, 0):
            r0 = _f32(x[x1 + off] - x[x2 + off])
            r1 = _f32(x[x1 + off + 1] - x[x2 + off + 1])
            x[x1 + off] = _f32(x[x1 + off] + x[x2 + off])
            x[x1 + off + 1] = _f32(x[x1 + off + 1] + x[x2 + off + 1])
            x[x2 + off] = _f32(r1 * trig[t + 1] + r0 * trig[t])
            x[x2 + off + 1] = _f32(r1 * trig[t] - r0 * trig[t + 1])
            t += stride
        x1 -= 8
        x2 -= 8


def _butterflies(look: MdctLook, x: list[float], points: int) -> None:
    stages = look.log2n - 5
    if (stages := stages - 1) > 0:
        _first(look.trig, x, points)
    i = 1
    while (stages := stages - 1) > 0:
        for j in range(1 << i):
            _generic(look.trig, x, (points >> i) * j, points >> i, 4 << i)
        i += 1
    for base in range(0, points, 32):
        _b32(x, base)


def _bitreverse(look: MdctLook, x: list[float]) -> None:
    n = look.n
    half = n >> 1
    w0 = 0
    w1 = half
    t = n
    bit = 0
    while w0 < w1:
        ia = half + look.bitrev[bit]
        ib = half + look.bitrev[bit + 1]
        # reference routine spills both butterfly inputs to float stack
        # subtraction/sum as host doubles is usually invisible, but it
        # changes the low-MDCT cancellation bins by whole dB after the
        # bit-domain log curve.
        r0 = _f32(x[ia + 1] - x[ib + 1])
        r1 = _f32(x[ia] + x[ib])
        r2 = _f32(r1 * look.trig[t] + r0 * look.trig[t + 1])
        r3 = _f32(r1 * look.trig[t + 1] - r0 * look.trig[t])
        w1 -= 4
        h0 = _f32(0.5 * (x[ia + 1] + x[ib + 1]))
        h1 = _f32(0.5 * (x[ia] - x[ib]))
        x[w0] = _f32(h0 + r2)
        x[w1 + 2] = _f32(h0 - r2)
        x[w0 + 1] = _f32(h1 + r3)
        x[w1 + 3] = _f32(r3 - h1)

        ia = half + look.bitrev[bit + 2]
        ib = half + look.bitrev[bit + 3]
        r0 = _f32(x[ia + 1] - x[ib + 1])
        r1 = _f32(x[ia] + x[ib])
        r2 = _f32(r1 * look.trig[t + 2] + r0 * look.trig[t + 3])
        r3 = _f32(r1 * look.trig[t + 3] - r0 * look.trig[t + 2])
        h0 = _f32(0.5 * (x[ia + 1] + x[ib + 1]))
        h1 = _f32(0.5 * (x[ia] - x[ib]))
        x[w0 + 2] = _f32(h0 + r2)
        x[w1] = _f32(h0 - r2)
        x[w0 + 3] = _f32(h1 + r3)
        x[w1 + 1] = _f32(r3 - h1)
        w0 += 4
        bit += 4
        t += 4


def mdct_forward(look: MdctLook, samples: Sequence[float]) -> list[float]:
    """Return the normalized ``n/2`` MDCT spectrum for one ``n``-sample block."""
    n = look.n
    if len(samples) < n:
        raise ValueError(f"need {n} samples, got {len(samples)}")
    n2, n4, n8 = n >> 1, n >> 2, n >> 3
    w = [0.0] * n
    w2 = n2
    x0 = n2 + n4
    x1 = x0 + 1
    t = n2
    i = 0
    while i < n8:
        x0 -= 4
        t -= 2
        # spectrum transform's var_8/var_C are float stack slots.  Keeping these sums in
        # host double lets a low bit leak into every later butterfly stage.
        r0 = _f32(samples[x0 + 2] + samples[x1])
        r1 = _f32(samples[x0] + samples[x1 + 2])
        w[w2 + i] = _f32(r1 * look.trig[t + 1] + r0 * look.trig[t])
        w[w2 + i + 1] = _f32(r1 * look.trig[t] - r0 * look.trig[t + 1])
        x1 += 4
        i += 2

    x1 = 1
    while i < n2 - n8:
        t -= 2
        x0 -= 4
        r0 = _f32(samples[x0 + 2] - samples[x1])
        r1 = _f32(samples[x0] - samples[x1 + 2])
        w[w2 + i] = _f32(r1 * look.trig[t + 1] + r0 * look.trig[t])
        w[w2 + i + 1] = _f32(r1 * look.trig[t] - r0 * look.trig[t + 1])
        x1 += 4
        i += 2

    x0 = n
    while i < n2:
        t -= 2
        x0 -= 4
        r0 = _f32(-samples[x0 + 2] - samples[x1])
        r1 = _f32(-samples[x0] - samples[x1 + 2])
        w[w2 + i] = _f32(r1 * look.trig[t + 1] + r0 * look.trig[t])
        w[w2 + i + 1] = _f32(r1 * look.trig[t] - r0 * look.trig[t + 1])
        x1 += 4
        i += 2

    butterfly_buf = w[n2:]
    _butterflies(look, butterfly_buf, n2)
    w[n2:] = butterfly_buf
    _bitreverse(look, w)
    out = [0.0] * n2
    t = n2
    x0 = n2
    for i in range(n4):
        x0 -= 1
        out[i] = _f32((w[2 * i] * look.trig[t] + w[2 * i + 1] * look.trig[t + 1]) * look.scale)
        out[x0] = _f32((w[2 * i] * look.trig[t + 1] - w[2 * i + 1] * look.trig[t]) * look.scale)
        t += 2
    return out
