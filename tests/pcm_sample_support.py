"""Little-endian sample encoders shared by the PCM intake suites.

The signed-16, signed-24 and float32 little-endian byte forms are the three
carriers ``read_raw_pcm`` and ``read_pcm_wav`` convert into the encoder's
float domain. Three merged suites spelled the same three builders under six
names, so they live here once; a test that needs a different shape (a
shifted 24-bit form, a stream built from int values) builds it in place.
"""

from __future__ import annotations

import struct


def int16_le_bytes(*values: int) -> bytes:
    """Signed 16-bit little-endian samples."""
    return b"".join(value.to_bytes(2, "little", signed=True) for value in values)


def int24_le_bytes(*values: int) -> bytes:
    """Signed 24-bit little-endian samples, from the low three bytes."""
    return b"".join((value & 0xFFFFFF).to_bytes(3, "little") for value in values)


def float32_le_bytes(*values: float) -> bytes:
    """IEEE-754 binary32 little-endian samples."""
    return struct.pack(f"<{len(values)}f", *values)
