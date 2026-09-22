"""LONG analysis geometry, ported from the paired build's geometry builder.

Registered surfaces are read exclusively by the verification runner, never this builder.
"""

from __future__ import annotations

import struct

from . import inputs
from . import short_seed

TARGET = "long_base"

#: Descriptor family -> sample rate. Both entries are registered geometries; the
#: LONG knot banks live in the descriptor's own pointer block, so the rate is
#: the only thing that changes between them.
SAMPLE_RATE = {(6, 40000, 70000): 44100, (2, 45000, 50000): 48000}


def constant_bytes(name):
    return inputs.constant_bytes(name)


def build(d, key, geo):
    return build_mode(d, key, geo, 2)


def build_mode(d, key, geo, mode):
    if tuple(geo) != tuple(key):
        raise ValueError("descriptor geometry does not match requested key")
    if tuple(key) not in SAMPLE_RATE:
        raise ValueError("unsupported LONG geometry key")
    if mode not in (2, 3):
        raise ValueError("LONG mode must be 2 or 3")
    rate = SAMPLE_RATE[tuple(key)]
    # Mode 2 takes the descriptor's +48 bank pointer; mode 3 takes +4c instead.
    # The knot writer fills the record window at +132..332.
    raw = constant_bytes(f"mask_bank_mode_{mode}")
    words = struct.unpack(f"<{len(raw) // 4}I", raw)
    knots = short_seed.mask_knots(words, short_seed.default_quality_index())
    # Same loop as SHORT, with the explicit LONG a4=1024.
    rows = short_seed.mask_curves(1024, rate, knots)
    out = {
        f"analysis.curves[{i}]": [short_seed.f32_bits(v) for v in row] for i, row in enumerate(rows)
    }
    # Same bin-center position and weights, but the field curve's own knots.
    # The 18th source word is present for the zero-weight endpoint load.
    field_knots = struct.unpack("<18f", constant_bytes("analysis_field_knots_f32"))
    field = short_seed.mask_curves(1024, rate, [field_knots])[0]
    out["analysis.field_19_curve"] = [short_seed.f32_bits(v) for v in field]
    # The descriptor's +3c block supplies the mode triples, NOT +50.
    # Source +12*mode feeds a2+120/+124; mode 2 is 10/10.
    # The common seed is copied; +112/+116 retain 0.5f.
    lower, upper = struct.unpack_from("<2i", constant_bytes("interval_seed_axis"), mode * 12)
    out["analysis.interval_u32"] = short_seed.interval_table(
        1024, rate, a2_120=lower, a2_124=upper
    )
    return out
