"""Named entry points for every runtime transcendental call in the encoder.

This module is the deterministic-math seam for the Wwise 2013.2 bit-exact
profile.  Production calls forward to the host libm unchanged.

:func:`start_recording` switches on an additional per-site record of
deduplicated ``(float64 input bits, float64 output bits)`` pairs, written as
compact 16-byte binary files so the live input domain of each call site becomes
an auditable contract asset for cross-implementation verification.  Recording
is opt-in through that call — this module reads no environment variable, so a
recorded domain is always something a caller asked for explicitly.  The
recorded *values* are the same either way: recording observes calls, it never
changes them.

Site names are stable identities; renaming one is a contract change.
"""
from __future__ import annotations

import math
import struct
from pathlib import Path
from typing import Callable

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

# Set only by `start_recording`; `None` means the wrappers below are pass-through.
_recording_dir: Path | None = None


class _SiteRecorder:
    __slots__ = ("seen", "calls")

    def __init__(self) -> None:
        self.seen: dict[int, int] = {}
        self.calls = 0


_recorders: dict[str, _SiteRecorder] = {name: _SiteRecorder() for name in _SITE_NAMES}


def start_recording(directory: str | Path) -> None:
    """Record every site call's bit pairs into ``directory``.

    Must be called before the encoding whose domain is being recorded; the
    flush is :func:`write_recording`.
    """
    global _recording_dir
    _recording_dir = Path(directory)


def recording_sites() -> tuple[str, ...]:
    """The site names being recorded right now (empty when recording is off)."""
    return _SITE_NAMES if _recording_dir is not None else ()


def math_bits(value: float) -> int:
    """Return the IEEE-754 double bit pattern of ``value`` as an unsigned int."""
    return int.from_bytes(_PACK("<d", value), "little")


def _make(site: str, fn: Callable[[float], float]) -> Callable[[float], float]:
    recorder = _recorders[site]

    def tracked(value: float) -> float:
        if _recording_dir is None:
            return fn(value)
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
    """Flush recorded site pairs to the directory given to :func:`start_recording`.

    Returns the per-site unique-input counts; an empty mapping when recording
    was never switched on.
    """
    directory = _recording_dir
    if directory is None:
        return {}
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
