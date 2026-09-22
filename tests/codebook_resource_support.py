"""Installed codebook tables for focused tests."""

from __future__ import annotations

from functools import lru_cache
from types import MappingProxyType

from tests.analysis_resource_support import installed_profile
from wwise_wem_reference.profiles.book_ids import load_book_table


@lru_cache(maxsize=1)
def installed_codebook_tables():
    profile = installed_profile(6, 44100)
    return MappingProxyType(
        {
            "t97": load_book_table("t97", profile),
            "t219": load_book_table("t219", profile),
        }
    )
