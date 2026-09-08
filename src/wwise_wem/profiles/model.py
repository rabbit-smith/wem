"""Immutable encoder profile value model."""
from __future__ import annotations

import hashlib
import math
from dataclasses import dataclass
from pathlib import Path
from types import MappingProxyType
from typing import Any, Mapping

from ..model import ContainerMetadata
from .resources import ResourceRef
from .bundle import ProfileKey, load_profile_bundle


def _read_profile_source(
    source: Path | ResourceRef,
    expected_sha256: str,
    *,
    profile_name: str,
    label: str,
) -> bytes:
    payload = source.read_bytes()
    digest = hashlib.sha256(payload).hexdigest()
    if digest != expected_sha256:
        raise ValueError(
            f"profile {profile_name} {label} checksum differs: {digest}"
        )
    return payload


@dataclass(frozen=True)
class EncoderProfile:
    """Complete immutable identity, setup and container defaults for encoding."""

    name: str
    key: ProfileKey
    setup_path: Path | ResourceRef
    setup_sha256: str
    block_sizes: tuple[int, int]
    container_metadata: ContainerMetadata
    endian: str = "le"
    seek_table: bytes = b""
    extra_chunks: tuple[tuple[bytes, bytes], ...] = ()
    quality: float | None = None

    def __post_init__(self) -> None:
        if not self.name:
            raise ValueError("profile name must not be empty")
        if not isinstance(self.key, ProfileKey):
            raise TypeError("profile key must be ProfileKey")
        if not isinstance(self.container_metadata, ContainerMetadata):
            raise TypeError("profile container metadata must be ContainerMetadata")
        if self.quality is not None:
            quality = float(self.quality)
            if not math.isfinite(quality):
                raise ValueError("profile quality must be a finite number")
            object.__setattr__(self, "quality", quality)
        if (self.channels, self.sample_rate) != (
            self.container_metadata.nChannels,
            self.container_metadata.nSamplesPerSec,
        ):
            raise ValueError("profile key geometry differs from container metadata")
        if self.key.quality_setup_identity != f"sha256:{self.setup_sha256}":
            raise ValueError("profile key quality/setup identity differs from setup SHA-256")
        if not isinstance(self.setup_path, ResourceRef):
            object.__setattr__(self, "setup_path", Path(self.setup_path))
        block_sizes = tuple(int(size) for size in self.block_sizes)
        if len(block_sizes) != 2 or any(size <= 0 for size in block_sizes):
            raise ValueError("profile block sizes must contain two positive sizes")
        expected_blocks = (
            1 << self.container_metadata.uBlocksize0Pow,
            1 << self.container_metadata.uBlocksize1Pow,
        )
        if block_sizes != expected_blocks:
            raise ValueError("profile block sizes differ from container metadata")
        object.__setattr__(self, "block_sizes", block_sizes)
        object.__setattr__(self, "seek_table", bytes(self.seek_table))
        object.__setattr__(
            self,
            "extra_chunks",
            tuple(
                (bytes(chunk_id), bytes(payload))
                for chunk_id, payload in self.extra_chunks
            ),
        )

    @property
    def channels(self) -> int:
        return self.key.channels

    @property
    def sample_rate(self) -> int:
        return self.key.sample_rate

    @property
    def fmt(self) -> dict[str, int]:
        """Return a fresh legacy fmt dictionary for compatibility adapters."""
        return self.container_metadata.to_fmt_dict(
            frame_count=self.container_metadata.dwTotalPCMFrames
        )

    def setup_packet(self) -> bytes:
        return _read_profile_source(
            self.setup_path,
            self.setup_sha256,
            profile_name=self.name,
            label="setup",
        )

    def runtime_manifest(self) -> Mapping[str, Any]:
        """Return the installed profile manifest for this profile identity."""
        bundle = load_profile_bundle(profile=self.name, verify_all=False)
        if bundle.key != self.key:
            raise ValueError(
                f"profile {self.name} differs from installed profile bundle"
            )
        if self.setup_sha256 != bundle.setup.sha256:
            raise ValueError(
                f"profile {self.name} setup differs from installed profile bundle"
            )
        manifest = bundle.runtime_manifest
        files = {
            ref.path.relative_to(manifest.ref.path.parent).as_posix(): ref.sha256
            for ref in manifest.resources.values()
        }
        return MappingProxyType(
            {
                "schema": manifest.schema,
                "resources": MappingProxyType(
                    {
                        name: MappingProxyType(
                            {
                                "path": ref.path.relative_to(
                                    manifest.ref.path.parent
                                ).as_posix(),
                                "sha256": ref.sha256,
                            }
                        )
                        for name, ref in manifest.resources.items()
                    }
                ),
                "files": MappingProxyType(dict(files)),
            }
        )


WwiseVorbisProfile = EncoderProfile
