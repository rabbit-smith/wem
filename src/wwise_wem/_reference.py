"""Lazy access to the development-tree reference implementation.

The wheel ships only this facade; the pure-Python reference implementation
(``wwise_wem_reference``) lives in the development tree.  Every facade path
that needs reference code (the pure-Python encode pipeline) resolves it
through :func:`reference_module` so an
installed facade without the reference tree fails with one clear
``ImportError`` instead of a half-built run.
"""

from __future__ import annotations

from importlib import import_module
from types import ModuleType

_REFERENCE_MISSING_ERROR = (
    "the pure-Python reference implementation (wwise_wem_reference) is not "
    "importable in this environment; this copy of the package ships only "
    "the facade, so encoding requires the native kernel _wwise_wem_native "
    "(build it with `cd crates/wem-python && maturin develop`, or install a "
    "distribution that ships the native extension)"
)


def reference_module(name: str = "") -> ModuleType:
    """Import a module from the reference tree with a clear failure."""
    module_name = f"wwise_wem_reference.{name}" if name else "wwise_wem_reference"
    try:
        return import_module(module_name)
    except ImportError as error:
        raise ImportError(_REFERENCE_MISSING_ERROR) from error


__all__ = ["reference_module"]
