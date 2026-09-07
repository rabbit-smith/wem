#!/usr/bin/env python3
"""Immutable window and band configuration for transient detection."""
from __future__ import annotations

import struct
from functools import lru_cache

from ..analysis.config import TransientBandConfig, TransientDetectorTables
from wwise_wem.profiles.resources import ResourceRef


def _u32_f32(value: int) -> float:
    return struct.unpack("<f", struct.pack("<I", int(value) & 0xFFFFFFFF))[0]


@lru_cache(maxsize=None)
def load_transient_tables(ref: ResourceRef) -> TransientDetectorTables:
    """Load the checked n=128 static detector table."""
    if not isinstance(ref, ResourceRef):
        raise TypeError("transient resource must be ResourceRef")
    data = ref.read_json()
    if data.get("schema") != "wem.transient-detector-table.v1":
        raise ValueError("transient table schema changed")
    if data.get("sample_rate") != 44100 or data.get("n") != 128:
        raise ValueError("transient table geometry changed")
    window_u32 = data.get("window_u32")
    config_u32 = data.get("config_u32")
    rows = data.get("bands")
    if not isinstance(window_u32, list) or len(window_u32) != 128:
        raise ValueError("transient window must contain 128 float words")
    if not isinstance(config_u32, list) or len(config_u32) != 26:
        raise ValueError("transient config must contain 26 float words")
    if not isinstance(rows, list) or len(rows) != 12:
        raise ValueError("transient descriptor table must contain twelve bands")
    bands: list[TransientBandConfig] = []
    for row in rows:
        weights_u32 = row.get("weights_u32")
        count = row.get("count")
        if (
            not isinstance(count, int)
            or not isinstance(weights_u32, list)
            or len(weights_u32) != count
            or not isinstance(row.get("offset"), int)
            or not isinstance(row.get("scale_u32"), int)
        ):
            raise ValueError("transient descriptor row is malformed")
        bands.append(TransientBandConfig(
            offset=int(row["offset"]),
            weights=tuple(_u32_f32(value) for value in weights_u32),
            scale=_u32_f32(int(row["scale_u32"])),
        ))
    return TransientDetectorTables(
        n=128,
        bias=_u32_f32(int(data["bias_u32"])),
        window=tuple(_u32_f32(value) for value in window_u32),
        config=tuple(_u32_f32(value) for value in config_u32),
        bands=tuple(bands),
    )
