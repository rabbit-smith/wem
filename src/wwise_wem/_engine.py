"""Engine selection between the native kernel and the reference implementation.

The public encode facade prefers the Rust kernel (module
``_wwise_wem_native``, built from ``crates/wem-python``) when it is
importable, and falls back to the pure-Python reference implementation in
the development-tree ``wwise_wem_reference`` package otherwise.  Both
engines are contractually byte-identical; the reference implementation
remains the reference oracle.

The ``WWISE_WEM_ENGINE`` environment variable pins the choice:

* ``auto`` (default): native when importable, pure Python otherwise.
* ``native``: always native; a missing extension raises
  :class:`ImportError` instead of downgrading silently.
* ``python``: always the pure-Python implementation.

Only byte-producing encode paths consult this module; profile metadata,
registry, and container-reading helpers stay pure Python regardless.
"""

from __future__ import annotations

import importlib
import os
from dataclasses import dataclass
from types import ModuleType

#: Environment variable that pins the facade engine choice.
ENGINE_ENV_VAR = "WWISE_WEM_ENGINE"

#: Native extension module name produced by ``crates/wem-python``.
NATIVE_MODULE_NAME = "_wwise_wem_native"

_AUTO = "auto"
_NATIVE = "native"
_PYTHON = "python"
_CHOICES = (_AUTO, _NATIVE, _PYTHON)

_native_checked = False
_native_module: ModuleType | None = None


def native_module() -> ModuleType | None:
    """Import (once) and return the native kernel module, or ``None``."""
    global _native_checked, _native_module
    if not _native_checked:
        try:
            _native_module = importlib.import_module(NATIVE_MODULE_NAME)
        except ImportError:
            _native_module = None
        _native_checked = True
    return _native_module


def native_available() -> bool:
    """Whether the native kernel extension is importable."""
    return native_module() is not None


def requested_engine() -> str:
    """Return the pinned engine choice (default ``auto``)."""
    raw = os.environ.get(ENGINE_ENV_VAR, _AUTO).strip().lower()
    if raw not in _CHOICES:
        raise ValueError(
            f"{ENGINE_ENV_VAR} must be one of 'auto', 'native', 'python'; "
            f"got {raw!r}"
        )
    return raw


def active_engine() -> str:
    """Resolve the engine to run now.

    Raises ``ImportError`` when ``native`` is pinned but the extension is
    not importable: an explicit native request never downgrades silently.
    """
    requested = requested_engine()
    if requested == _NATIVE:
        if native_module() is None:
            raise ImportError(
                f"{ENGINE_ENV_VAR}=native requires the native extension "
                f"{NATIVE_MODULE_NAME!r}, which is not importable; build it "
                f"with `cd crates/wem-python && maturin develop` (or unset "
                f"{ENGINE_ENV_VAR})"
            )
        return _NATIVE
    if requested == _PYTHON:
        return _PYTHON
    return _NATIVE if native_module() is not None else _PYTHON


@dataclass(frozen=True)
class EngineStatus:
    """Snapshot of the facade engine decision, for diagnostics."""

    requested: str
    native_available: bool
    active: str
    reason: str


def engine_status() -> EngineStatus:
    """Report the current engine decision without raising for missing native."""
    requested = requested_engine()
    native = native_module() is not None
    if requested == _NATIVE:
        if native:
            return EngineStatus(requested, native, _NATIVE, "native pinned and importable")
        return EngineStatus(
            requested,
            native,
            _NATIVE,
            "native pinned but extension missing; encode calls raise ImportError",
        )
    if requested == _PYTHON:
        return EngineStatus(requested, native, _PYTHON, "python pinned")
    if native:
        return EngineStatus(requested, native, _NATIVE, "auto: native importable")
    return EngineStatus(requested, native, _PYTHON, "auto: native unavailable")


__all__ = [
    "ENGINE_ENV_VAR",
    "EngineStatus",
    "NATIVE_MODULE_NAME",
    "active_engine",
    "engine_status",
    "native_available",
    "native_module",
    "requested_engine",
]
