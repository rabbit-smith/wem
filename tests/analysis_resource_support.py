from __future__ import annotations

from functools import lru_cache

from wwise_wem_reference.analysis.config import AnalysisProfileResources
from wwise_wem_reference.profiles.assembly import assemble_analysis_resources
from wwise_wem.profiles.bundle import (
    ProfileBundle,
    installed_profile_names,
    load_profile_bundle,
)


@lru_cache(maxsize=1)
def installed_analysis_resources() -> AnalysisProfileResources:
    return assemble_analysis_resources(load_profile_bundle(verify_all=False))


@lru_cache(maxsize=None)
def installed_profile_bundle(
    channels: int,
    sample_rate: int,
    generation: str = "2013.2",
) -> ProfileBundle:
    """The installed bundle one profile key names, matched by that key.

    Resolution is key-based on purpose: a profile name is not a selection
    surface, so the suites never pick a bundle by name — the same rule the
    kernel's ``resolve_wem_profile_selection`` follows.
    """
    for name in installed_profile_names():
        bundle = load_profile_bundle(profile=name, verify_all=False)
        key = bundle.key
        if (key.generation, key.channels, key.sample_rate) == (
            generation,
            channels,
            sample_rate,
        ):
            return bundle
    raise ValueError(
        f"no installed profile bundle for {channels}ch/{sample_rate}Hz/{generation}"
    )
