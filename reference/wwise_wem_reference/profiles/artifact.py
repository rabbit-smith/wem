"""Read the compiled profile tables the kernel exposes.

The profile values are external facts measured from the paired build. They live
in Rust now (``crates/wem-profiles/src/generated``), and the compiled artifact
is the only carrier: this module decodes the canonical little-endian dump the
kernel hands out (``wwise_wem._core.profile_tables()``) into flat, named
bit-pattern tables, and resolves a caller's structured profile selection
against them.

There is no second source to resolve against: no resource tree, no index, no
manifest, no digest chain, no profile name and no filesystem read. A profile's
human label is derived from its identity (:mod:`wwise_wem.profiles.key`), and
every typed table is reshaped here exactly as the kernel's own carrier does.

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

import math
import struct
from dataclasses import dataclass, field, replace
from functools import lru_cache
from typing import Any

from wwise_wem.model import ContainerMetadata
from wwise_wem.profiles.key import ProfileKey, profile_label

MAGIC = b"WEMPROF\0"
#: Version 2 dropped the stored profile name (the label is derived now);
#: version 3 dropped the stored setup digest (the key's identity names the
#: packet and the packet's own bytes travel in the stream).
VERSION = 3

KIND_U32 = 0
KIND_I64 = 1
KIND_F32 = 2
KIND_F64 = 3
KIND_STR = 4
KIND_U64 = 6

_WIDTHS = {KIND_U32: 4, KIND_I64: 8, KIND_F32: 4, KIND_F64: 8, KIND_U64: 8}

#: The container metadata fields, in the order ``container.fields`` carries
#: them (``wwise_wem.model.ContainerMetadata`` names, kernel field order).
_CONTAINER_FIELDS = (
    "wFormatTag",
    "nChannels",
    "nSamplesPerSec",
    "nAvgBytesPerSec",
    "nBlockAlign",
    "wBitsPerSample",
    "cbSize",
    "wReserved0",
    "dwChannelMask",
    "dwTotalPCMFrames",
    "dwFirstAudioPacketOffset",
    "dwDataPayloadSize",
    "dwUnknown_0x24",
    "dwSeekTableSize",
    "dwVorbisDataOffset",
    "uMaxPacketSize",
    "uUnknown_0x32",
    "dwUnknown_0x34",
    "dwUnknown_0x38",
    "dwUnknown_0x3C",
    "uBlocksize0Pow",
    "uBlocksize1Pow",
)


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


@dataclass(frozen=True, eq=False)
class CompiledProfile:
    """One profile exactly as the kernel compiled it.

    The container-plan fields (``endian``, ``seek_table``, ``extra_chunks``)
    and ``quality`` are the same fixed defaults the kernel's own profile value
    model carries: a compiled profile has no recorded endianness or extra
    chunks, and ``quality`` is bound by a selection, never by the carrier.
    """

    generation: str
    channel_layout: str
    quality_setup_identity: str
    channels: int
    sample_rate: int
    block_sizes: tuple[int, int]
    setup_packet: bytes
    container_fields: tuple[int, ...]
    #: The decoded table blocks, keyed by their names in the stream.
    tables: dict[str, Any] = field(default_factory=dict, repr=False)
    endian: str = "le"
    seek_table: bytes = b""
    extra_chunks: tuple[tuple[bytes, bytes], ...] = ()
    quality: float | None = None

    def _identity(self) -> tuple:
        """The cached equality/hash key: every header field plus the block
        names.

        The block *values* are left out — one carrier per identity determines
        them — but the block names and shapes are in, so a profile whose
        tables were replaced (a suite that injects a synthetic block) is not
        mistaken for the carrier it was derived from. That keeps the loaders'
        caches keyed on exactly what the identity decides.
        """
        return (
            self.generation,
            self.channel_layout,
            self.quality_setup_identity,
            self.channels,
            self.sample_rate,
            self.block_sizes,
            self.setup_packet,
            self.container_fields,
            tuple(
                (name, len(value) if isinstance(value, (list, bytes, str)) else 0)
                for name, value in sorted(self.tables.items())
            ),
        )

    def __eq__(self, other: object) -> bool:
        if not isinstance(other, CompiledProfile):
            return NotImplemented
        return self._identity() == other._identity()

    def __hash__(self) -> int:
        return hash(self._identity())

    def with_tables(self, **blocks: Any) -> "CompiledProfile":
        """A copy of this profile carrying extra or replaced table blocks.

        Used by suites that need a carrier shape the installed kernel does not
        ship (a profile with quality curves, for instance). The identity is
        unchanged and the copy compares unequal to the original.
        """
        tables = dict(self.tables)
        tables.update(blocks)
        return replace(self, tables=tables)

    @property
    def key(self) -> ProfileKey:
        """The profile's exact identity."""
        return ProfileKey(
            self.channels,
            self.sample_rate,
            self.generation,
            self.channel_layout,
            self.quality_setup_identity,
        )

    @property
    def container_metadata(self) -> ContainerMetadata:
        """The recorded container geometry as the value model spells it."""
        return ContainerMetadata(**dict(zip(_CONTAINER_FIELDS, self.container_fields)))

    @property
    def setup_available(self) -> bool:
        """Every compiled profile carries its setup packet."""
        return True

    @property
    def pending_reason(self) -> None:
        """A compiled profile is never a draft (no manifest, no pending state)."""
        return None

    def label(self) -> str:
        """The human label for this profile, derived from its identity."""
        return profile_label(self.channels, self.sample_rate, self.generation)

    def table(self, key: str) -> Any:
        """One named table; a missing key is a reader/kernel disagreement."""
        try:
            return self.tables[key]
        except KeyError as error:
            raise ProfileBlobError(f"profile {self.label()} has no table {key!r}") from error

    def optional_table(self, key: str) -> Any:
        """One named table, or ``None`` when the carrier does not record it."""
        return self.tables.get(key)

    def int(self, key: str) -> int:
        return int(self.table(key)[0])

    def floats(self, key: str) -> list[float]:
        return list(self.table(key))


def named_blocks(profile: "CompiledProfile", prefix: str, suffix: str) -> list[str]:
    """The names of the ``{prefix}{name}{suffix}`` blocks the carrier records.

    The dump is a flat, self-describing stream of named blocks, so a repeated
    table group is enumerated by its own names rather than by a count the
    reader would have to trust.
    """
    end = -len(suffix) if suffix else None
    return [
        key[len(prefix) : end]
        for key in profile.tables
        if key.startswith(prefix)
        and key.endswith(suffix)
        and len(key) > len(prefix) + len(suffix)
    ]


def _read_profile(reader: _Reader) -> CompiledProfile:
    generation = reader.text()
    channel_layout = reader.text()
    quality_setup_identity = reader.text()
    channels = reader.i64()
    sample_rate = reader.i64()
    block0 = reader.i64()
    block1 = reader.i64()
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
        generation=generation,
        channel_layout=channel_layout,
        quality_setup_identity=quality_setup_identity,
        channels=channels,
        sample_rate=sample_rate,
        block_sizes=(block0, block1),
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


@lru_cache(maxsize=1)
def compiled_profiles() -> tuple[CompiledProfile, ...]:
    """Every profile the loaded kernel carries.

    The dump is read straight out of the extension: the artifact is the
    carrier, so there is no second copy to drift from it.
    """
    from wwise_wem import _core

    # The kernel's tables cannot change within a process: decode once, so
    # every loader caches on one stable profile object.
    return decode(bytes(_core.profile_tables()))


def resolve_selection(
    selection: Any,
    quality: float | None = None,
) -> CompiledProfile:
    """Resolve one structured selection to the installed profile it denotes.

    The selection must denote exactly one installed profile: an unsatisfiable
    selection and an ambiguous one both raise :class:`ValueError` here — a
    selection is never satisfied by a first match. With ``quality`` omitted the
    compiled profile is returned as the carrier records it; with a quality
    value a copy carrying that value is returned, and the carrier is never
    mutated.
    """
    identity = (
        selection.version.generation,
        int(selection.channels),
        int(selection.sample_rate),
    )
    installed = compiled_profiles()
    matches = tuple(
        profile
        for profile in installed
        if (profile.generation, profile.channels, profile.sample_rate) == identity
    )
    if not matches:
        described = ", ".join(
            sorted(
                f"{profile.channels}ch/{profile.sample_rate}Hz/{profile.generation}"
                for profile in installed
            )
        )
        raise ValueError(
            f"no installed Wwise {selection.version.label} profile for "
            f"{identity[1]}ch/{identity[2]}Hz; installed: {described}"
        )
    if len(matches) > 1:
        names = ", ".join(sorted(profile.key.describe() for profile in matches))
        raise ValueError(
            f"Wwise {selection.version.label} profile selection "
            f"{identity[1]}ch/{identity[2]}Hz is ambiguous: {names}"
        )
    profile = matches[0]
    if quality is None:
        return profile
    value = float(quality)
    if not math.isfinite(value):
        raise ValueError("quality must be a finite number")
    return replace(profile, quality=value)


__all__ = [
    "MAGIC",
    "VERSION",
    "CompiledProfile",
    "ProfileBlobError",
    "compiled_profiles",
    "decode",
    "named_blocks",
    "resolve_selection",
]
