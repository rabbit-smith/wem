#!/usr/bin/env python3
"""Selection-to-profile resolution over the packaged profile index.

Profile names, profile directories and profile paths are not part of any
caller-facing surface: a caller selects with a structured ``WwiseProfile``
(Wwise generation plus PCM geometry, see :mod:`wwise_wem.profiles.model`) and
the kernel resolves it against the configurations it carries.

This module is the development-tree mirror of that resolution, used by the
reference oracle, the parity suites and the tooling scripts, which need one
profile's materials rather than its bytes. It is deliberately not on the
encoding path: :func:`wwise_wem.api.encode` hands its ``WwiseProfile``
straight to the kernel, so the kernel stays the single authority on which
configuration a selection denotes. The resolver reads the packaged index and
manifest through :func:`~wwise_wem.profiles.bundle.load_profile_bundle` and
matches on the manifest key ``(generation, channels, sample_rate)``.
"""
from __future__ import annotations

import dataclasses
from functools import lru_cache
from typing import TYPE_CHECKING

from .bundle import ProfileBundle, installed_profile_names, load_profile_bundle
from .model import EncoderProfile

if TYPE_CHECKING:
    from .._core import WwiseProfile


def _profile_from_bundle(bundle: ProfileBundle) -> EncoderProfile:
    return EncoderProfile(
        name=bundle.name,
        key=bundle.key,
        setup_path=bundle.setup,
        setup_sha256=bundle.setup.sha256,
        block_sizes=bundle.block_sizes,
        container_metadata=bundle.container_metadata,
    )


@lru_cache(maxsize=1)
def _installed_profiles() -> tuple[EncoderProfile, ...]:
    """Every profile the packaged index installs, in index order."""
    return tuple(
        _profile_from_bundle(load_profile_bundle(profile=name, verify_all=False))
        for name in installed_profile_names()
    )


def _describe_installed(profiles: tuple[EncoderProfile, ...]) -> str:
    return ", ".join(
        f"{profile.channels}ch/{profile.sample_rate}Hz/{profile.key.generation}"
        for profile in sorted(profiles, key=lambda item: item.key)
    )


def resolve_selection(
    selection: WwiseProfile,
    quality: float | None = None,
) -> EncoderProfile:
    """Resolve one structured selection to the installed profile it denotes.

    The selection must denote exactly one installed profile: an unsatisfiable
    selection and an ambiguous one both raise :class:`ValueError` here — a
    selection is never satisfied by a first match. With ``quality`` omitted
    the cached installed profile is returned; with a quality value a copy
    carrying that value is returned, and the installed profile is never
    mutated.
    """
    identity = (
        selection.version.generation,
        int(selection.channels),
        int(selection.sample_rate),
    )
    installed = _installed_profiles()
    matches = tuple(
        profile
        for profile in installed
        if (profile.key.generation, profile.channels, profile.sample_rate)
        == identity
    )
    if not matches:
        raise ValueError(
            f"no installed Wwise {selection.version.label} profile for "
            f"{identity[1]}ch/{identity[2]}Hz; installed: "
            f"{_describe_installed(installed)}"
        )
    if len(matches) > 1:
        names = ", ".join(sorted(profile.name for profile in matches))
        raise ValueError(
            f"Wwise {selection.version.label} profile selection "
            f"{identity[1]}ch/{identity[2]}Hz is ambiguous: {names}"
        )
    profile = matches[0]
    if quality is None:
        return profile
    return dataclasses.replace(profile, quality=float(quality))


__all__ = ["resolve_selection"]
