"""Immutable container plan for the reference encoder."""

from __future__ import annotations

from dataclasses import dataclass
from types import MappingProxyType
from typing import Any, Mapping

from ..profiles.artifact import CompiledProfile


@dataclass(frozen=True)
class ContainerPlan:
    """Per-output container metadata, kept separate from codec identity.

    The plan carries the profile's container metadata and nothing that
    describes *which* profile it came from, mirroring the kernel's
    `ContainerPlan`: the selection is the caller's own argument, so a label
    naming it back is not a second observation.
    """

    fmt: Mapping[str, Any]
    endian: str
    seek_table: bytes
    extra_chunks: tuple[tuple[bytes, bytes], ...]

    def __post_init__(self) -> None:
        if not isinstance(self.fmt, Mapping):
            raise TypeError("container fmt must be a mapping")
        if self.endian not in ("le", "be"):
            raise ValueError("container endian must be 'le' or 'be'")
        extras: list[tuple[bytes, bytes]] = []
        for chunk_id, payload in self.extra_chunks:
            chunk_id = bytes(chunk_id)
            if len(chunk_id) != 4:
                raise ValueError("container extra chunk id must be exactly 4 bytes")
            extras.append((chunk_id, bytes(payload)))
        object.__setattr__(self, "fmt", MappingProxyType(dict(self.fmt)))
        object.__setattr__(self, "seek_table", bytes(self.seek_table))
        object.__setattr__(self, "extra_chunks", tuple(extras))

    @classmethod
    def from_profile(cls, profile: CompiledProfile) -> "ContainerPlan":
        return cls(
            fmt=profile.container_metadata.to_fmt_dict(
                frame_count=profile.container_metadata.dwTotalPCMFrames
            ),
            endian=profile.endian,
            seek_table=profile.seek_table,
            extra_chunks=profile.extra_chunks,
        )


__all__ = ["ContainerPlan"]
