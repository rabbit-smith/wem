"""Static MDCT trig banks, read from the compiled profile artifact.

The banks are Rust constants in the kernel (``crates/wem-profiles/src/generated``)
and reach every other language through the compiled artifact
(:mod:`wwise_wem_reference.profiles.artifact`); nothing ships as a resource
tree, so there is no document left to decode here. The values arrive as stored
f32 words and the MDCT look is built from them exactly as the kernel's own
carrier builds it.
"""
from __future__ import annotations

from functools import lru_cache
from types import MappingProxyType
from typing import Mapping

from ..analysis.config import MdctLook
from ..analysis.dsp.transform import make_mdct_look
from .artifact import CompiledProfile, named_blocks

_MDCT_SIZES = frozenset({128, 256, 512, 1024, 2048})


@lru_cache(maxsize=None)
def load_mdct_looks(profile: CompiledProfile) -> Mapping[int, MdctLook]:
    """Rebuild every static trig bank the profile carries."""
    if not isinstance(profile, CompiledProfile):
        raise TypeError("MDCT tables require a CompiledProfile")
    result: dict[int, MdctLook] = {}
    for key in named_blocks(profile, "mdct.", ".key"):
        n = int(key)
        trig = tuple(float(word) for word in profile.table(f"mdct.{n}.trig"))
        result[n] = make_mdct_look(n, static_trig=trig)
    if set(result) != _MDCT_SIZES:
        raise ValueError("Wwise MDCT trig table profile set is incomplete")
    return MappingProxyType(result)


__all__ = ["load_mdct_looks"]
