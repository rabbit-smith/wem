"""Encoder profile identity, bundles, and installed registry."""

from .bundle import (
    ProfileBundle,
    ProfileKey,
    RuntimeResourceManifest,
    load_profile_bundle,
)
from .model import EncoderProfile, WwiseVorbisProfile
from .registry import (
    PROFILES,
    PROFILE_REGISTRY,
    WWISE2013_6CH_44100,
    WWISE2013_6CH_44100_SETUP_SHA256,
    WWISE2013_RUNTIME_MANIFEST_SHA256,
    ProfileRegistry,
    load_wem_profile,
    resolve_wem_profile,
)

__all__ = [
    "PROFILES",
    "PROFILE_REGISTRY",
    "EncoderProfile",
    "ProfileBundle",
    "ProfileKey",
    "ProfileRegistry",
    "RuntimeResourceManifest",
    "WWISE2013_6CH_44100",
    "WWISE2013_6CH_44100_SETUP_SHA256",
    "WWISE2013_RUNTIME_MANIFEST_SHA256",
    "WwiseVorbisProfile",
    "load_profile_bundle",
    "load_wem_profile",
    "resolve_wem_profile",
]
