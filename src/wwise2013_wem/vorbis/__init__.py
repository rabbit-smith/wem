"""Pure Vorbis syntax, bit I/O, and codebook primitives."""

from __future__ import annotations

from typing import Any

from .bitio import BitReader, OggPack


_CODEBOOK_EXPORTS = {
    "Codebook",
    "StaticCodebook",
    "book_maptype1_quantvals",
    "codebook_from_static",
    "float32_unpack",
    "ieee_float32_bits",
    "ilog",
    "make_codewords",
    "static_from_entry_dict",
}

_SETUP_EXPORTS = {
    "pack_floor1",
    "pack_mapping0",
    "pack_mode",
    "pack_residue",
    "pack_setup",
    "parse_floor1",
    "parse_mapping0",
    "parse_mode",
    "parse_residue",
    "parse_setup",
}


def __getattr__(name: str) -> Any:
    if name in _CODEBOOK_EXPORTS:
        from . import codebook

        return getattr(codebook, name)
    if name in _SETUP_EXPORTS:
        from . import setup

        return getattr(setup, name)
    raise AttributeError(f"module {__name__!r} has no attribute {name!r}")


def __dir__() -> list[str]:
    return sorted(set(globals()) | _CODEBOOK_EXPORTS | _SETUP_EXPORTS)


__all__ = [
    "BitReader",
    "OggPack",
    "Codebook",
    "StaticCodebook",
    "book_maptype1_quantvals",
    "codebook_from_static",
    "float32_unpack",
    "ieee_float32_bits",
    "ilog",
    "make_codewords",
    "static_from_entry_dict",
    "pack_floor1",
    "pack_mapping0",
    "pack_mode",
    "pack_residue",
    "pack_setup",
    "parse_floor1",
    "parse_mapping0",
    "parse_mode",
    "parse_residue",
    "parse_setup",
]
