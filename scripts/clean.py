#!/usr/bin/env python3
"""Remove local build, test, and bytecode artifacts."""
from __future__ import annotations

import shutil
from pathlib import Path


ROOT = Path(__file__).resolve().parents[1]
DIRECTORIES = {"__pycache__", ".pytest_cache", ".mypy_cache", ".ruff_cache", "build", "dist"}
FILES = {".coverage", "coverage.xml", "output.wem"}


def main() -> None:
    for path in sorted(ROOT.rglob("*"), reverse=True):
        if path.is_dir() and (path.name in DIRECTORIES or path.name.endswith(".egg-info")):
            shutil.rmtree(path, ignore_errors=True)
        elif path.is_file() and (path.name in FILES or path.suffix in {".pyc", ".pyo"}):
            path.unlink(missing_ok=True)


if __name__ == "__main__":
    main()
