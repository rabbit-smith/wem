"""Checksum-addressed MDCT table decoding for profile assembly."""
from __future__ import annotations

import base64
import struct
from functools import lru_cache
from types import MappingProxyType
from typing import Mapping

from ..analysis.config import MdctLook
from ..analysis.dsp.transform import make_mdct_look
from wwise_wem.profiles.resources import ResourceRef


@lru_cache(maxsize=None)
def load_mdct_looks(ref: ResourceRef) -> Mapping[int, MdctLook]:
    """Decode all static trig banks from an explicit manifest resource."""
    if not isinstance(ref, ResourceRef):
        raise TypeError("MDCT resource must be ResourceRef")
    payload = ref.read_json()
    if not isinstance(payload, dict) or payload.get("schema") != "wem.mdct-trig-static.v1":
        raise ValueError("unsupported Wwise MDCT trig-table schema")
    profiles = payload.get("profiles")
    if not isinstance(profiles, dict):
        raise ValueError("Wwise MDCT trig-table profiles are malformed")
    result: dict[int, MdctLook] = {}
    for key, profile in profiles.items():
        if not isinstance(profile, dict):
            raise ValueError("Wwise MDCT trig profile is malformed")
        n = int(key)
        expected_words = n + n // 4
        raw = base64.b64decode(
            profile["little_endian_f32_base64"], validate=True
        )
        if profile.get("word_count") != expected_words or len(raw) != 4 * expected_words:
            raise ValueError(f"malformed Wwise MDCT trig table for n={n}")
        trig = struct.unpack(f"<{expected_words}f", raw)
        result[n] = make_mdct_look(n, static_trig=trig)
    if set(result) != {128, 256, 512, 1024, 2048}:
        raise ValueError("Wwise MDCT trig table profile set is incomplete")
    return MappingProxyType(result)


__all__ = ["load_mdct_looks"]
