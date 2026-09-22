"""Read the compiled profile tables the kernel exposes.

The profile values are external facts measured from the paired build. They live
in Rust now (``crates/wem-profiles/src/generated``), and the compiled artifact
is the only carrier: this module decodes the canonical little-endian dump the
kernel hands out (``wwise_wem._core.profile_tables()``) into flat, named
bit-pattern tables.

What that does and does not change
----------------------------------
The reference implementation remains the specification of the encoder's
*algorithm and ordering*: which bank feeds which transform, how the tone table
nests, how a quality value walks the breakpoint axis, how the transient record
family materializes, how the recordings are packed. None of that is in the
dump, and none of it moved. What moved is where the *numbers* come from — the
same numbers the recorded resource documents carried, transferred as stored
bits instead of as decimal text.

Floats arrive as their IEEE bit patterns, so decoding is exact: a value is
reconstructed with ``struct`` and never by parsing a decimal string.
"""

from __future__ import annotations

import struct
from dataclasses import dataclass, field
from typing import Any

MAGIC = b"WEMPROF\0"
VERSION = 1

KIND_U32 = 0
KIND_I64 = 1
KIND_F32 = 2
KIND_F64 = 3
KIND_STR = 4
KIND_U64 = 6

_WIDTHS = {KIND_U32: 4, KIND_I64: 8, KIND_F32: 4, KIND_F64: 8, KIND_U64: 8}


class ProfileBlobError(ValueError):
    """The kernel's profile dump is not a stream this reader understands."""


class _Reader:
    def __init__(self, payload: bytes) -> None:
        self.payload = payload
        self.offset = 0

    def take(self, count: int) -> bytes:
        end = self.offset + count
        if end > len(self.payload):
            raise ProfileBlobError("profile dump ends before its last field")
        chunk = self.payload[self.offset : end]
        self.offset = end
        return chunk

    def u8(self) -> int:
        return self.take(1)[0]

    def u32(self) -> int:
        return struct.unpack("<I", self.take(4))[0]

    def i64(self) -> int:
        return struct.unpack("<q", self.take(8))[0]

    def text(self) -> str:
        return self.take(self.u32()).decode("utf-8")

    def raw(self) -> bytes:
        return self.take(self.u32())


def _unpack(kind: int, count: int, payload: bytes) -> Any:
    if kind == KIND_U32:
        return list(struct.unpack(f"<{count}I", payload))
    if kind == KIND_U64:
        return list(struct.unpack(f"<{count}Q", payload))
    if kind == KIND_I64:
        return list(struct.unpack(f"<{count}q", payload))
    if kind == KIND_F32:
        return [struct.unpack("<f", payload[index * 4 : index * 4 + 4])[0] for index in range(count)]
    if kind == KIND_F64:
        return [struct.unpack("<d", payload[index * 8 : index * 8 + 8])[0] for index in range(count)]
    if kind == KIND_STR:
        return payload.decode("utf-8")
    raise ProfileBlobError(f"unknown profile table kind {kind}")


@dataclass(frozen=True)
class CompiledProfile:
    """One profile exactly as the kernel compiled it."""

    name: str
    generation: str
    channel_layout: str
    quality_setup_identity: str
    channels: int
    sample_rate: int
    block_sizes: tuple[int, int]
    setup_sha256: str
    setup_packet: bytes
    container_fields: tuple[int, ...]
    tables: dict[str, Any] = field(default_factory=dict)

    def table(self, key: str) -> Any:
        """One named table; a missing key is a reader/kernel disagreement."""
        try:
            return self.tables[key]
        except KeyError as error:
            raise ProfileBlobError(f"profile {self.name} has no table {key!r}") from error

    def int(self, key: str) -> int:
        return int(self.table(key)[0])

    def floats(self, key: str) -> list[float]:
        return list(self.table(key))


def _read_profile(reader: _Reader) -> CompiledProfile:
    name = reader.text()
    generation = reader.text()
    channel_layout = reader.text()
    quality_setup_identity = reader.text()
    channels = reader.i64()
    sample_rate = reader.i64()
    block0 = reader.i64()
    block1 = reader.i64()
    setup_sha256 = reader.text()
    setup_packet = reader.raw()

    tables: dict[str, Any] = {}
    for _ in range(reader.u32()):
        key = reader.text()
        kind = reader.u8()
        count = reader.u32()
        if kind in _WIDTHS:
            tables[key] = _unpack(kind, count, reader.take(_WIDTHS[kind] * count))
        elif kind == KIND_STR:
            tables[key] = _unpack(kind, count, reader.take(count))
        else:
            raise ProfileBlobError(f"unknown profile table kind {kind} for {key!r}")

    return CompiledProfile(
        name=name,
        generation=generation,
        channel_layout=channel_layout,
        quality_setup_identity=quality_setup_identity,
        channels=channels,
        sample_rate=sample_rate,
        block_sizes=(block0, block1),
        setup_sha256=setup_sha256,
        setup_packet=setup_packet,
        container_fields=tuple(tables["container.fields"]),
        tables=tables,
    )


def decode(payload: bytes) -> tuple[CompiledProfile, ...]:
    """Decode the kernel's profile dump."""
    reader = _Reader(payload)
    if reader.take(len(MAGIC)) != MAGIC:
        raise ProfileBlobError("profile dump does not start with the WEMPROF magic")
    version = reader.u32()
    if version != VERSION:
        raise ProfileBlobError(f"unsupported profile dump version {version}")
    return tuple(_read_profile(reader) for _ in range(reader.u32()))


def compiled_profiles() -> tuple[CompiledProfile, ...]:
    """Every profile the loaded kernel carries.

    The dump is read straight out of the extension: the artifact is the
    carrier, so there is no second copy to drift from it.
    """
    from wwise_wem import _core

    return decode(bytes(_core.profile_tables()))


def compiled_profile(profile: CompiledProfile, generation: str, channels: int, sample_rate: int) -> CompiledProfile:
    """Pick one compiled profile by its selection triple."""
    for candidate in compiled_profiles():
        if (candidate.generation, candidate.channels, candidate.sample_rate) == (
            generation,
            channels,
            sample_rate,
        ):
            return candidate
    raise ProfileBlobError(f"no compiled profile for {channels}ch/{sample_rate}Hz/{generation}")


__all__ = [
    "MAGIC",
    "VERSION",
    "CompiledProfile",
    "ProfileBlobError",
    "compiled_profile",
    "compiled_profiles",
    "decode",
]
