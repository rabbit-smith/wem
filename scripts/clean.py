#!/usr/bin/env python3
"""Remove local build, test, and bytecode artifacts."""
from __future__ import annotations

import shutil
from pathlib import Path


ROOT = Path(__file__).resolve().parents[1]
DIRECTORIES = {
    "__pycache__",
    ".pytest_cache",
    ".mypy_cache",
    ".ruff_cache",
    "build",
    "dist",
}
FILES = {".coverage", "coverage.xml", "output.wem"}
# Virtual environments and git metadata are never touched.
SKIP_DIRS = {".git", ".venv", "venv"}
# Rust build tree (cargo / maturin develop artifacts).
CARGO_TARGET = ROOT / "crates" / "target"


def _is_native_artifact(path: Path) -> bool:
    # Dev-tree extension artifacts: maturin develop drops
    # `wwise_wem/_native.abi3.so` next to the sources.
    return path.name.endswith(".abi3.so") or (
        path.name.startswith("_native")
        and path.suffix in {".so", ".dylib", ".pyd"}
    )


def _iter_paths(root: Path):
    for path in sorted(root.rglob("*"), reverse=True):
        if any(part in SKIP_DIRS for part in path.parts):
            continue
        yield path


def main() -> None:
    for path in _iter_paths(ROOT):
        if path.is_dir() and path.resolve() == CARGO_TARGET.resolve():
            shutil.rmtree(path, ignore_errors=True)
        elif path.is_dir() and (
            path.name in DIRECTORIES or path.name.endswith(".egg-info")
        ):
            shutil.rmtree(path, ignore_errors=True)
        elif path.is_file() and (
            path.name in FILES
            or path.suffix in {".pyc", ".pyo"}
            or _is_native_artifact(path)
        ):
            path.unlink(missing_ok=True)
    print(
        "note: .venv/ (installed packages, native kernel) is intentionally "
        "untouched; remove it manually with `rm -rf .venv` if desired"
    )


if __name__ == "__main__":
    main()
