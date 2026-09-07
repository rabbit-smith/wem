#!/usr/bin/env python3
"""Installed Wwise Vorbis encoder profiles and exact profile registry."""
from __future__ import annotations

from types import MappingProxyType
from typing import Iterable, Iterator, Mapping, TypeVar

from .bundle import ProfileKey, WWISE_GENERATION, load_profile_bundle
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

    def resolve_setup(
        self,
        channels: int,
        sample_rate: int,
        setup_sha256: str,
    ) -> EncoderProfile:
        """Resolve a complete profile identity carried by a template setup."""
        geometry = (int(channels), int(sample_rate))
        matches = self._by_geometry.get(geometry, ())
        selected = [
            profile
            for profile in matches
            if profile.setup_sha256 == str(setup_sha256).lower()
        ]
        if len(selected) == 1:
            return selected[0]
        installed = ", ".join(
            f"{profile.name}({profile.setup_sha256})" for profile in matches
        ) or "none"
        raise ValueError(
            f"template setup SHA-256 {setup_sha256} has no installed profile "
            f"for {geometry[0]}ch/{geometry[1]}Hz; installed: {installed}"
        )

    def list(self) -> tuple[EncoderProfile, ...]:
        return tuple(self._by_name[name] for name in sorted(self._by_name))

    @property
    def profiles_by_name(self) -> Mapping[str, EncoderProfile]:
        return self._by_name


_WWISE2013_6CH_44100_BUNDLE = load_profile_bundle(verify_all=False)
WWISE2013_6CH_44100_SETUP_SHA256 = _WWISE2013_6CH_44100_BUNDLE.setup.sha256
WWISE2013_RUNTIME_MANIFEST_SHA256 = (
    _WWISE2013_6CH_44100_BUNDLE.runtime_manifest.ref.sha256
)
WWISE2013_6CH_44100 = EncoderProfile(
    name=_WWISE2013_6CH_44100_BUNDLE.name,
    key=_WWISE2013_6CH_44100_BUNDLE.key,
    setup_path=_WWISE2013_6CH_44100_BUNDLE.setup,
    setup_sha256=WWISE2013_6CH_44100_SETUP_SHA256,
    block_sizes=_WWISE2013_6CH_44100_BUNDLE.block_sizes,
    container_metadata=_WWISE2013_6CH_44100_BUNDLE.container_metadata,
)


PROFILE_REGISTRY = ProfileRegistry((WWISE2013_6CH_44100,))
PROFILES = PROFILE_REGISTRY.profiles_by_name


def load_wem_profile(name: str) -> EncoderProfile:
    profile = PROFILE_REGISTRY.get(str(name))
    if profile is None:
        raise ValueError(
            f"unknown WEM profile {name!r}; available={','.join(sorted(PROFILES))}"
        )
    return profile


def resolve_wem_profile(channels: int, sample_rate: int) -> EncoderProfile:
    """Resolve an installed exact profile from WAV geometry."""
    return PROFILE_REGISTRY.resolve(int(channels), int(sample_rate))
