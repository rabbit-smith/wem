from __future__ import annotations

from functools import lru_cache

from wwise_wem_reference.analysis.config import AnalysisProfileResources
from wwise_wem_reference.profiles.assembly import assemble_analysis_resources
from wwise_wem.profiles.bundle import load_profile_bundle


@lru_cache(maxsize=1)
def installed_analysis_resources() -> AnalysisProfileResources:
    return assemble_analysis_resources(load_profile_bundle(verify_all=False))
