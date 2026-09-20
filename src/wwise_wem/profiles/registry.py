#!/usr/bin/env python3
"""Installed Wwise Vorbis encoder profiles and exact profile registry."""
from __future__ import annotations

import dataclasses
from types import MappingProxyType
from typing import Iterable, Iterator, Mapping, TypeVar

from .bundle import (
    ProfileBundle,
    ProfileKey,
    WWISE_GENERATION,
    installed_profile_names,
    load_profile_bundle,
)
from .model import EncoderProfile


_DEFAULT = TypeVar("_DEFAULT")


class ProfileRegistry:
    """Read-only exact lookup by full profile key or stable profile name."""

    def __init__(self, profiles: Iterable[EncoderProfile]) -> None:
        by_key: dict[ProfileKey, EncoderProfile] = {}
        by_name: dict[str, EncoderProfile] = {}
        by_geometry: dict[tuple[int, int], list[EncoderProfile]] = {}
        for profile in tuple(profiles):
            if not isinstance(profile, EncoderProfile):
                raise TypeError("profile registry entries must be EncoderProfile")
            if profile.key in by_key:
                raise ValueError(f"duplicate profile key: {profile.key}")
            if profile.name in by_name:
                raise ValueError(f"duplicate profile name: {profile.name}")
            by_key[profile.key] = profile
            by_name[profile.name] = profile
            by_geometry.setdefault((profile.channels, profile.sample_rate), []).append(
                profile
            )
        self._by_key: Mapping[ProfileKey, EncoderProfile] = MappingProxyType(by_key)
        self._by_name: Mapping[str, EncoderProfile] = MappingProxyType(by_name)
        self._by_geometry: Mapping[
            tuple[int, int], tuple[EncoderProfile, ...]
        ] = MappingProxyType(
            {geometry: tuple(matches) for geometry, matches in by_geometry.items()}
        )

    def __getitem__(self, key: ProfileKey) -> EncoderProfile:
        return self._by_key[key]

    def __iter__(self) -> Iterator[ProfileKey]:
        return iter(self._by_key)

    def __len__(self) -> int:
        return len(self._by_key)

    def keys(self):
        return self._by_key.keys()

    def values(self):
        return self._by_key.values()

    def items(self):
        return self._by_key.items()

    def resolve(
        self,
        channels: int | ProfileKey,
        sample_rate: int | None = None,
    ) -> EncoderProfile:
        if isinstance(channels, ProfileKey):
            try:
                return self._by_key[channels]
            except KeyError as error:
                raise ValueError(f"unknown WEM profile key {channels}") from error
        if sample_rate is None:
            raise TypeError("sample_rate is required when resolving by channels")
        geometry = (int(channels), int(sample_rate))
        matches = self._by_geometry.get(geometry, ())
        if not matches:
            installed = ", ".join(
                f"{item.channels}ch/{item.sample_rate}Hz" for item in self.list()
            )
            raise ValueError(
                f"no Wwise {WWISE_GENERATION} profile for "
                f"{geometry[0]}ch/{geometry[1]}Hz; installed: {installed}"
            )
        if len(matches) != 1:
            names = ", ".join(profile.name for profile in matches)
            raise ValueError(
                f"profile geometry {geometry[0]}ch/{geometry[1]}Hz is ambiguous: "
                f"{names}; resolve with a complete ProfileKey"
            )
        return matches[0]

    def get(
        self,
        identity: str | ProfileKey,
        default: _DEFAULT | None = None,
    ) -> EncoderProfile | _DEFAULT | None:
        """Return a profile or ``default``, matching ``dict.get`` semantics."""
        mapping = self._by_name if isinstance(identity, str) else self._by_key
        return mapping.get(identity, default)  # type: ignore[arg-type, return-value]

    def list(self) -> tuple[EncoderProfile, ...]:
        return tuple(self._by_name[name] for name in sorted(self._by_name))

    @property
    def profiles_by_name(self) -> Mapping[str, EncoderProfile]:
        return self._by_name


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


PROFILE_REGISTRY = ProfileRegistry(_INSTALLED_PROFILES.values())
PROFILES = PROFILE_REGISTRY.profiles_by_name


def load_wem_profile(name: str, quality: float | None = None) -> EncoderProfile:
    """Return the installed profile, optionally bound to a quality factor.

    With ``quality`` omitted the registry's cached instance is returned exactly
    as before. With a quality value, a copy of the profile carrying that quality
    is returned; the quality is applied during analysis-resource assembly. The
    registered profile itself is never mutated.
    """
    profile = PROFILE_REGISTRY.get(str(name))
    if profile is None:
        raise ValueError(
            f"unknown WEM profile {name!r}; available={','.join(sorted(PROFILES))}"
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
    profile = PROFILE_REGISTRY.resolve(int(channels), int(sample_rate))
    if quality is None:
        return profile
    return dataclasses.replace(profile, quality=float(quality))
