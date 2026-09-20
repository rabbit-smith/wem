#!/usr/bin/env python3
"""Installed Wwise Vorbis encoder profiles and exact profile registry."""
from __future__ import annotations

import dataclasses

from .bundle import (
    ProfileBundle,
    WWISE_GENERATION,
    installed_profile_names,
    load_profile_bundle,
)
from .model import EncoderProfile


def _profile_from_bundle(bundle: ProfileBundle) -> EncoderProfile:
    return EncoderProfile(
        name=bundle.name,
        key=bundle.key,
        setup_path=bundle.setup,
        setup_sha256=bundle.setup.sha256,
        block_sizes=bundle.block_sizes,
        container_metadata=bundle.container_metadata,
    )


_INSTALLED_PROFILES = {
    name: _profile_from_bundle(
        load_profile_bundle(profile=name, verify_all=False)
    )
    for name in installed_profile_names()
}


def _required_profile(name: str) -> EncoderProfile:
    try:
        return _INSTALLED_PROFILES[name]
    except KeyError as error:
        raise ValueError(f"required built-in profile {name!r} is absent") from error


WWISE2013_6CH_44100 = _required_profile("wwise2013-6ch-44100")
WWISE2013_2CH_48000 = _required_profile("wwise2013-2ch-48000")


def load_wem_profile(name: str, quality: float | None = None) -> EncoderProfile:
    """Return the installed profile, optionally bound to a quality factor.

    With ``quality`` omitted the cached instance is returned. With a quality
    value, a copy carrying that value is returned; the installed profile is
    never mutated.
    """
    profile = _INSTALLED_PROFILES.get(str(name))
    if profile is None:
        raise ValueError(
            f"unknown WEM profile {name!r}; "
            f"available={','.join(sorted(_INSTALLED_PROFILES))}"
        )
    if quality is None:
        return profile
    return dataclasses.replace(profile, quality=float(quality))


def resolve_wem_profile(
    channels: int,
    sample_rate: int,
    quality: float | None = None,
) -> EncoderProfile:
    """Resolve an installed exact profile from WAV geometry."""
    geometry = (int(channels), int(sample_rate))
    matches = [
        profile
        for profile in _INSTALLED_PROFILES.values()
        if (profile.channels, profile.sample_rate) == geometry
    ]
    if not matches:
        installed = ", ".join(
            f"{item.channels}ch/{item.sample_rate}Hz"
            for item in sorted(_INSTALLED_PROFILES.values(), key=lambda item: item.name)
        )
        raise ValueError(
            f"no Wwise {WWISE_GENERATION} profile for "
            f"{geometry[0]}ch/{geometry[1]}Hz; installed: {installed}"
        )
    if len(matches) > 1:
        names = ", ".join(sorted(profile.name for profile in matches))
        raise ValueError(
            f"multiple Wwise {WWISE_GENERATION} profiles match "
            f"{geometry[0]}ch/{geometry[1]}Hz: {names}; select one by name"
        )
    profile = matches[0]
    if quality is None:
        return profile
    return dataclasses.replace(profile, quality=float(quality))


def profile_names() -> tuple[str, ...]:
    """Return installed profile names in stable order."""
    return tuple(sorted(_INSTALLED_PROFILES))
