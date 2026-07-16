"""Checked package-resource adapters for Wwise Vorbis codebooks."""

from __future__ import annotations

from collections.abc import Mapping, Sequence

from .book_ids import resolve_book_id
from ..vorbis.codebook import Codebook, codebook_from_static, static_from_entry_dict


def load_codebook(
    resolved: dict,
    tables: Mapping[str, Sequence[dict]],
) -> Codebook:
    """Build a runtime codebook from a resolved book descriptor."""
    if "error" in resolved:
        raise ValueError(f"resolve error: {resolved}")

    table = resolved.get("table")
    index = resolved.get("index")
    book_id = resolved.get("book_id")
    if resolved.get("lengthlist") is not None:
        static = static_from_entry_dict(resolved)
    else:
        if table is None or index is None:
            raise ValueError("resolved book needs table+index or lengthlist")
        static = static_from_entry_dict(tables[str(table)][int(index)])

    return codebook_from_static(
        static,
        book_id=book_id,
        table=str(table) if table else None,
        index=index,
    )


def load_setup_codebooks(
    book_ids: Sequence[int],
    tables: Mapping[str, Sequence[dict]],
) -> tuple[Codebook, ...]:
    """Resolve setup book identifiers and build their runtime codebooks."""
    return tuple(
        load_codebook(resolve_book_id(int(book_id), tables), tables)
        for book_id in book_ids
    )
