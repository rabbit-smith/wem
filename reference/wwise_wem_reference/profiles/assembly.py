"""Build typed runtime aggregates from a profile's logical manifest."""
from __future__ import annotations

from dataclasses import dataclass
from types import MappingProxyType
from typing import Any, Mapping

from ..analysis.config import (
    AnalysisProfileResources,
    make_long_floor_envelope_look,
    make_wwise_psy_look,
)
from ..vorbis.codebook import Codebook
from ..vorbis.setup import parse_setup
from .book_ids import load_book_table
from wwise_wem.profiles.bundle import ProfileBundle
from .codebooks import load_setup_codebooks
from .frozen import load_frozen_tables
from .transform import load_mdct_looks
from .transient import load_transient_tables
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


def assemble_analysis_resources(bundle: ProfileBundle) -> AnalysisProfileResources:
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
    return AnalysisProfileResources(
        mdct_looks=load_mdct_looks(manifest.resource("transform.mdct")),
        transient=load_transient_tables(manifest.resource("analysis.transient")),
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
    )


def assemble_encoder_profile_resources(
    bundle: ProfileBundle,
    *,
    setup_packet: bytes | None = None,
) -> EncoderProfileResources:
    """Resolve every analysis and Vorbis input from one profile identity."""
    if not isinstance(bundle, ProfileBundle):
        raise TypeError("encoder resource assembly requires ProfileBundle")
    packet = bundle.setup_packet() if setup_packet is None else bytes(setup_packet)
    setup = parse_setup(packet, channels=bundle.key.channels)
    manifest = bundle.runtime_manifest
    tables = MappingProxyType(
        {
            "t97": load_book_table(
                "t97", manifest.resource("vorbis.codebooks.t97")
            ),
            "t219": load_book_table(
                "t219", manifest.resource("vorbis.codebooks.t219")
            ),
        }
    )
    return EncoderProfileResources(
        analysis=assemble_analysis_resources(bundle),
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
