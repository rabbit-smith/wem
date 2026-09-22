"""Bit-exact Wwise Vorbis WEM encoder, and a decoder for the containers it writes."""

from typing import Any

from .api import decode, encode
from .application.models import EncodeResult, EncodeStats
from .model import PcmBuffer, RawPcm, WwiseWemError


__all__ = [
    "EncodeResult",
    "EncodeStats",
    "PcmBuffer",
    "RawPcm",
    "WwiseProfile",
    "WwiseVersion",
    "WwiseWemError",
    "decode",
    "encode",
]

# The structured profile selector lives in the native extension. It is
# resolved on first use (PEP 562) rather than at import time, so importing the
# package stays a pure-Python operation: the facade remains importable from
# the distribution zip, and the extension stays what it is everywhere else — a
# required asset of the byte-producing path, surfacing as the ordinary
# ImportError when it is absent.
_SELECTOR_TYPES = frozenset({"WwiseProfile", "WwiseVersion"})


def __getattr__(name: str) -> Any:
    """Resolve one lazily re-exported extension type on first use."""
    if name in _SELECTOR_TYPES:
        from . import _core

        return getattr(_core, name)
    raise AttributeError(f"module {__name__!r} has no attribute {name!r}")


def __dir__() -> list[str]:
    return sorted(set(globals()) | _SELECTOR_TYPES)
