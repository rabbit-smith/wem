"""Installed manifest-addressed codebook tables for focused tests."""

from __future__ import annotations

from functools import lru_cache
from types import MappingProxyType

from wwise_wem_reference.profiles.book_ids import load_book_table
from wwise_wem.profiles.bundle import load_profile_bundle


@lru_cache(maxsize=1)
def installed_codebook_tables():
    manifest = load_profile_bundle(verify_all=False).runtime_manifest
    return MappingProxyType(
        {
            "t97": load_book_table(
                "t97", manifest.resource("vorbis.codebooks.t97")
            ),
            "t219": load_book_table(
                "t219", manifest.resource("vorbis.codebooks.t219")
            ),
        }
    )
