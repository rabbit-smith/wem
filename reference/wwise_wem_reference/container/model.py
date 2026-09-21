"""Immutable container plan for the reference encoder."""

from __future__ import annotations

from dataclasses import dataclass
from types import MappingProxyType
from typing import Any, Mapping

from wwise_wem.profiles.bundle import WWISE_GENERATION_LABEL
from wwise_wem.profiles.model import EncoderProfile


@dataclass(frozen=True)
class ContainerPlan:
    """Per-output container metadata, kept separate from codec identity."""

    fmt: Mapping[str, Any]
    endian: str
    seek_table: bytes
    extra_chunks: tuple[tuple[bytes, bytes], ...]
    metadata_source: str

    def __post_init__(self) -> None:
        if not isinstance(self.fmt, Mapping):
            raise TypeError("container fmt must be a mapping")
        if self.endian not in ("le", "be"):
            raise ValueError("container endian must be 'le' or 'be'")
        if not isinstance(self.metadata_source, str) or not self.metadata_source:
            raise ValueError("metadata_source must be a non-empty string")
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
    def from_profile(cls, profile: EncoderProfile) -> "ContainerPlan":
        # The provenance label names the selected configuration, not the
        # bundle it happens to live in: it mirrors the kernel's
        # `ContainerPlan::from_profile`, so the oracle and the kernel report
        # the same `metadata_source` for the same selection.
        return cls(
            fmt=profile.container_metadata.to_fmt_dict(
                frame_count=profile.container_metadata.dwTotalPCMFrames
            ),
            endian=profile.endian,
            seek_table=profile.seek_table,
            extra_chunks=profile.extra_chunks,
            metadata_source=(
                f"profile:{profile.channels}ch/{profile.sample_rate}Hz/"
                f"{WWISE_GENERATION_LABEL}"
            ),
        )


__all__ = ["ContainerPlan"]
