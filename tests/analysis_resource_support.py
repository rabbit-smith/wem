from __future__ import annotations

from functools import lru_cache

from wwise_wem import WwiseProfile, WwiseVersion
from wwise_wem_reference.analysis.config import AnalysisProfileResources
from wwise_wem_reference.profiles.artifact import (
    CompiledProfile,
    compiled_profiles,
    resolve_selection,
)
from wwise_wem_reference.profiles.assembly import assemble_analysis_resources


@lru_cache(maxsize=1)
def installed_analysis_resources() -> AnalysisProfileResources:
    """Analysis resources of the default installed configuration (6ch/44100)."""
    return assemble_analysis_resources(
        resolve_selection(WwiseProfile(WwiseVersion.DEFAULT, 6, 44100))
    )


def installed_profile(
    channels: int,
    sample_rate: int,
    generation: str = "2013.2",
) -> CompiledProfile:
    """The compiled profile one selection triple denotes.

    Resolution is identity-based on purpose: a profile label is not a
    selection surface, so the suites never pick a profile by name — the same
    rule the kernel's ``resolve_wem_profile_selection`` follows. The lookup
    goes through the carrier's own cached inventory, so this helper and
    :func:`resolve_selection` hand back the same object while the cache is
    warm.
    """
    matches = [
        profile
        for profile in compiled_profiles()
        if (profile.generation, profile.channels, profile.sample_rate)
        == (generation, channels, sample_rate)
    ]
    if len(matches) != 1:
        raise ValueError(
            f"no single installed profile for {channels}ch/{sample_rate}Hz/{generation}"
        )
    return matches[0]
