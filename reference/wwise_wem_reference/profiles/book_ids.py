#!/usr/bin/env python3
"""Map Wwise setup 10-bit book IDs to installed static codebook tables.

The Wwise 2013 codebook registry assigns IDs contiguously:
  t97  → IDs 0 .. 96
  t219 → IDs 97 .. 315

The installed Wwise 2013 profiles use these two exported tables. Extend the
registry when another installed profile requires an additional table.

"""
from __future__ import annotations

from functools import lru_cache

from collections.abc import Mapping, Sequence

from wwise_wem.profiles.resources import ResourceRef

T97_COUNT = 97
T219_COUNT = 219

_BOOK_COUNTS = {"t97": T97_COUNT, "t219": T219_COUNT}


@lru_cache(maxsize=None)
def load_book_table(table: str, resource: ResourceRef) -> tuple[dict, ...]:
    """Load one checked installed table, shared by mapping and VQ runtime."""
    try:
        expected_count = _BOOK_COUNTS[str(table)]
    except KeyError as error:
        raise FileNotFoundError(f"missing decoded codebook table {table!r}") from error
    payload = resource.read_json()
    if (
        not isinstance(payload, list)
        or len(payload) != expected_count
        or any(not isinstance(row, dict) for row in payload)
    ):
        raise ValueError(
            f"decoded codebook table {table!r} must contain "
            f"{expected_count} object rows"
        )
    return tuple(payload)


def resolve_book_id(
    book_id: int,
    tables: Mapping[str, Sequence[dict]],
) -> dict:
    if book_id < 0:
        return {"error": "negative id", "book_id": book_id}
    if book_id < T97_COUNT:
        books = tables["t97"]
        b = books[book_id]
        return {
            "book_id": book_id,
            "table": "t97",
            "index": book_id,
            "dim": b["dim"],
            "entries": b["entries"],
            "maptype": b["maptype"],
        }
    j = book_id - T97_COUNT
    if j < T219_COUNT:
        books = tables["t219"]
        b = books[j]
        return {
            "book_id": book_id,
            "table": "t219",
            "index": j,
            "dim": b["dim"],
            "entries": b["entries"],
            "maptype": b["maptype"],
            "q_min": b.get("q_min"),
            "q_delta": b.get("q_delta"),
            "q_quant": b.get("q_quant"),
            "q_sequencep": b.get("q_sequencep"),
            "quantlist": b.get("quantlist"),
            "lengthlist": b.get("lengthlist"),
        }
    return {"error": "book id is outside the installed codebook tables", "book_id": book_id}
