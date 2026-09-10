"""Build typed runtime aggregates from a profile's logical manifest."""
from __future__ import annotations

import dataclasses
import struct
from dataclasses import dataclass
from types import MappingProxyType
from typing import Any, Mapping

from ..analysis.config import (
    AnalysisProfileResources,
    WwisePsySeedSurface,
    make_long_floor_envelope_look,
    make_wwise_psy_look,
)
from ..vorbis.codebook import Codebook
from ..vorbis.setup import parse_setup
from .book_ids import load_book_table
from wwise_wem.profiles.bundle import ProfileBundle
from .codebooks import load_setup_codebooks
from .frozen import load_frozen_tables
from .quality import (
    QUALITY_CURVES_RESOURCE,
    QUALITY_SEMANTIC_SHORT_PREFIX,
    QualityCurves,
    load_quality_curves,
    normalize_quality_factor,
)
from .transform import load_mdct_looks
from .transient import load_transient_resource
from .psychoacoustics.config import load_short_seed_surface
from .psychoacoustics.long_tables import load_long_psy_tables
from .psychoacoustics.long_variants import load_long_variant
from .psychoacoustics.short_tables import load_short_psy_profiles


@dataclass(frozen=True)
class EncoderProfileResources:
    """Complete immutable codec inputs assembled from one profile manifest."""

    analysis: AnalysisProfileResources
    setup_packet: bytes
    setup: Mapping[str, Any]
    codebooks: tuple[Codebook, ...]

    def __post_init__(self) -> None:
        object.__setattr__(self, "setup_packet", bytes(self.setup_packet))
        object.__setattr__(self, "setup", MappingProxyType(dict(self.setup)))
        object.__setattr__(self, "codebooks", tuple(self.codebooks))


# Scalar psychoacoustic fields that Wwise varies with the quality factor along
# a profile's quality-curves table. These are the namespaced parameter names
# (``short.<field>``) accepted by the quality override mechanism. The
# authoritative mapping (which fields, which curves, which control points) is
# calibrated by the quality-formula spec; until then this set is the supported
# surface. Long psy uses packed u32 coefficients and the mask-curve blend
# factors are not yet pinned to these names, so they are deliberately excluded
# here (better to omit than to guess) and are listed as pending in the report.
_SHORT_QUALITY_FIELDS = frozenset(
    {
        "ath_offset",
        "ath_floor",
        "seed_ceiling",
        "max_curve_db",
        "regular_curve_bias",
        "regular_curve_cap",
        "curve_offset",
        "curve_slope",
        "curve_offset_2",
    }
)


def _resolve_quality_curves(
    bundle: ProfileBundle, quality: float | None
) -> tuple[QualityCurves | None, Mapping[str, float] | None, bool]:
    """Load and evaluate the profile's quality curves for one quality value.

    ``quality`` of ``None`` keeps the historical behavior exactly: no resource
    lookup, no overrides. A quality value requires the profile to carry the
    optional resource; asking for it from a profile without curves is a
    configuration error, not a silent fallback. The quality is normalized
    onto the breakpoint axis before evaluation (spec profile-select entry).
    """
    if quality is None:
        return None, None, False
    curves_ref = bundle.runtime_manifest.resources.get(QUALITY_CURVES_RESOURCE)
    if curves_ref is None:
        raise ValueError(
            f"profile {bundle.name!r} has no quality-curves resource; "
            "quality interpolation is unavailable for this profile"
        )
    curves = load_quality_curves(curves_ref)
    if curves is None:
        raise ValueError(f"profile {bundle.name!r} quality-curves failed to load")
    normalized = normalize_quality_factor(quality)
    values, extrapolated = curves.evaluate_result(normalized)
    return curves, values, extrapolated


def _apply_short_quality_overrides(
    surface: WwisePsySeedSurface,
    values: Mapping[str, float],
    semantics: Mapping[str, str],
) -> WwisePsySeedSurface:
    """Return the short surface with recognized quality overrides applied.

    v2 routing: each evaluated curve is looked up in the resource's
    ``semantics`` map; a ``short.<field>`` semantic writes the interpolated
    value onto the same-named short surface field, while the other semantic
    forms (``no-op``, ``transient.record-index-axis``) name mechanisms that
    consume their curve elsewhere (the record family owns its index table) or
    do not consume it at runtime. Every override write goes through the f32
    boundary: the short psychoacoustic scalars are stored as float32, so the
    interpolated f64 value is rounded exactly as the reference casts it.
    Unrecognized parameter names are rejected so a malformed curves file can
    never silently take effect.
    """
    overrides: dict[str, float] = {}
    for name, value in values.items():
        semantic = semantics.get(name)
        if semantic is None or not semantic.startswith(QUALITY_SEMANTIC_SHORT_PREFIX):
            continue
        field = semantic[len(QUALITY_SEMANTIC_SHORT_PREFIX) :]
        if field not in _SHORT_QUALITY_FIELDS:
            raise ValueError(
                f"quality curve {name!r} semantic {semantic!r} names an "
                "unsupported parameter; supported short parameters: "
                + ", ".join(sorted(_SHORT_QUALITY_FIELDS))
            )
        overrides[field] = _f32_override(value)
    if not overrides:
        return surface
    # Every key in _SHORT_QUALITY_FIELDS is a float field of
    # WwisePsySeedSurface, so the float values are type-correct; mypy cannot
    # verify this through a dynamically-keyed **kwargs expansion on
    # dataclasses.replace.
    return dataclasses.replace(surface, **overrides)  # type: ignore[arg-type]


def _f32_override(value: float) -> float:
    """Round an interpolated f64 override through the f32 storage boundary."""
    return struct.unpack("<f", struct.pack("<f", float(value)))[0]


def assemble_analysis_resources(
    bundle: ProfileBundle,
    *,
    quality: float | None = None,
) -> AnalysisProfileResources:
    """Resolve MDCT/transient inputs through stable manifest logical names."""
    if not isinstance(bundle, ProfileBundle):
        raise TypeError("analysis resource assembly requires ProfileBundle")
    manifest = bundle.runtime_manifest
    short_surface = load_short_seed_surface(
        manifest.resource("psychoacoustics.short-seed")
    )
    long_base = load_long_psy_tables(
        manifest.resource("psychoacoustics.long-base")
    )
    long_modes_ref = manifest.resource("psychoacoustics.long-modes")
    long_variants = {
        mode: load_long_variant(mode, long_modes_ref, long_base)
        for mode in (2, 3)
    }
    frozen_ref = (
        manifest.resource("analysis.frozen-tables")
        if "analysis.frozen-tables" in manifest.resources
        else None
    )
    frozen = load_frozen_tables(frozen_ref) if frozen_ref is not None else None

    quality_normalized = None if quality is None else float(quality)
    quality_curves, quality_values, quality_extrapolated = _resolve_quality_curves(
        bundle, quality_normalized
    )
    if quality_values is not None:
        short_surface = _apply_short_quality_overrides(
            short_surface,
            quality_values,
            quality_curves.semantics if quality_curves is not None else {},
        )

    return AnalysisProfileResources(
        mdct_looks=load_mdct_looks(manifest.resource("transform.mdct")),
        transient=load_transient_resource(
            manifest.resource("analysis.transient"), quality_normalized
        ),
        short_profiles=load_short_psy_profiles(
            manifest.resource("psychoacoustics.short-profiles")
        ),
        short_surface=short_surface,
        short_look=make_wwise_psy_look(
            short_surface,
            frozen_ln=frozen.coordinate_ln if frozen is not None else None,
        ),
        long_base=long_base,
        long_variants=long_variants,
        long_floor_looks={
            mode: make_long_floor_envelope_look(table)
            for mode, table in long_variants.items()
        },
        frozen=frozen,
        quality_value=quality_normalized,
        quality_extrapolated=quality_extrapolated,
    )


def assemble_encoder_profile_resources(
    bundle: ProfileBundle,
    *,
    setup_packet: bytes | None = None,
    quality: float | None = None,
) -> EncoderProfileResources:
    """Resolve every analysis and Vorbis input from one profile identity."""
    if not isinstance(bundle, ProfileBundle):
        raise TypeError("encoder resource assembly requires ProfileBundle")
    packet = bundle.setup_packet() if setup_packet is None else bytes(setup_packet)
    setup = parse_setup(packet, channels=bundle.key.channels)
    manifest = bundle.runtime_manifest
    t97 = load_book_table(
        "t97", manifest.resource("vorbis.codebooks.t97")
    )
    # A profile carries only the residue tables its setup references: t219
    # (6ch) and/or t282 (2ch/48k). Load whichever are installed rather than
    # assuming the historical t97+t219 pair (mirrors the Rust assembly).
    tables = {"t97": t97}
    for name in ("vorbis.codebooks.t219", "vorbis.codebooks.t282"):
        try:
            ref = manifest.resource(name)
        except ValueError:
            continue
        tables[name.rsplit(".", 1)[1]] = load_book_table(name.rsplit(".", 1)[1], ref)
    return EncoderProfileResources(
        analysis=assemble_analysis_resources(bundle, quality=quality),
        setup_packet=packet,
        setup=setup,
        codebooks=load_setup_codebooks(setup["book_ids"], tables),
    )


__all__ = [
    "AnalysisProfileResources",
    "EncoderProfileResources",
    "assemble_analysis_resources",
    "assemble_encoder_profile_resources",
]
