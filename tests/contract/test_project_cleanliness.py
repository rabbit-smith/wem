"""Keep reverse-engineering provenance out of the distributable package."""

from __future__ import annotations

import json
import re
import unittest
from pathlib import Path
from typing import Iterator


ROOT = Path(__file__).resolve().parents[2]
PACKAGE = ROOT / "src" / "wwise2013_wem"
TEXT_SUFFIXES = {".py", ".json"}

# These expressions target provenance markers, not ordinary hexadecimal data.
# RIFF constants, packed float words, and SHA-256 digests therefore remain valid.
PROVENANCE_PATTERNS = {
    "disassembler function name": re.compile(r"(?i)\bsub_[0-9a-f]{4,}\b"),
    "binary address label": re.compile(r"(?i)\bRVA\b"),
    "binary module label": re.compile(r"(?i)\bDLL\b"),
    "known reverse address": re.compile(
        r"(?i)(?<![0-9a-f])(?:19040|18aa0|183d0|1d990|cbb0|c5b0)"
        r"(?![0-9a-f])"
    ),
    "backticked address": re.compile(r"(?i)`{1,2}[0-9a-f]{5,8}`{1,2}"),
    "image address": re.compile(r"(?i)\b0x10[0-9a-f]{6}\b"),
    "instrumentation marker": re.compile(r"(?i)\b(?:capture|the probe|hook)\b"),
    "research marker": re.compile(
        r"(?i)\b(?:ctf|experiment|experimental|prototype|research)\b"
    ),
    "temporary path": re.compile(r"(?i)(?:^|[\s\"'])/tmp(?:/|\b)"),
    "tool workspace path": re.compile(r"(?i)(?:^|[\s\"'])tools[/\\]"),
}


def _package_files() -> Iterator[Path]:
    for path in sorted(PACKAGE.rglob("*")):
        if path.is_file() and "__pycache__" not in path.parts:
            yield path


def _json_strings(value: object, location: str = "$") -> Iterator[tuple[str, str]]:
    if isinstance(value, dict):
        for key, child in value.items():
            key_location = f"{location}.{key}"
            yield key_location, str(key)
            yield from _json_strings(child, key_location)
    elif isinstance(value, list):
        for index, child in enumerate(value):
            yield from _json_strings(child, f"{location}[{index}]")
    elif isinstance(value, str):
        yield location, value


def _violations(surface: str) -> list[tuple[str, str]]:
    return [
        (label, match.group(0))
        for label, pattern in PROVENANCE_PATTERNS.items()
        for match in pattern.finditer(surface)
    ]


class ProjectCleanlinessTests(unittest.TestCase):
    def test_package_paths_have_no_provenance_markers(self):
        failures: list[str] = []
        for path in _package_files():
            relative = path.relative_to(PACKAGE).as_posix()
            for label, marker in _violations(relative):
                failures.append(f"{relative}: {label}: {marker!r}")
        self.assertEqual(failures, [])

    def test_python_and_json_strings_have_no_provenance_markers(self):
        failures: list[str] = []
        for path in _package_files():
            if path.suffix not in TEXT_SUFFIXES:
                continue
            relative = path.relative_to(PACKAGE).as_posix()
            if path.suffix == ".json":
                value = json.loads(path.read_text(encoding="utf-8"))
                surfaces = _json_strings(value)
            else:
                surfaces = (
                    (f"line {number}", line)
                    for number, line in enumerate(
                        path.read_text(encoding="utf-8").splitlines(), start=1
                    )
                )
            for location, surface in surfaces:
                for label, marker in _violations(surface):
                    failures.append(
                        f"{relative}:{location}: {label}: {marker!r}"
                    )
        self.assertEqual(failures, [])

    def test_formal_project_surfaces_have_no_provenance_markers(self):
        surfaces = [ROOT / "pyproject.toml", ROOT / "README.md", ROOT / "docs" / "domain-model.md"]
        surfaces.extend(sorted((ROOT / "docs").rglob("*.md")))
        surfaces.extend(sorted((ROOT / ".github").rglob("*.yml")))
        failures: list[str] = []
        for path in surfaces:
            for label, marker in _violations(path.read_text(encoding="utf-8")):
                failures.append(
                    f"{path.relative_to(ROOT)}: {label}: {marker!r}"
                )
        self.assertEqual(failures, [])

    def test_binary_constants_and_digests_are_not_address_markers(self):
        representative_format_data = (
            "RIFF = 0x52494646; float_word = 0x3f800000; "
            "sha256 = 17851d26c6210b85e498ae0452d2562d7b9e2c3e9e795c459656b9c9d8d35247"
        )
        self.assertEqual(_violations(representative_format_data), [])


if __name__ == "__main__":
    unittest.main()
