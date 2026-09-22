"""What ships, and what may not appear in it.

The distribution cases pin the exported names and the module inventory
against the allowlist, and that the extension's name stays in step across the
manifest, the Cargo library name and the pyproject entries. The cleanliness
cases pin that no provenance marker reaches any surface a reader of the
published tree can see — not only the distributable package. Both read the
same package, so they share a module; each class keeps its own test names and
failure messages.
"""

from __future__ import annotations

import json
import re
import subprocess
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

# Keep development-process provenance out of every published surface.

ROOT = Path(__file__).resolve().parents[2]
PACKAGE = ROOT / "src" / "wwise_wem"
TEXT_SUFFIXES = {".py", ".json"}

# These expressions target provenance markers, not ordinary hexadecimal data.
# RIFF constants, packed float words, and SHA-256 digests therefore remain valid.
# Assembled from adjacent literals so that this module does not contain, as a
# literal, any string its own patterns match. The module is published: a pattern
# that doubles as an example of what it forbids defeats itself.
ADDRESS_LABEL = "R" "VA"
MODULE_LABEL = "D" "LL"
IMAGE_LABEL = "Conversion" "Plugin"
SECTION_LABEL = "r" "data"
POSIX_HOME = "/Users" "/"

PROVENANCE_PATTERNS = {
    "disassembler symbol name": re.compile(
        r"(?i)\b(?:sub|loc|off|byte|word|dword|qword|unk|stru|nullsub|asc|flt|dbl)"
        r"_[0-9a-f]{3,}\b"
    ),
    "binary address label": re.compile(r"(?i)\b" + ADDRESS_LABEL + r"\b"),
    "binary module label": re.compile(r"(?i)\b" + MODULE_LABEL + r"\b"),
    # `0x10203040` is the synthetic frame count the container suites use as
    # test data, not an image address, so it is excluded here.
    "image address": re.compile(r"(?i)\b0x10(?!203040)[0-9a-f]{6}\b"),
    "virtual address": re.compile(r"(?i)\bVA 0x[0-9a-f]+"),
    "binary image label": re.compile(
        r"(?i)" + IMAGE_LABEL + r"|\b" + SECTION_LABEL + r"\b"
    ),
    "local absolute path": re.compile(
        r"(?i)(?:^|[\s\"'])(?:" + POSIX_HOME + r"|/home/|[a-z]:\\\\users\\\\)"
    ),
    "internal lane label": re.compile(r"\bLane-[A-Z]\b"),
    # Escaped, so that naming the pattern here does not itself disclose the
    # internal report reference it looks for.
    "internal report reference": re.compile("\u00a7\u673a\u5236|\breport \u00a7"),
    "temporary path": re.compile(r"(?i)(?:^|[\s\"'])/tmp(?:/|\b)"),
    "tool workspace path": re.compile(r"(?i)(?:^|[\s\"'])tools[/\\]"),
}

# Markers a development document may legitimately discuss — the untracked
# development corpus and the round labels of the work that produced a value —
# but which a shipped or compiled surface may only name as a live reference.
CODE_SURFACE_PATTERNS = {
    # `corpus/profiles/` is the documented untracked development-material tree
    # the profile generators read, so naming it is a live reference. Any other
    # `corpus/<...>` subpath points at a development artifact no reader can
    # obtain, and the bare word is allowed so a surface can state that it does
    # not read the tree at all.
    "development corpus path": re.compile(r"(?i)\bcorpus/(?!profiles\b)[a-z0-9]"),
    "development round label": re.compile(r"\bR-\d+[a-z]?\b"),
    "development script name": re.compile(r"\bgen_r\d+[a-z0-9_]*\.py\b"),
}

# Surfaces whose text is compiled into a shipped artifact or read from the
# published tree: the Rust kernel (its comments and strings end up in the
# extension), the Python reference the suites drive, the tooling and the
# suites themselves.
CODE_SURFACE_PREFIXES = (
    ("crates/", ".rs"),
    ("reference/", ".py"),
    ("reference/", ".json"),
    ("scripts/", ".py"),
    ("tests/", ".py"),
    ("tests/", ".json"),
)


def _package_files() -> Iterator[Path]:
    for path in sorted(PACKAGE.rglob("*")):
        if path.is_file() and "__pycache__" not in path.parts:
            yield path


def _tracked_files(*suffixes: str) -> Iterator[Path]:
    """Tracked files ending in one of `suffixes`, sorted by path.

    The tracked set is the published surface. A recursive glob is not: it also
    reads build output such as `crates/target/**/out/embedded_profiles.rs`,
    which is ignored and never published.
    """
    listing = subprocess.run(
        ["git", "-C", str(ROOT), "ls-files", "-z"],
        check=True,
        capture_output=True,
    ).stdout.decode("utf-8")
    for name in sorted(listing.split("\0")):
        if name and name.endswith(suffixes):
            yield ROOT / name


def _code_surface_files() -> Iterator[Path]:
    """Every tracked code surface, excluding this module.

    This module is excluded because it defines the patterns; scanning it would
    only ever match its own definitions.
    """
    this_file = Path(__file__).resolve()
    for path in _tracked_files(*{suffix for _, suffix in CODE_SURFACE_PREFIXES}):
        name = path.relative_to(ROOT).as_posix()
        if not any(
            name.startswith(prefix) and name.endswith(suffix)
            for prefix, suffix in CODE_SURFACE_PREFIXES
        ):
            continue
        if path.resolve() == this_file:
            continue
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
        # Every document published with the tree — the root documents, the
        # subtree guides, the documentation set — plus the CI configuration.
        surfaces = [*_tracked_files(".md", ".yml"), ROOT / "pyproject.toml"]
        failures: list[str] = []
        for path in surfaces:
            for label, marker in _violations(path.read_text(encoding="utf-8")):
                failures.append(
                    f"{path.relative_to(ROOT)}: {label}: {marker!r}"
                )
        self.assertEqual(failures, [])

    def test_code_surfaces_have_no_provenance_markers(self):
        # The kernel, the reference implementation, the tooling and the suites:
        # surfaces whose text is compiled into a shipped artifact or published
        # with the tree. They are held to the development-corpus patterns as
        # well, because a shipped surface may only point at a live reference.
        patterns = (*PROVENANCE_PATTERNS.items(), *CODE_SURFACE_PATTERNS.items())
        failures: list[str] = []
        for path in _code_surface_files():
            relative = path.relative_to(ROOT).as_posix()
            text = path.read_text(encoding="utf-8")
            for label, pattern in patterns:
                for match in pattern.finditer(text):
                    failures.append(f"{relative}: {label}: {match.group(0)!r}")
        self.assertEqual(failures, [])

    def test_binary_constants_and_digests_are_not_address_markers(self):
        representative_format_data = (
            "RIFF = 0x52494646; float_word = 0x3f800000; "
            "sha256 = 17851d26c6210b85e498ae0452d2562d7b9e2c3e9e795c459656b9c9d8d35247"
        )
        self.assertEqual(_violations(representative_format_data), [])

    def test_synthetic_frame_count_is_not_an_address_marker(self):
        # `dwTotalPCMFrames` carries a synthetic frame count in the container
        # suites; the image-address pattern must not read it as a location.
        self.assertEqual(_violations('"dwTotalPCMFrames": 0x10203040,'), [])


if __name__ == "__main__":
    unittest.main()
