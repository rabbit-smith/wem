"""Deterministic sample conversion into the signed-16 encoder domain.

These helpers are the single place where non-16-bit PCM samples are mapped
into the signed-16 domain consumed by the encoder.  Every function is
pure: identical input bits produce identical output bits on every
platform, and no hidden state is read or written.

Arithmetic rules (normative):

* integer paths use only integer addition, shift, and comparison;
* the float path uses only the IEEE finiteness check, multiplication by
  the constant ``32768.0``, addition of the constant ``0.5``, truncation
  toward zero, and explicit saturation comparisons;
* no transcendental functions, no locale, no platform state.

Conversion rules (per sample, all values two's-complement LE sources):

* 24-bit signed -> int16: round-to-nearest with ties away from zero,
  then saturate to ``[-32768, 32767]``.
  For ``s >= 0``: ``(s + 128) >> 8``; for ``s < 0``: ``-((-s + 128) >> 8)``.
  The upper band ``s >= 8388480`` (0x7FF800) rounds to 32768 and
  saturates to 32767; the minimum ``s == -8388608`` maps exactly to
  -32768.
* IEEE-754 float -> int16: reject non-finite values; scale by
  ``* 32768.0``; round-to-nearest with ties away from zero; then
  saturate to ``[-32768, 32767]``.  ``1.0`` saturates to 32767 while
  ``-1.0`` maps exactly to -32768 (the signed-16 asymmetry).
* int16 -> legacy float domain: ``value / 32768.0`` (the historical
  ``read_pcm16_wav`` normalization, unchanged).

Inputs arriving through these rules are in-domain for the encoder;
their WEM bytes are deterministic, but the project does not promise
bit-exact agreement with Wwise imports for converted inputs.
"""

from __future__ import annotations

__all__ = [
    "float_to_int16",
    "int16_to_domain_value",
    "sample24_to_int16",
    "unpack_sample24",
]

_INT16_MIN = -32768
_INT16_MAX = 32767
_SAMPLE24_MIN = -8388608
_SAMPLE24_MAX = 8388607
_SCALE = 32768.0


def _require_integer_sample(value: int, label: str) -> int:
    if not isinstance(value, int) or isinstance(value, bool):
        raise TypeError(f"{label} must be an integer")
    return value


def unpack_sample24(payload: bytes, offset: int) -> int:
    """Read one little-endian two's-complement 24-bit sample as signed int."""
    lo, mid, hi = payload[offset], payload[offset + 1], payload[offset + 2]
    value = lo | (mid << 8) | (hi << 16)
    if value & 0x800000:
        value -= 1 << 24
    return value


def sample24_to_int16(sample24: int) -> int:
    """Convert one signed 24-bit sample into the signed-16 domain.

    Round-to-nearest, ties away from zero, then saturate:

    * ``s >= 0``: ``(s + 128) >> 8``
    * ``s < 0``:  ``-((-s + 128) >> 8)``
    * clamp the result to ``[-32768, 32767]`` (live only for
      ``s >= 8388480``, which rounds to 32768).

    The signed 24-bit range ``[-8388608, 8388607]`` is required.
    """
    _require_integer_sample(sample24, "24-bit sample")
    if not _SAMPLE24_MIN <= sample24 <= _SAMPLE24_MAX:
        raise ValueError("24-bit sample is outside the signed 24-bit range")
    if sample24 >= 0:
        value = (sample24 + 128) >> 8
    else:
        value = -((-sample24 + 128) >> 8)
    if value > _INT16_MAX:
        value = _INT16_MAX
    if value < _INT16_MIN:
        value = _INT16_MIN
    return value


def float_to_int16(sample: int | float) -> int:
    """Convert one IEEE-754 float sample into the signed-16 domain.

    Steps (deterministic, in order):

    1. reject non-finite values (NaN and +/-Inf) with ``ValueError``;
    2. scale: ``sample * 32768.0``;
    3. round-to-nearest, ties away from zero:
       positive ``int(scaled + 0.5)``, negative ``-int(-scaled + 0.5)``;
    4. saturate: clamp the rounded value to ``[-32768, 32767]``.

    Boundaries: ``1.0 -> 32767`` (saturated), ``-1.0 -> -32768``
    (exact), a float just below 1.0 (``1 - 2**-24``) rounds to 32768
    and saturates to 32767.
    """
    if isinstance(sample, bool) or not isinstance(sample, (int, float)):
        raise TypeError("float sample must be an int or float")
    if sample != sample or sample == float("inf") or sample == float("-inf"):
        raise ValueError("float sample must be finite (NaN and Inf are rejected)")
    scaled = sample * _SCALE
    if scaled > 0.0:
        value = int(scaled + 0.5)
    elif scaled < 0.0:
        value = -int(-scaled + 0.5)
    else:
        value = 0
    if value > _INT16_MAX:
        value = _INT16_MAX
    if value < _INT16_MIN:
        value = _INT16_MIN
    return value


def int16_to_domain_value(sample16: int) -> float:
    """Map a signed-16 sample into the legacy encoder float domain.

    Uses the historical ``value / 32768.0`` normalization verbatim so
    converted inputs land on exactly the floats the signed-16 path
    already produces for the same sample.
    """
    _require_integer_sample(sample16, "signed-16 sample")
    if not _INT16_MIN <= sample16 <= _INT16_MAX:
        raise ValueError("signed-16 sample is outside the signed-16 range")
    return sample16 / _SCALE
