"""Bit-exact Wwise Vorbis WEM encoder."""

from __future__ import annotations

from typing import Any

from .api import encode_wav
from .application.models import EncodeResult, EncodeStats
from .model import (
    ContainerMetadata,
    PacketResult,
    PcmBuffer,
    SetupConfig,
)
from .profiles import (
    PROFILES,
    PROFILE_REGISTRY,
    EncoderProfile,
    ProfileKey,
    ProfileRegistry,
    WwiseVorbisProfile,
    load_wem_profile,
    resolve_wem_profile,
)


_LEGACY_EXPORTS = {"encode_wav_to_wem", "read_pcm16_wav"}
_LAZY_EXPORTS = {*_LEGACY_EXPORTS, "Encoder"}


def __getattr__(name: str) -> Any:
    """Load the encoder core and compatibility functions only on demand."""
    if name == "Encoder":
        from .application.encoder import Encoder

        return Encoder
    if name in _LEGACY_EXPORTS:
        from .application import compat

        return getattr(compat, name)
    raise AttributeError(f"module {__name__!r} has no attribute {name!r}")


def __dir__() -> list[str]:
    return sorted(set(globals()) | _LAZY_EXPORTS)


__all__ = [
    "PROFILES",
    "PROFILE_REGISTRY",
    "ContainerMetadata",
    "Encoder",
    "EncoderProfile",
    "EncodeResult",
    "EncodeStats",
    "PacketResult",
    "PcmBuffer",
    "ProfileKey",
    "ProfileRegistry",
    "SetupConfig",
    "WwiseVorbisProfile",
    "encode_wav",
    "encode_wav_to_wem",
    "load_wem_profile",
    "read_pcm16_wav",
    "resolve_wem_profile",
]
