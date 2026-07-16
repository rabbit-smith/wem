"""Immutable models and legacy-dictionary adapters for WEM composition."""

from __future__ import annotations

from dataclasses import dataclass, field
from pathlib import Path
from types import MappingProxyType
from typing import Any, Mapping

from ..model import ContainerMetadata
from .riff import build_riff, parse_chunks


def _as_bytes(value: Any, label: str) -> bytes:
    if not isinstance(value, (bytes, bytearray, memoryview)):
        raise TypeError(f"{label} must be bytes-like")
    return bytes(value)


def _freeze(value: Any) -> Any:
    if isinstance(value, Mapping):
        return MappingProxyType({key: _freeze(item) for key, item in value.items()})
    if isinstance(value, (list, tuple)):
        return tuple(_freeze(item) for item in value)
    if isinstance(value, (set, frozenset)):
        return frozenset(_freeze(item) for item in value)
    if isinstance(value, (bytearray, memoryview)):
        return bytes(value)
    return value


def _thaw(value: Any) -> Any:
    if isinstance(value, Mapping):
        return {key: _thaw(item) for key, item in value.items()}
    if isinstance(value, tuple):
        return [_thaw(item) for item in value]
    if isinstance(value, frozenset):
        return {_thaw(item) for item in value}
    return value


@dataclass(frozen=True)
class RiffChunk:
    """One ordered RIFF chunk."""

    id: bytes
    payload: bytes

    def __post_init__(self) -> None:
        chunk_id = _as_bytes(self.id, "chunk id")
        if len(chunk_id) != 4:
            raise ValueError(f"chunk id must be exactly 4 bytes, got {len(chunk_id)}")
        object.__setattr__(self, "id", chunk_id)
        object.__setattr__(self, "payload", _as_bytes(self.payload, "chunk payload"))

    @property
    def fourcc(self) -> str:
        return self.id.decode("ascii", "replace")

    @classmethod
    def from_legacy(cls, value: Any) -> "RiffChunk":
        if isinstance(value, cls):
            return value
        if isinstance(value, Mapping):
            chunk_id = value["id"]
            if isinstance(chunk_id, str):
                chunk_id = chunk_id.encode("ascii")
            return cls(chunk_id, value["payload"])
        if isinstance(value, (tuple, list)) and len(value) == 2:
            chunk_id = value[0].encode("ascii") if isinstance(value[0], str) else value[0]
            return cls(chunk_id, value[1])
        raise TypeError("chunk must be RiffChunk, (id, payload), or a mapping")

    def to_legacy_tuple(self) -> tuple[bytes, bytes]:
        return self.id, self.payload


@dataclass(frozen=True)
class WemFmt:
    """Immutable typed view of an encoded Wwise ``fmt `` payload."""

    kind: str
    raw: bytes
    fields: Mapping[str, Any] = field(default_factory=dict)

    def __post_init__(self) -> None:
        if not isinstance(self.kind, str) or not self.kind:
            raise ValueError("fmt kind must be a non-empty string")
        if not isinstance(self.fields, Mapping):
            raise TypeError("fmt fields must be a mapping")
        object.__setattr__(self, "raw", _as_bytes(self.raw, "fmt raw"))
        object.__setattr__(self, "fields", _freeze(dict(self.fields)))

    def to_legacy_dict(self) -> dict[str, Any]:
        return _thaw(self.fields)


def _extra(value: Any) -> RiffChunk:
    return RiffChunk.from_legacy(value)


def _chunks_from_legacy(parts: Mapping[str, Any]) -> tuple[RiffChunk, ...]:
    if parts.get("raw") is not None:
        _endian, parsed = parse_chunks(_as_bytes(parts["raw"], "raw WEM"))
        return tuple(RiffChunk(c["id"].encode("ascii"), c["payload"]) for c in parsed)

    fmt_raw = _as_bytes(parts["fmt_raw"], "fmt_raw")
    data_raw = _as_bytes(parts["data_raw"], "data_raw")
    before = [_extra(item) for item in parts.get("extras_before_data", ())]
    outside = [_extra(item) for item in parts.get("extras_after_data", ())]
    ids = parts.get("chunk_ids")
    if not ids:
        return (RiffChunk(b"fmt ", fmt_raw), *before, RiffChunk(b"data", data_raw), *outside)

    result: list[RiffChunk] = []
    before_i = outside_i = 0
    seen_fmt = seen_data = False
    for value in ids:
        chunk_id = value.encode("ascii") if isinstance(value, str) else _as_bytes(value, "chunk id")
        if chunk_id == b"fmt ":
            if seen_fmt:
                raise ValueError("chunk_ids contains duplicate fmt")
            result.append(RiffChunk(chunk_id, fmt_raw))
            seen_fmt = True
        elif chunk_id == b"data":
            if seen_data:
                raise ValueError("chunk_ids contains duplicate data")
            result.append(RiffChunk(chunk_id, data_raw))
            seen_data = True
        elif seen_fmt and not seen_data:
            if before_i >= len(before) or before[before_i].id != chunk_id:
                raise ValueError("extras_before_data differs from chunk_ids")
            result.append(before[before_i])
            before_i += 1
        else:
            if outside_i >= len(outside) or outside[outside_i].id != chunk_id:
                raise ValueError("extras_after_data differs from chunk_ids")
            result.append(outside[outside_i])
            outside_i += 1
    if before_i != len(before) or outside_i != len(outside):
        raise ValueError("legacy extras contain chunks absent from chunk_ids")
    return tuple(result)


@dataclass(frozen=True)
class WemParts:
    """A complete WEM whose original RIFF chunk order remains representable."""

    endian: str
    kind: str
    fmt: ContainerMetadata | WemFmt
    chunks: tuple[RiffChunk, ...]
    setup_packet: bytes | None = None
    audio_packets: tuple[bytes, ...] = ()
    seek_table: bytes = b""
    path: str | None = None
    file: str | None = None
    raw: bytes | None = field(default=None, repr=False, compare=False)

    def __post_init__(self) -> None:
        if self.endian not in ("le", "be"):
            raise ValueError("endian must be 'le' or 'be'")
        if not isinstance(self.kind, str) or not self.kind:
            raise ValueError("WEM kind must be a non-empty string")
        if not isinstance(self.fmt, (ContainerMetadata, WemFmt)):
            raise TypeError("fmt must be ContainerMetadata or WemFmt")
        chunks = tuple(RiffChunk.from_legacy(chunk) for chunk in self.chunks)
        if sum(chunk.id == b"fmt " for chunk in chunks) != 1:
            raise ValueError("WEM parts require exactly one fmt chunk")
        if sum(chunk.id == b"data" for chunk in chunks) != 1:
            raise ValueError("WEM parts require exactly one data chunk")
        fmt_raw = next(chunk.payload for chunk in chunks if chunk.id == b"fmt ")
        if isinstance(self.fmt, WemFmt) and self.fmt.raw != fmt_raw:
            raise ValueError("fmt view raw bytes differ from the ordered fmt chunk")

        setup = None if self.setup_packet is None else _as_bytes(self.setup_packet, "setup packet")
        audio = tuple(_as_bytes(packet, "audio packet") for packet in self.audio_packets)
        seek = _as_bytes(self.seek_table, "seek table")
        if self.kind != "wwise_vorbis" and (setup is not None or audio or seek):
            raise ValueError("only Wwise Vorbis parts may contain packets or a seek table")
        raw = None if self.raw is None else _as_bytes(self.raw, "raw WEM")
        if raw is not None:
            raw_endian, parsed = parse_chunks(raw)
            raw_chunks = tuple(RiffChunk(c["id"].encode("ascii"), c["payload"]) for c in parsed)
            if raw_endian != self.endian or raw_chunks != chunks:
                raise ValueError("raw WEM differs from endian or ordered chunks")

        object.__setattr__(self, "chunks", chunks)
        object.__setattr__(self, "setup_packet", setup)
        object.__setattr__(self, "audio_packets", audio)
        object.__setattr__(self, "seek_table", seek)
        object.__setattr__(self, "path", None if self.path is None else str(self.path))
        object.__setattr__(self, "file", None if self.file is None else str(self.file))
        object.__setattr__(self, "raw", raw)

    @property
    def ordered_chunks(self) -> tuple[RiffChunk, ...]:
        return self.chunks

    @property
    def extra_chunks(self) -> tuple[RiffChunk, ...]:
        return tuple(c for c in self.chunks if c.id not in (b"fmt ", b"data"))

    @property
    def extras_before_data(self) -> tuple[RiffChunk, ...]:
        fmt_i = next(i for i, c in enumerate(self.chunks) if c.id == b"fmt ")
        data_i = next(i for i, c in enumerate(self.chunks) if c.id == b"data")
        if fmt_i >= data_i:
            return ()
        return tuple(c for c in self.chunks[fmt_i + 1 : data_i] if c.id not in (b"fmt ", b"data"))

    @property
    def extras_after_data(self) -> tuple[RiffChunk, ...]:
        fmt_i = next(i for i, c in enumerate(self.chunks) if c.id == b"fmt ")
        data_i = next(i for i, c in enumerate(self.chunks) if c.id == b"data")
        if fmt_i >= data_i:
            return self.extra_chunks
        return tuple((*self.chunks[:fmt_i], *self.chunks[data_i + 1 :]))

    @property
    def packets(self) -> tuple[bytes, ...]:
        return self.audio_packets if self.setup_packet is None else (self.setup_packet, *self.audio_packets)

    @property
    def fmt_raw(self) -> bytes:
        return next(c.payload for c in self.chunks if c.id == b"fmt ")

    @property
    def data_raw(self) -> bytes:
        return next(c.payload for c in self.chunks if c.id == b"data")

    def to_bytes(self) -> bytes:
        return self.raw if self.raw is not None else build_riff(
            [chunk.to_legacy_tuple() for chunk in self.chunks], endian=self.endian
        )

    @classmethod
    def from_legacy_dict(cls, parts: Mapping[str, Any]) -> "WemParts":
        if not isinstance(parts, Mapping):
            raise TypeError("legacy WEM parts must be a mapping")
        kind = str(parts.get("kind") or "unknown")
        chunks = _chunks_from_legacy(parts)
        fmt_raw = next((c.payload for c in chunks if c.id == b"fmt "), None)
        if fmt_raw is None:
            raise ValueError("legacy WEM parts are missing fmt")
        old_fmt = parts.get("fmt", {})
        if isinstance(old_fmt, ContainerMetadata):
            fmt: ContainerMetadata | WemFmt = old_fmt
        elif isinstance(old_fmt, Mapping):
            fmt = WemFmt(kind, fmt_raw, old_fmt)
        else:
            raise TypeError("legacy fmt must be a mapping or ContainerMetadata")
        packets = tuple(_as_bytes(p, "packet") for p in parts.get("packets", ()))
        setup = packets[0] if kind == "wwise_vorbis" and packets else None
        audio = packets[1:] if setup is not None else ()
        path = parts.get("path")
        file = parts.get("file")
        if file is None and path is not None:
            file = Path(str(path)).name
        return cls(
            endian=str(parts.get("endian", "le")), kind=kind, fmt=fmt, chunks=chunks,
            setup_packet=setup, audio_packets=audio, seek_table=parts.get("seek_table", b""),
            path=None if path is None else str(path), file=None if file is None else str(file),
            raw=parts.get("raw"),
        )

    from_legacy = from_legacy_dict

    def to_legacy_dict(self) -> dict[str, Any]:
        fmt_fields = self.fmt.to_legacy_dict() if isinstance(self.fmt, WemFmt) else self.fmt.to_fmt_dict(frame_count=self.fmt.dwTotalPCMFrames)
        result: dict[str, Any] = {
            "endian": self.endian, "raw": self.to_bytes(), "fmt_raw": self.fmt_raw,
            "data_raw": self.data_raw,
            "extras_before_data": [c.to_legacy_tuple() for c in self.extras_before_data],
            "extras_after_data": [c.to_legacy_tuple() for c in self.extras_after_data],
            "chunk_ids": [c.fourcc for c in self.chunks], "kind": self.kind, "fmt": fmt_fields,
        }
        if self.path is not None:
            result["path"] = self.path
        if self.file is not None:
            result["file"] = self.file
        if self.kind == "wwise_vorbis":
            result.update(seek_table=self.seek_table, packets=list(self.packets), sizes=[len(p) for p in self.packets])
        else:
            result["pcm_data"] = self.data_raw
        return result

    to_legacy = to_legacy_dict


__all__ = ["RiffChunk", "WemFmt", "WemParts"]
