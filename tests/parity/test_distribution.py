"""What ships, and what may not appear in it.

The distribution cases pin the exported names and the module inventory
against the allowlist, and that the extension's name stays in step across the
manifest, the Cargo library name and the pyproject entries. The cleanliness
cases pin that no provenance marker reaches package text, a path or a JSON
string. Both read the same package, so they share a module; each class keeps
its own test names and failure messages.
"""

from __future__ import annotations

import json
import re
import unittest
import wwise_wem

from pathlib import Path
from typing import Iterator

# --------------------------------------------------------------------------
# merged from tests/parity/test_distribution.py
# --------------------------------------------------------------------------

# Exact public and wheel-source distribution boundaries.

ROOT = Path(__file__).resolve().parents[2]
PACKAGE = ROOT / "src" / "wwise_wem"
ALLOWLIST = Path(__file__).with_name("distribution_allowlist.json")


def _allowlist() -> dict[str, object]:
    return json.loads(ALLOWLIST.read_text(encoding="utf-8"))


def _wheel_name(path: Path) -> str:
    return "wwise_wem/" + path.relative_to(PACKAGE).as_posix()


class DistributionTests(unittest.TestCase):
    def test_root_all_is_the_exact_supported_surface(self) -> None:
        expected = tuple(_allowlist()["root_exports"])
        self.assertEqual(tuple(wwise_wem.__all__), expected)
        self.assertEqual(
            tuple(name for name in expected if name not in dir(wwise_wem)),
            (),
        )

    def test_source_modules_and_resources_match_distribution_allowlist(self) -> None:
        allowlist = _allowlist()
        self.assertEqual(
            allowlist["schema"],
            "wwise-wem.distribution-allowlist.v1",
        )
        modules = sorted(_wheel_name(path) for path in PACKAGE.rglob("*.py"))
        resources = sorted(
            _wheel_name(path)
            for path in (PACKAGE / "data").rglob("*")
            if path.is_file()
        )
        self.assertEqual(modules, allowlist["modules"])
        self.assertEqual(resources, allowlist["resources"])

    def test_allowlist_excludes_deleted_and_research_modules(self) -> None:
        allowlist = _allowlist()
        paths = tuple(allowlist["modules"]) + tuple(allowlist["resources"])
        deleted = {
            "bitreader.py",
            "oggpack.py",
            "pack_wem.py",
            "parse_setup.py",
            "parse_wem.py",
            "scheduler_windows.py",
            "static_codebook.py",
        }
        self.assertFalse(
            {Path(path).name for path in paths} & deleted,
        )
        markers = tuple(str(value).lower() for value in allowlist["forbidden_path_markers"])
        violations = [
            path
            for path in paths
            if any(marker in path.lower() for marker in markers)
        ]
        self.assertEqual(violations, [])

        content_markers = tuple(
            str(value).encode("ascii").lower()
            for value in allowlist["forbidden_content_markers"]
        )
        content_violations = []
        for name in paths:
            if not name.endswith((".py", ".json")):
                continue
            payload = (ROOT / "src" / name).read_bytes().lower()
            if any(marker in payload for marker in content_markers):
                content_violations.append(name)
        self.assertEqual(content_violations, [])

    def test_native_extension_stays_synced_across_surfaces(self) -> None:
        """The in-package extension must agree across its name surfaces.

        The wheel ships the abi3 extension as `wwise_wem/_core.abi3.so`
        (or `.pyd`).  Drift between the allowlist, the Cargo lib name,
        and the maturin module-name settings would silently break either
        the import or the wheel layout, so the spellings are pinned here
        in one place.  The module name is an install-time fact owned by
        these files only: the facade imports `wwise_wem._core` directly
        and keeps no second copy of the name.
        """
        allowlist = _allowlist()
        native = allowlist["native_extensions"]
        self.assertEqual(native, ["wwise_wem._core"])

        # Cargo lib name: the cdylib the wheel renames into place.
        cargo = (ROOT / "crates" / "wem-python" / "Cargo.toml").read_text(
            encoding="utf-8"
        )
        lib_name = _section_value(cargo, "lib", "name")
        self.assertEqual(lib_name, "_core")
        self.assertIn("cdylib", _section_value(cargo, "lib", "crate-type"))

        # Root pyproject: the maturin module-name that places the .so.
        root = ROOT / "pyproject.toml"
        pyproject = root.read_text(encoding="utf-8")
        self.assertIn('build-backend = "maturin"', pyproject)
        module_name = _section_value(pyproject, "tool.maturin", "module-name")
        self.assertEqual(module_name, native[0])

        # Binding-crate pyproject: documents the same module placement.
        binding_pyproject = (
            ROOT / "crates" / "wem-python" / "pyproject.toml"
        ).read_text(encoding="utf-8")
        binding_name = _section_value(
            binding_pyproject, "tool.maturin", "module-name"
        )
        self.assertEqual(binding_name, native[0])


def _section_value(text: str, section: str, key: str) -> str:
    """Read one `key = "value"` line under a `[section]` (TOML-free parse)."""
    marker = f"[{section}]"
    start = text.index(marker)
    end = text.find("\n[", start + len(marker))
    block = text[start : end if end != -1 else len(text)]
    for line in block.splitlines():
        if line.strip().startswith(f"{key} ="):
            return line.split("=", 1)[1].strip().strip('"')
    raise AssertionError(f"{key} not found under {marker}")


# --------------------------------------------------------------------------
# merged from tests/parity/test_project_cleanliness.py
# --------------------------------------------------------------------------

# Keep reverse-engineering provenance out of the distributable package.

ROOT = Path(__file__).resolve().parents[2]
PACKAGE = ROOT / "src" / "wwise_wem"
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
        surfaces = [ROOT / "pyproject.toml", ROOT / "README.md"]
        # Every document under docs/ is reached by the recursive scan below, so
        # naming one explicitly here only goes stale when it moves.
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
