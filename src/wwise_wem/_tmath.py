"""Named entry points for every runtime transcendental call in the encoder.

This module is the deterministic-math seam for the Wwise 2013.2 bit-exact
profile.  Production calls forward to the host libm unchanged; setting
``WEM_TMATH_RECORD=<directory>`` additionally records deduplicated
``(float64 input bits, float64 output bits)`` pairs per site as compact
16-byte binary files so the live input domain of each call site becomes an
auditable contract asset for cross-implementation verification.

Site names are stable identities; renaming one is a contract change.
"""
from __future__ import annotations

import math
import os
import struct
from pathlib import Path
from typing import Callable

_RECORD_DIR = os.environ.get("WEM_TMATH_RECORD", "")
_PACK = struct.pack

_SITE_NAMES = (
    "floor_fit.ln",
    "residue.log10",
    "spectrum.cos",
    "spectrum.sin",
    "config.ln",
    "transform.cos",
    "transform.sin",
    "transform.window_short.sin",
    "transform.window_long.sin",
)


class _SiteRecorder:
    __slots__ = ("seen", "calls")

    def __init__(self) -> None:
        self.seen: dict[int, int] = {}
        self.calls = 0


def call_counts() -> dict[str, int]:
    """Per-site wrapper call counts since import (untracked sites are unwrapped)."""
    return {name: recorder.calls for name, recorder in _recorders.items()}


_recorders: dict[str, _SiteRecorder] = {name: _SiteRecorder() for name in _SITE_NAMES}

RECORDED_SITES: tuple[str, ...] = _SITE_NAMES if _RECORD_DIR else ()


def math_bits(value: float) -> int:
    """Return the IEEE-754 double bit pattern of ``value`` as an unsigned int."""
    return int.from_bytes(_PACK("<d", value), "little")


def _make(site: str, fn: Callable[[float], float]) -> Callable[[float], float]:
    recorder = _recorders[site]
    if not _RECORD_DIR:
        return fn

    def tracked(value: float) -> float:
        recorder.calls += 1
        result = fn(value)
        recorder.seen[math_bits(value)] = math_bits(result)
        return result

    return tracked


# Site-bound wrappers. Defaults are exact host-libm calls; recording only samples.
ln_f64 = _make("floor_fit.ln", math.log)
log10_f64 = _make("residue.log10", math.log10)
cos_f64 = _make("spectrum.cos", math.cos)
sin_f64 = _make("spectrum.sin", math.sin)
config_ln_f64 = _make("config.ln", math.log)
transform_cos_f64 = _make("transform.cos", math.cos)
transform_sin_f64 = _make("transform.sin", math.sin)
window_short_sin_f64 = _make("transform.window_short.sin", math.sin)
window_long_sin_f64 = _make("transform.window_long.sin", math.sin)


def write_recording() -> dict[str, int]:
    """Flush recorded site pairs to ``$WEM_TMATH_RECORD``; returns per-site counts."""
    if not _RECORD_DIR:
        return {}
    directory = Path(_RECORD_DIR)
    directory.mkdir(parents=True, exist_ok=True)
    counts: dict[str, int] = {}
    for name, recorder in _recorders.items():
        if not recorder.seen:
            counts[name] = 0
            continue
        payload = b"".join(
            _PACK("<QQ", input_bits, output_bits)
            for input_bits, output_bits in sorted(recorder.seen.items())
        )
        (directory / f"{name}.in-out.u64le.bin").write_bytes(payload)
        counts[name] = len(recorder.seen)
    return counts
