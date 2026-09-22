"""What ships, and what may not appear in it.

The distribution cases pin the exported names and the module inventory
against the allowlist, and that the extension's name stays in step across the
manifest, the Cargo library name and the pyproject entries. The cleanliness
cases pin that no provenance marker reaches any surface a reader of the
published tree can see — not only the distributable package. Both read the
same package, so they share a module; each class keeps its own test names and
failure messages.

The pattern sets and the scan itself live in `scripts/check_provenance.py`,
which runs on a fresh checkout with no compiled extension. The cases here
consume them rather than owning them, so this audit and the pre-commit hook
cannot disagree about what a published surface is.
"""

from __future__ import annotations

import json
import unittest
import wwise_wem

from pathlib import Path

import scripts.check_provenance as provenance

# --------------------------------------------------------------------------
# merged from tests/parity/test_distribution.py
# --------------------------------------------------------------------------

# Exact public and wheel-source distribution boundaries.

ROOT = provenance.ROOT
PACKAGE = provenance.PACKAGE
ALLOWLIST = Path(__file__).with_name("distribution_allowlist.json")
TEXT_SUFFIXES = provenance.TEXT_SUFFIXES


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


class ProjectCleanlinessTests(unittest.TestCase):
    def test_standalone_gate_finds_no_violations_in_the_tracked_tree(self):
        # The gate the pre-commit hook runs, over the same tree: the package,
        # the code surfaces, the documents, the declared root files, the file
        # names themselves, and the guard that refuses an undeclared root file.
        self.assertEqual(
            [violation.report() for violation in provenance.audit_violations()],
            [],
        )

    def test_package_paths_have_no_provenance_markers(self):
        failures: list[str] = []
        for path in provenance.package_files():
            relative = path.relative_to(PACKAGE).as_posix()
            for label, marker in provenance.violations(relative):
                failures.append(f"{relative}: {label}: {marker!r}")
        self.assertEqual(failures, [])

    def test_python_and_json_strings_have_no_provenance_markers(self):
        failures: list[str] = []
        for path in provenance.package_files():
            if path.suffix not in TEXT_SUFFIXES:
                continue
            relative = path.relative_to(PACKAGE).as_posix()
            if path.suffix == ".json":
                value = json.loads(path.read_text(encoding="utf-8"))
                surfaces = provenance.json_strings(value)
            else:
                surfaces = (
                    (f"line {number}", line)
                    for number, line in enumerate(
                        path.read_text(encoding="utf-8").splitlines(), start=1
                    )
                )
            for location, surface in surfaces:
                for label, marker in provenance.violations(surface):
                    failures.append(
                        f"{relative}:{location}: {label}: {marker!r}"
                    )
        self.assertEqual(failures, [])

    def test_formal_project_surfaces_have_no_provenance_markers(self):
        # Every document published with the tree — the root documents, the
        # subtree guides, the documentation set — plus the root text files no
        # suffix rule reaches: the licence texts, the makefile, the ignore and
        # attribute files, the pyright configuration and the packaging manifest.
        failures: list[str] = []
        for path in provenance.document_files():
            text = path.read_text(encoding="utf-8")
            for label, marker in provenance.violations(text):
                failures.append(
                    f"{path.relative_to(ROOT)}: {label}: {marker!r}"
                )
        self.assertEqual(failures, [])

    def test_code_surfaces_have_no_provenance_markers(self):
        # The kernel, the reference implementation, the tooling and the suites:
        # surfaces whose text is compiled into a shipped artifact or published
        # with the tree. They are held to the development-corpus patterns as
        # well, because a shipped surface may only point at a live reference.
        patterns = (
            *provenance.PROVENANCE_PATTERNS.items(),
            *provenance.CODE_SURFACE_PATTERNS.items(),
        )
        failures: list[str] = []
        for path in provenance.code_surface_files():
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
        self.assertEqual(provenance.violations(representative_format_data), [])

    def test_synthetic_frame_count_is_not_an_address_marker(self):
        # `dwTotalPCMFrames` carries a synthetic frame count in the container
        # suites; the image-address pattern must not read it as a location.
        self.assertEqual(
            provenance.violations('"dwTotalPCMFrames": 0x10203040,'),
            [],
        )


if __name__ == "__main__":
    unittest.main()
