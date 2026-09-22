#!/usr/bin/env python3
"""Map Wwise setup 10-bit book IDs to installed static codebook tables.

The Wwise 2013 codebook registry assigns IDs contiguously:
  t97  → IDs 0 .. 96
  t219 → IDs 97 .. 315
  t282 → IDs 316 .. 597

The installed Wwise 2013 profiles use these exported tables. Extend the
registry when another installed profile requires an additional table.

"""
from __future__ import annotations

from collections.abc import Mapping, Sequence
from functools import lru_cache

from .artifact import CompiledProfile

T97_COUNT = 97
T219_COUNT = 219
T282_COUNT = 282

_BOOK_COUNTS = {"t97": T97_COUNT, "t219": T219_COUNT, "t282": T282_COUNT}


@lru_cache(maxsize=None)
def load_book_table(table: str, profile: CompiledProfile) -> tuple[dict, ...]:
    """Rebuild one installed codebook table from the compiled carrier.

    The carrier records the *decoded* rows: the optional fields (``i``,
    ``quantvals``, ``lengthlist``, ``quantlist``) travel with an explicit
    presence flag, so a row that recorded no value does not grow one here —
    ``row.get(...)`` keeps meaning what it meant against the recorded document.
    """
    if not isinstance(profile, CompiledProfile):
        raise TypeError("codebook tables require a CompiledProfile")
    name = str(table)
    try:
        expected_count = _BOOK_COUNTS[name]
    except KeyError as error:
        raise FileNotFoundError(f"missing decoded codebook table {table!r}") from error
    prefix = f"codebook.{name}"
    dim = [int(value) for value in profile.table(f"{prefix}.dim")]
    if len(dim) != expected_count:
        raise ValueError(
            f"decoded codebook table {table!r} must contain "
            f"{expected_count} object rows"
        )
    entries = [int(value) for value in profile.table(f"{prefix}.entries")]
    maptype = [int(value) for value in profile.table(f"{prefix}.maptype")]
    q_min = [int(value) for value in profile.table(f"{prefix}.q_min")]
    q_delta = [int(value) for value in profile.table(f"{prefix}.q_delta")]
    q_quant = [int(value) for value in profile.table(f"{prefix}.q_quant")]
    q_sequencep = [int(value) for value in profile.table(f"{prefix}.q_sequencep")]
    i_present = profile.table(f"{prefix}.i_present")
    i_values = profile.table(f"{prefix}.i")
    quantvals_present = profile.table(f"{prefix}.quantvals_present")
    quantvals_values = profile.table(f"{prefix}.quantvals")
    lengthlist, lengthlist_offsets, lengthlist_present = _optional_column(profile, prefix, "lengthlist")
    quantlist, quantlist_offsets, quantlist_present = _optional_column(profile, prefix, "quantlist")

    rows: list[dict] = []
    for index in range(expected_count):
        row: dict = {
            "dim": dim[index],
            "entries": entries[index],
            "maptype": maptype[index],
            "q_min": q_min[index],
            "q_delta": q_delta[index],
            "q_quant": q_quant[index],
            "q_sequencep": q_sequencep[index],
        }
        if i_present[index]:
            row["i"] = int(i_values[index])
        if quantvals_present[index]:
            row["quantvals"] = int(quantvals_values[index])
        if lengthlist_present[index]:
            row["lengthlist"] = list(
                lengthlist[lengthlist_offsets[index] : lengthlist_offsets[index + 1]]
            )
        if quantlist_present[index]:
            row["quantlist"] = list(
                quantlist[quantlist_offsets[index] : quantlist_offsets[index + 1]]
            )
        rows.append(row)
    return tuple(rows)


def _optional_column(
    profile: CompiledProfile,
    prefix: str,
    column: str,
) -> tuple[list[int], list[int], list[int]]:
    """The flattened ``(values, offsets, present)`` triple of one optional column."""
    values = [int(value) for value in profile.table(f"{prefix}.{column}")]
    offsets = [int(value) for value in profile.table(f"{prefix}.{column}_offsets")]
    present = [int(value) for value in profile.table(f"{prefix}.{column}_present")]
    return values, offsets, present


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
        t219_books = tables.get("t219")
        if t219_books is None:
            return {"error": "book id is outside the installed codebook tables", "book_id": book_id}
        b = t219_books[j]
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
    k = book_id - T97_COUNT - T219_COUNT
    if k < T282_COUNT:
        t282_books = tables.get("t282")
        if t282_books is None:
            return {"error": "book id is outside the installed codebook tables", "book_id": book_id}
        b = t282_books[k]
        return {
            "book_id": book_id,
            "table": "t282",
            "index": k,
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
