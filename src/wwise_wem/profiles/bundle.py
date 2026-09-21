"""Validated, zip-safe encoder profile bundles."""

from __future__ import annotations

import json
from dataclasses import dataclass
from functools import lru_cache
from pathlib import PurePosixPath
from types import MappingProxyType
from typing import Any, Mapping

from ..model import ContainerMetadata
from .resources import ResourceRef, normalize_resource_path, resource_traversable


PACKAGE = "wwise_wem"
DEFAULT_INDEX = "data/profiles/index.json"
INDEX_SCHEMA = "wwise-wem.profile-index.v1"
BUNDLE_SCHEMA = "wwise-wem.profile-manifest.v1"
MANIFEST_SCHEMA = BUNDLE_SCHEMA
WWISE_GENERATION = "2013.2"
# The short spelling of the same generation, as it appears in messages, on a
# command line, and in the kernel's `metadata_source`; mirrors the kernel's
# `WwiseVersion::label()` (the oracle and the tooling cannot read that type,
# because they must also run where the native extension is absent).
WWISE_GENERATION_LABEL = "2013"


@dataclass(frozen=True, order=True)
class ProfileKey:
    """Complete encoder profile identity."""

    channels: int
    sample_rate: int
    generation: str
    channel_layout: str
    quality_setup_identity: str

    def __post_init__(self) -> None:
        if self.channels <= 0 or self.sample_rate <= 0:
            raise ValueError("profile channels and sample rate must be positive")
        if not self.generation:
            raise ValueError("profile generation must not be empty")
        if not self.channel_layout:
            raise ValueError("profile channel layout must not be empty")
        if not self.quality_setup_identity:
            raise ValueError("profile quality/setup identity must not be empty")


def _mapping(value: Any, label: str) -> Mapping[str, Any]:
    if not isinstance(value, dict):
        raise ValueError(f"profile bundle {label} must be an object")
    return value


def _string(value: Any, label: str) -> str:
    if not isinstance(value, str) or not value:
        raise ValueError(f"profile bundle {label} must be non-empty text")
    return value


def _integer(value: Any, label: str) -> int:
    if not isinstance(value, int) or isinstance(value, bool):
        raise ValueError(f"profile bundle {label} must be an integer")
    return value


def _read_json(
    package: str,
    resource: str | PurePosixPath,
    label: str,
) -> Mapping[str, Any]:
    relative = normalize_resource_path(resource)
    try:
        raw = resource_traversable(package, relative).read_text(encoding="utf-8")
    except (FileNotFoundError, IsADirectoryError) as error:
        raise ValueError(f"missing profile {label} resource {relative}") from error
    try:
        return _mapping(json.loads(raw), label)
    except json.JSONDecodeError as error:
        raise ValueError(f"profile {label} {relative} is not valid JSON") from error


@dataclass(frozen=True)
class RuntimeResourceManifest:
    """Profile manifest and its checksum-verified logical resources."""

    ref: ResourceRef
    schema: str
    resources: Mapping[str, ResourceRef]

    def __post_init__(self) -> None:
        if self.schema != MANIFEST_SCHEMA:
            raise ValueError(f"unsupported profile manifest schema {self.schema!r}")
        if not self.resources:
            raise ValueError("profile manifest must contain at least one resource")
        object.__setattr__(self, "resources", MappingProxyType(dict(self.resources)))

    @classmethod
    def load(
        cls,
        ref: ResourceRef,
        payload: Mapping[str, Any] | None = None,
    ) -> "RuntimeResourceManifest":
        data = _mapping(ref.read_json() if payload is None else payload, "manifest")
        resource_data = _mapping(data.get("resources"), "manifest resources")
        resources: dict[str, ResourceRef] = {}
        for logical_name, raw_entry in resource_data.items():
            name = _string(logical_name, "resource logical name")
            entry = _mapping(raw_entry, f"resource {name}")
            path = normalize_resource_path(
                _string(entry.get("path"), f"resource {name}.path")
            )
            resources[name] = ResourceRef(
                ref.package,
                ref.path.parent / path,
                _string(entry.get("sha256"), f"resource {name}.sha256"),
            )
        return cls(ref, str(data.get("schema", "")), resources)

    def resource(self, name_or_path: str | PurePosixPath) -> ResourceRef:
        key = str(name_or_path)
        if key in self.resources:
            return self.resources[key]
        relative = normalize_resource_path(name_or_path).as_posix()
        for ref in self.resources.values():
            if ref.path.relative_to(self.ref.path.parent).as_posix() == relative:
                return ref
        raise ValueError(f"resource {key} is absent from profile manifest")

    def verify_all(self) -> None:
        for name in sorted(self.resources):
            self.resources[name].verify()


@dataclass(frozen=True)
class ProfileBundle:
    """Complete immutable profile identity and its runtime resource set.

    Draft bundles that still await a setup export carry
    ``setup_available == False`` and a manifest-declared
    ``pending_reason``; their setup-dependent accessors raise instead of
    silently forging a setup.
    """

    name: str
    key: ProfileKey
    container_metadata: ContainerMetadata
    runtime_manifest: RuntimeResourceManifest
    block_sizes: tuple[int, int]
    setup_available: bool = True
    pending_reason: str | None = None

    def __post_init__(self) -> None:
        if not self.name:
            raise ValueError("profile bundle name must not be empty")
        if (self.key.channels, self.key.sample_rate) != (
            self.container_metadata.nChannels,
            self.container_metadata.nSamplesPerSec,
        ):
            raise ValueError("profile bundle geometry differs from container metadata")
        blocks = tuple(int(value) for value in self.block_sizes)
        expected = (
            1 << self.container_metadata.uBlocksize0Pow,
            1 << self.container_metadata.uBlocksize1Pow,
        )
        if len(blocks) != 2 or blocks != expected:
            raise ValueError("profile bundle block sizes differ from container metadata")
        object.__setattr__(self, "block_sizes", blocks)
        if "vorbis.setup" in self.runtime_manifest.resources:
            if self.key.quality_setup_identity != f"sha256:{self.setup.sha256}":
                raise ValueError(
                    "profile bundle setup identity differs from setup SHA-256"
                )
        if bool(self.setup_available) != ("vorbis.setup" in self.runtime_manifest.resources):
            raise ValueError(
                "profile bundle setup_available differs from the manifest resources"
            )

    @property
    def setup(self) -> ResourceRef:
        if not self.setup_available:
            raise ValueError(
                "profile has no setup resource; "
                "requires paired Wwise export; encoding unavailable"
            )
        return self.runtime_manifest.resource("vorbis.setup")

    def setup_packet(self) -> bytes:
        return self.setup.read_bytes()

    def verify_all(self) -> None:
        self.runtime_manifest.verify_all()


def installed_profile_names(
    resource: str = DEFAULT_INDEX,
    *,
    package: str = PACKAGE,
) -> tuple[str, ...]:
    """Return the complete profile inventory declared by an index."""
    index_path = normalize_resource_path(resource)
    index = _read_json(package, index_path, "index")
    if index.get("schema") != INDEX_SCHEMA:
        raise ValueError(f"unsupported profile index schema {index.get('schema')!r}")
    profiles = _mapping(index.get("profiles"), "index.profiles")
    if not profiles:
        raise ValueError("profile index profiles must be a non-empty object")
    return tuple(_string(name, "index profile name") for name in profiles)


@lru_cache(maxsize=None)
def load_profile_bundle(
    resource: str = DEFAULT_INDEX,
    *,
    package: str = PACKAGE,
    verify_all: bool = True,
    profile: str | None = None,
) -> ProfileBundle:
    """Load a profile through the package index and its self-contained manifest."""
    index_path = normalize_resource_path(resource)
    index = _read_json(package, index_path, "index")
    if index.get("schema") != INDEX_SCHEMA:
        raise ValueError(f"unsupported profile index schema {index.get('schema')!r}")
    selected = profile or _string(index.get("default"), "index.default")
    profiles = _mapping(index.get("profiles"), "index.profiles")
    try:
        index_entry = _mapping(profiles[selected], f"index profile {selected}")
    except KeyError as error:
        raise ValueError(f"profile {selected!r} is absent from profile index") from error
    manifest_relative = normalize_resource_path(
        _string(index_entry.get("manifest"), "index profile manifest")
    )
    manifest_ref = ResourceRef(
        package,
        index_path.parent / manifest_relative,
        _string(index_entry.get("sha256"), "index profile sha256"),
    )
    payload = _mapping(manifest_ref.read_json(), "manifest")
    if payload.get("schema") != BUNDLE_SCHEMA:
        raise ValueError(f"unsupported profile manifest schema {payload.get('schema')!r}")
    key_data = _mapping(payload.get("key"), "key")
    try:
        key = ProfileKey(
            _integer(key_data["channels"], "key.channels"),
            _integer(key_data["sample_rate"], "key.sample_rate"),
            generation=_string(key_data["generation"], "key.generation"),
            channel_layout=_string(key_data["channel_layout"], "key.channel_layout"),
            quality_setup_identity=_string(
                key_data["quality_setup_identity"],
                "key.quality_setup_identity",
            ),
        )
        metadata = ContainerMetadata.from_fmt_dict(
            _mapping(payload.get("container_metadata"), "container_metadata")
        )
        block_data = payload["block_sizes"]
        if not isinstance(block_data, list):
            raise ValueError("profile bundle block_sizes must be an array")
        blocks = tuple(
            _integer(value, "block_sizes entry") for value in block_data
        )
    except (KeyError, TypeError, ValueError) as error:
        raise ValueError(f"profile manifest fields are malformed: {error}") from error
    runtime_manifest = RuntimeResourceManifest.load(manifest_ref, payload)
    setup_present = "vorbis.setup" in payload.get("resources", {})
    setup_available = payload.get("setup_available", setup_present)
    if not isinstance(setup_available, bool):
        raise ValueError("profile manifest setup_available must be a boolean")
    pending_reason = payload.get("pending_reason")
    if pending_reason is not None and (
        not isinstance(pending_reason, str) or not pending_reason
    ):
        raise ValueError("profile manifest pending_reason must be text")
    bundle = ProfileBundle(
        name=_string(payload.get("name"), "name"),
        key=key,
        container_metadata=metadata,
        runtime_manifest=runtime_manifest,
        block_sizes=blocks,  # type: ignore[arg-type]
        setup_available=setup_available,
        pending_reason=pending_reason,
    )
    if bundle.name != selected:
        raise ValueError("profile index name differs from profile manifest")
    if verify_all:
        bundle.verify_all()
    return bundle
