"""Exact public and wheel-source distribution boundaries."""

from __future__ import annotations

import json
import unittest
from pathlib import Path

import wwise_wem


ROOT = Path(__file__).resolve().parents[2]
PACKAGE = ROOT / "src" / "wwise_wem"
ALLOWLIST = Path(__file__).with_name("distribution_allowlist.json")


def _contract() -> dict[str, object]:
    return json.loads(ALLOWLIST.read_text(encoding="utf-8"))


def _wheel_name(path: Path) -> str:
    return "wwise_wem/" + path.relative_to(PACKAGE).as_posix()


class DistributionContractTests(unittest.TestCase):
    def test_root_all_is_the_exact_supported_surface(self) -> None:
        expected = tuple(_contract()["root_exports"])
        self.assertEqual(tuple(wwise_wem.__all__), expected)
        self.assertEqual(
            tuple(name for name in expected if name not in dir(wwise_wem)),
            (),
        )

    def test_source_modules_and_resources_match_distribution_allowlist(self) -> None:
        contract = _contract()
        self.assertEqual(
            contract["schema"],
            "wwise-wem.distribution-allowlist.v1",
        )
        modules = sorted(_wheel_name(path) for path in PACKAGE.rglob("*.py"))
        resources = sorted(
            _wheel_name(path)
            for path in (PACKAGE / "data").rglob("*")
            if path.is_file()
        )
        self.assertEqual(modules, contract["modules"])
        self.assertEqual(resources, contract["resources"])

    def test_allowlist_excludes_deleted_and_research_modules(self) -> None:
        contract = _contract()
        paths = tuple(contract["modules"]) + tuple(contract["resources"])
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
        markers = tuple(str(value).lower() for value in contract["forbidden_path_markers"])
        violations = [
            path
            for path in paths
            if any(marker in path.lower() for marker in markers)
        ]
        self.assertEqual(violations, [])

        content_markers = tuple(
            str(value).encode("ascii").lower()
            for value in contract["forbidden_content_markers"]
        )
        content_violations = []
        for name in paths:
            if not name.endswith((".py", ".json")):
                continue
            payload = (ROOT / "src" / name).read_bytes().lower()
            if any(marker in payload for marker in content_markers):
                content_violations.append(name)
        self.assertEqual(content_violations, [])

    def test_native_extension_contract_stays_synced_across_surfaces(self) -> None:
        """The in-package extension must agree across its name surfaces.

        The wheel ships the abi3 extension as `wwise_wem/_core.abi3.so`
        (or `.pyd`).  Drift between the allowlist, the Cargo lib name,
        and the maturin module-name settings would silently break either
        the import or the wheel layout, so the spellings are pinned here
        in one place.  The module name is an install-time fact owned by
        these files only: the facade imports `wwise_wem._core` directly
        and keeps no second copy of the name.
        """
        contract = _contract()
        native = contract["native_extensions"]
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


if __name__ == "__main__":
    unittest.main()
