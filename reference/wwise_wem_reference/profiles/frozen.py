"""Load the checksum-addressed frozen transcendental tables of one exact profile."""
from __future__ import annotations

import struct

from ..analysis.config import FrozenMathTables
from ..analysis.dsp.transform import _u32_f32
from wwise_wem.profiles.resources import ResourceRef


def _bits_to_f64(hex_bits: str) -> float:
    return struct.unpack("<d", bytes.fromhex(hex_bits))[0]


def _hex_to_u32(hex_bits: str) -> int:
    return int.from_bytes(bytes.fromhex(hex_bits), "little")


def _hex_to_u64(hex_bits: str) -> int:
    return int.from_bytes(bytes.fromhex(hex_bits), "little")


def load_frozen_tables(ref: ResourceRef) -> FrozenMathTables:
    """Validate the frozen-math payload shape and return typed bit-mapped tables."""
    if not isinstance(ref, ResourceRef):
        raise TypeError("frozen tables resource must be ResourceRef")
    payload = ref.read_json()
    if not isinstance(payload, dict) or payload.get("schema") != "wem.frozen-math.v1":
        raise ValueError("unexpected frozen-math schema")
    coordinate_ln = {
        _hex_to_u64(in_bits): _bits_to_f64(out_bits)
        for in_bits, out_bits in payload.get("coordinate_ln", [])
    }
    fft_twiddles = {
        int(length): (
            _bits_to_f64(cos_bits),
            _bits_to_f64(sin_bits),
        )
        for length, cos_bits, sin_bits in payload.get("fft_twiddles", [])
    }
    window_halves: dict[int, tuple[float, ...]] = {}
    for key, values in payload.get("window_halves", {}).items():
        size = int(key)
        if size < 2 or size & 1 or len(values) != size // 2:
            raise ValueError("frozen window halves disagree with their block size")
        window_halves[size] = tuple(_u32_f32(_hex_to_u32(word)) for word in values)
    if not coordinate_ln or not fft_twiddles or not window_halves:
        raise ValueError("frozen-math payload is missing a section")
    return FrozenMathTables(
        coordinate_ln=coordinate_ln,
        fft_twiddles=fft_twiddles,
        window_halves=window_halves,
    )


__all__ = ["load_frozen_tables"]
