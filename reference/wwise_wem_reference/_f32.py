"""The one float32 storage boundary of the reference tree.

Every ``_f32(...)`` call in the oracle marks a float32 store: the value is
rounded to single precision (IEEE-754 round-to-nearest-even, the x86 default
the paired build runs with) and returned as a Python float. The standards make
that call the greppable marker of a boundary — each one is a statement the Rust
kernel rounds at too — so the call sites stay where they are and only the store
itself lives here, once.

The other f32 flavour in this tree is *not* folded in here:
:func:`wwise_wem_reference.geometry_materializer.f32_primitives.f32` is a
fail-loud port of the paired build's x87 helper and raises ``ValueError`` where
this raises ``OverflowError``; the failure class is part of that surface.
"""

from __future__ import annotations

import struct


def _f32(value: float) -> float:
    return struct.unpack("<f", struct.pack("<f", float(value)))[0]
