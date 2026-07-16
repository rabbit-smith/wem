"""Float32 spectral transforms used by Wwise psychoacoustic analysis."""
from __future__ import annotations

import math
import struct
from typing import Sequence


WWISE_LOG_SCALE = 0.0000007177114298428933
WWISE_LOG_BIAS = 764.6162109375
WWISE_LOG_ADD = 0.345


def _f32(value: float) -> float:
    return struct.unpack("<f", struct.pack("<f", float(value)))[0]


def _float_abs_bits(value: float) -> int:
    """Return the positive float32 bit pattern used by spectral scaling."""
    return struct.unpack("<I", struct.pack("<f", float(value)))[0] & 0x7FFFFFFF


def wwise_float_log(value: float) -> float:
    """Apply the encoder format's float-bit logarithm approximation."""
    return _f32(_float_abs_bits(value) * WWISE_LOG_SCALE - WWISE_LOG_BIAS)


def wwise_fft_packed(samples: Sequence[float]) -> list[float]:
    """Return the packed real-FFT layout used by frame analysis.

    For an even ``n`` the layout is ``[DC, Re(1), Im(1), ..., Re(n/2)]``.
    Every input and butterfly result is rounded to float32.
    """
    n = len(samples)
    if n < 2 or n & (n - 1) or n & 1:
        raise ValueError("FFT size must be an even power of two >= 2")

    real = [_f32(value) for value in samples]
    imag = [0.0] * n

    j = 0
    for i in range(1, n):
        bit = n >> 1
        while j & bit:
            j ^= bit
            bit >>= 1
        j ^= bit
        if i < j:
            real[i], real[j] = real[j], real[i]

    length = 2
    while length <= n:
        half = length >> 1
        angle = -2.0 * math.pi / length
        step_re = _f32(math.cos(angle))
        step_im = _f32(math.sin(angle))
        for start in range(0, n, length):
            wr = 1.0
            wi = 0.0
            for k in range(half):
                even = start + k
                odd = even + half
                tr = _f32(wr * real[odd] - wi * imag[odd])
                ti = _f32(wr * imag[odd] + wi * real[odd])
                er = real[even]
                ei = imag[even]
                real[even] = _f32(er + tr)
                imag[even] = _f32(ei + ti)
                real[odd] = _f32(er - tr)
                imag[odd] = _f32(ei - ti)
                wr, wi = (
                    _f32(wr * step_re - wi * step_im),
                    _f32(wr * step_im + wi * step_re),
                )
        length <<= 1

    packed = [_f32(real[0])]
    for k in range(1, n // 2):
        packed.extend((_f32(real[k]), _f32(imag[k])))
    packed.append(_f32(real[n // 2]))
    return packed


def wwise_log_curve(samples: Sequence[float]) -> list[float]:
    """Convert a packed analysis spectrum to the encoder log domain."""
    n = len(samples)
    packed = wwise_fft_packed(samples)
    offset = _f32(wwise_float_log(4.0 / n) + WWISE_LOG_ADD)
    out = [_f32(wwise_float_log(packed[0]) + offset + WWISE_LOG_ADD)]
    for index in range(1, n - 1, 2):
        power = _f32(
            packed[index] * packed[index]
            + packed[index + 1] * packed[index + 1]
        )
        out.append(
            _f32(0.5 * wwise_float_log(power) + offset + WWISE_LOG_ADD)
        )
    return out


def wwise_mdct_log_curve(samples: Sequence[float]) -> list[float]:
    """Convert normalized MDCT coefficients to the encoder log domain."""
    return [
        _f32(wwise_float_log(abs(float(value))) + WWISE_LOG_ADD)
        for value in samples
    ]
