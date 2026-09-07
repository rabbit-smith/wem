"""Bit-exact Wwise Vorbis WEM encoder."""

from __future__ import annotations

from typing import Any

from .api import encode_pcm_wav, encode_raw_pcm, encode_wav
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


_LEGACY_EXPORTS = {"read_pcm16_wav"}
_ADAPTER_EXPORTS = {"read_pcm_wav", "read_raw_pcm"}
_LAZY_EXPORTS = {*_LEGACY_EXPORTS, "Encoder", *_ADAPTER_EXPORTS}


def __getattr__(name: str) -> Any:
    """Load the encoder core and compatibility functions only on demand."""
    if name == "Encoder":
        from .application.encoder import Encoder

        return Encoder
    if name in _LEGACY_EXPORTS:
        from .application import compat

        return getattr(compat, name)
    if name in _ADAPTER_EXPORTS:
        if name == "read_pcm_wav":
            from .adapters import wav as wav_adapter

            return getattr(wav_adapter, name)
        from .adapters import raw as raw_adapter

        return getattr(raw_adapter, name)
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
    "encode_pcm_wav",
    "encode_raw_pcm",
    "encode_wav",
    "load_wem_profile",
    "read_pcm16_wav",
    "read_pcm_wav",
    "read_raw_pcm",
    "resolve_wem_profile",
]
