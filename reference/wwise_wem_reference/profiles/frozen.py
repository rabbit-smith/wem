"""Frozen transcendental tables, read from the compiled profile artifact.

All floating-point content is restored from stored IEEE bit patterns; no float
string parsing anywhere. The tables are Rust constants in the kernel and reach
this module through the compiled artifact, so nothing here validates a
document shape any more: the carrier was checked when it was generated.
"""
from __future__ import annotations

import struct
from functools import lru_cache

from ..analysis.config import FrozenMathTables
from .artifact import CompiledProfile


def _bits_to_f64(bits: int) -> float:
    return struct.unpack("<d", struct.pack("<Q", int(bits) & 0xFFFFFFFFFFFFFFFF))[0]


@lru_cache(maxsize=None)
def load_frozen_tables(profile: CompiledProfile) -> FrozenMathTables:
    """Rebuild the frozen-math tables the profile carries."""
    if not isinstance(profile, CompiledProfile):
        raise TypeError("frozen tables require a CompiledProfile")
    coordinate_ln = {
        int(in_bits): _bits_to_f64(out_bits)
        for in_bits, out_bits in zip(
            profile.table("frozen.coordinate_ln.in_bits"),
            profile.table("frozen.coordinate_ln.out_bits"),
        )
    }
    fft_twiddles = {
        int(length): (_bits_to_f64(cos_bits), _bits_to_f64(sin_bits))
        for length, cos_bits, sin_bits in zip(
            profile.table("frozen.fft_twiddles.length"),
            profile.table("frozen.fft_twiddles.cos_bits"),
            profile.table("frozen.fft_twiddles.sin_bits"),
        )
    }
    sizes = profile.table("frozen.window_halves.size")
    words = profile.table("frozen.window_halves.words")
    offsets = profile.table("frozen.window_halves.offsets")
    window_halves = {
        int(size): tuple(float(value) for value in words[offsets[index] : offsets[index + 1]])
        for index, size in enumerate(sizes)
    }
    return FrozenMathTables(
        coordinate_ln=coordinate_ln,
        fft_twiddles=fft_twiddles,
        window_halves=window_halves,
    )


__all__ = ["load_frozen_tables"]
