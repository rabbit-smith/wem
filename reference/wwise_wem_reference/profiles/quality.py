"""Load and evaluate the optional per-profile quality-interpolation curves.

Wwise quality is not a distinct profile: the same geometry and setup share one
configuration, and the psychoacoustic parameters vary as linear interpolants
along a fixed breakpoint table. This module owns that optional resource
(``analysis/quality-curves.json``, schema ``wem.quality-curves.v2``), its
completeness validation, the immutable value object, and the single
interpolation kernel.

Schema v2 adds the per-curve ``semantics`` map: each descriptor curve
(``descNN.<field>``) is mapped onto the mechanism it drives. The three
recognized semantic forms are ``short.<field>`` (a short psychoacoustic
surface override), ``no-op`` (recorded control points without a runtime
consumer), and ``transient.record-index-axis`` (the curve belongs to the
temporal transient record-index mechanism; its recorded control points are
kept for provenance and are not consumed by the override path). The override
resolution in the assembly layer keys off these semantics, so the curve
names stay exactly as recorded in the paired build.

The kernel is aligned to the authoritative quality formula recorded for the
paired build and is intentionally isolated in ``_linear_frac`` so that any
later re-calibration can replace the math and its tests in one place without
touching the loaders or the assembly wiring. The normalization entry
(``normalize_quality_factor``) and the two-step fractional breakpoint
arithmetic (``frac = i + ratio`` written, ``f = frac - i`` read) are pinned to
the reference behavior; both implementations agree bit for bit on the shared
test vectors. Out-of-domain quality values are clamped to the first control
point or carried by the ``(N-1, N)`` segment and reported as extrapolated
(honesty: the caller must be able to tell that a quality fell outside the
recorded control points).
"""
from __future__ import annotations

import math
from dataclasses import dataclass
from types import MappingProxyType
from typing import Any, Mapping

from .artifact import CompiledProfile, named_blocks

QUALITY_CURVES_SCHEMA = "wem.quality-curves.v2"
QUALITY_CURVES_INTERPOLATION = "linear-frac"
# Per-curve semantic forms (v2). The first two are the mechanism-level
# constants; the ``short.<field>`` form names a short psychoacoustic surface
# override target (validated against the supported field set in the assembly
# layer, where the field set is owned).
QUALITY_SEMANTIC_NO_OP = "no-op"
QUALITY_SEMANTIC_TRANSIENT_RECORD_INDEX_AXIS = "transient.record-index-axis"
QUALITY_SEMANTIC_SHORT_PREFIX = "short."

# Spec normalization constants (profile-select entry):
#   qnorm = quality / 10.0 + 1e-7, clamped to the float32 constant promoted
#   to double whenever it reaches 1.0.
QUALITY_NORMALIZE_ADDEND = 1e-7
QUALITY_NORMALIZE_CLAMP = 0.9998999834060669


def _finite_float(value: Any, label: str) -> float:
    """Coerce and validate a JSON number as a finite float."""
    if isinstance(value, bool) or not isinstance(value, (int, float)):
        raise ValueError(f"{label} must be a number")
    result = float(value)
    if not math.isfinite(result):
        raise ValueError(f"{label} must be finite (NaN/inf rejected)")
    return result


def _float_tuple(value: Any, expected_len: int, label: str) -> tuple[float, ...]:
    # A sequence: the carrier hands a curve over as a tuple, a recorded
    # document handed it over as a list, and the value object is the same.
    if not isinstance(value, (list, tuple)):
        raise ValueError(f"{label} must be an array")
    if len(value) != expected_len:
        raise ValueError(
            f"{label} must contain {expected_len} values, found {len(value)}"
        )
    return tuple(_finite_float(item, f"{label}[{i}]") for i, item in enumerate(value))


@dataclass(frozen=True)
class QualityCurves:
    """Immutable quality-interpolation tables for one exact profile.

    ``breakpoints`` is a strictly increasing control-point domain (the Wwise
    quality axis, already normalized). ``curves`` maps a stable parameter name
    to the value taken at each breakpoint. Both have equal length and are
    validated at load time. ``semantics`` maps every curve name onto the
    semantic of the mechanism it drives (v2); it must cover the curve names
    exactly, so a malformed curves file can never take partial effect.
    """

    schema: str
    breakpoints: tuple[float, ...]
    curves: Mapping[str, tuple[float, ...]]
    semantics: Mapping[str, str]

    def __post_init__(self) -> None:
        if self.schema != QUALITY_CURVES_SCHEMA:
            raise ValueError(f"unsupported quality-curves schema {self.schema!r}")
        breakpoints = tuple(float(b) for b in self.breakpoints)
        if len(breakpoints) < 2:
            raise ValueError("quality-curves needs at least two breakpoints")
        if any(breakpoints[i] >= breakpoints[i + 1] for i in range(len(breakpoints) - 1)):
            raise ValueError("quality-curves breakpoints must be strictly increasing")
        object.__setattr__(self, "breakpoints", breakpoints)
        curves = {
            str(name): _float_tuple(
                values, len(breakpoints), f"quality curve {name!r}"
            )
            for name, values in self.curves.items()
        }
        if not curves:
            raise ValueError("quality-curves must define at least one curve")
        object.__setattr__(self, "curves", MappingProxyType(curves))
        if not isinstance(self.semantics, Mapping) or not all(
            isinstance(name, str) and isinstance(semantic, str) and semantic
            for name, semantic in self.semantics.items()
        ):
            raise ValueError("quality-curves semantics must be a name/semantic map")
        known = set(self.semantics)
        if set(curves) != known:
            raise ValueError(
                "quality-curves semantics must cover every curve name exactly "
                f"(missing or extra: {sorted(set(curves) ^ known)})"
            )
        object.__setattr__(
            self,
            "semantics",
            MappingProxyType({str(k): str(v) for k, v in self.semantics.items()}),
        )

    def _evaluate_all(
        self, quality: float
    ) -> tuple[dict[str, float], bool]:
        """Return ``(values_by_param, extrapolated)`` for one quality.

        Delegates every sample to :func:`_linear_frac` so the whole mechanism
        shares a single, swappable interpolation implementation.
        """
        if not math.isfinite(quality):
            raise ValueError("quality must be a finite number")
        values: dict[str, float] = {}
        extrapolated = False
        for name, samples in self.curves.items():
            value, outside = _linear_frac(self.breakpoints, samples, quality)
            values[name] = value
            extrapolated = extrapolated or outside
        return values, extrapolated

    def evaluate(self, quality: float) -> dict[str, float]:
        """Interpolate every curve at ``quality`` and return a values dict."""
        values, _ = self._evaluate_all(quality)
        return values

    def evaluate_result(
        self, quality: float
    ) -> tuple[dict[str, float], bool]:
        """Like :meth:`evaluate`, also reporting whether it was clamped."""
        return self._evaluate_all(quality)


def _linear_frac(
    breakpoints: tuple[float, ...], samples: tuple[float, ...], quality: float
) -> tuple[float, bool]:
    """Linear piecewise interpolation over a strictly increasing table.

    Returns ``(value, extrapolated)``. Spec-aligned kernel: the fractional
    breakpoint index is written as
    ``frac = i + (q - bp[i]) / (bp[i+1] - bp[i])`` and read back as
    ``f = frac - i`` (the two-step arithmetic is not the identity for
    large ``i``); at or past the last breakpoint the index is clamped to
    ``N - 0.001`` so the ``(N-1, N)`` segment carries the value. Quality
    strictly below the first breakpoint is clamped to the first control
    point. This is the single kernel behind the whole mechanism; the Rust
    side (``wem_profiles::quality::linear_frac``) mirrors it bit for bit.
    """
    n = len(breakpoints) - 1
    low, high = breakpoints[0], breakpoints[n]
    if quality <= low:
        return samples[0], quality < low
    if quality >= high:
        frac = n - 0.001
        i = n - 1
        f = frac - i
        value = (1.0 - f) * samples[i] + f * samples[i + 1]
        return value, quality > high
    i = 0
    while i < n and quality >= breakpoints[i + 1]:
        i += 1
    frac = i + (quality - breakpoints[i]) / (breakpoints[i + 1] - breakpoints[i])
    f = frac - i
    value = (1.0 - f) * samples[i] + f * samples[i + 1]
    return value, False


def normalize_quality_factor(quality: float) -> float:
    """Normalize a 0-10 quality factor onto the breakpoint axis.

    Spec profile-select entry: ``quality / 10.0 + 1e-7``, clamped to
    ``QUALITY_NORMALIZE_CLAMP`` whenever it reaches 1.0. The Rust side
    (``wem_profiles::quality::normalize_quality_factor``) mirrors this bit
    for bit on the shared test vectors.
    """
    normalized = quality / 10.0 + QUALITY_NORMALIZE_ADDEND
    return QUALITY_NORMALIZE_CLAMP if normalized >= 1.0 else normalized


def load_quality_curves(profile: CompiledProfile) -> QualityCurves | None:
    """Rebuild the optional quality-curves table from the compiled artifact.

    A profile with no curves records an empty breakpoint block, and that is
    ``None`` here: the historical behavior (no interpolation at all) is kept
    exactly, and a quality request against such a profile is a configuration
    error rather than a silent fallback.
    """
    if not isinstance(profile, CompiledProfile):
        raise TypeError("quality curves require a CompiledProfile")
    breakpoints = tuple(float(value) for value in profile.table("quality_curves.breakpoints"))
    if not breakpoints:
        return None
    curves: dict[str, tuple[float, ...]] = {
        name: tuple(float(value) for value in profile.table(f"quality_curves.curve.{name}"))
        for name in named_blocks(profile, "quality_curves.curve.", "")
    }
    semantics = {
        name: str(profile.table(f"quality_curves.semantic.{name}"))
        for name in named_blocks(profile, "quality_curves.semantic.", "")
    }
    return QualityCurves(
        schema=QUALITY_CURVES_SCHEMA,
        breakpoints=breakpoints,
        curves=curves,
        semantics=semantics,
    )


__all__ = [
    "QUALITY_CURVES_INTERPOLATION",
    "QUALITY_CURVES_SCHEMA",
    "QUALITY_NORMALIZE_ADDEND",
    "QUALITY_NORMALIZE_CLAMP",
    "QUALITY_SEMANTIC_NO_OP",
    "QUALITY_SEMANTIC_SHORT_PREFIX",
    "QUALITY_SEMANTIC_TRANSIENT_RECORD_INDEX_AXIS",
    "QualityCurves",
    "load_quality_curves",
    "normalize_quality_factor",
]
