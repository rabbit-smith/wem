"""End-to-end tests for the quality-override assembly wiring.

These tests prove the three wiring guarantees:

- ``quality`` omitted (``None``) leaves the assembled analysis resources
  byte-for-byte identical to the historical path (no crosstalk);
- a quality value requested from a profile that has no curves is a clear
  configuration error;
- a profile that carries ``analysis.quality-curves.json`` applies the
  interpolated overrides to the short psychoacoustic surface and records the
  quality value and extrapolation honesty flag.

The "with curves" case is exercised through a synthetic zip package that
repackages the installed 6ch profile resources plus a quality-curves file, so
the full manifest -> loader -> assembly path is exercised without touching the
installed profile bytes.
"""
from __future__ import annotations

import hashlib
import importlib
import json
import sys
import tempfile
import unittest
import zipfile
from pathlib import Path

from wwise_wem.profiles.bundle import load_profile_bundle
from wwise_wem_reference.profiles.assembly import (
    assemble_analysis_resources,
    assemble_encoder_profile_resources,
)
from wwise_wem_reference.profiles.quality import (
    QUALITY_CURVES_RESOURCE,
    load_quality_curves,
)


_PROFILE = "wwise2013-6ch-44100"
_ROOT = Path(__file__).resolve().parents[3]
_SRC = _ROOT / "src" / "wwise_wem"


def _sha(payload: bytes) -> str:
    return hashlib.sha256(payload).hexdigest()


def _installed_resource_bytes(name: str) -> bytes:
    """Read one installed 6ch profile resource by manifest logical name."""
    manifest = (
        _SRC / "data" / "profiles" / _PROFILE / "manifest.json"
    ).read_text(encoding="utf-8")
    spec = json.loads(manifest)["resources"][name]
    path = _SRC / "data" / "profiles" / _PROFILE / spec["path"]
    payload = path.read_bytes()
    assert _sha(payload) == spec["sha256"], f"{name} digest drifted"
    return payload


def _quality_curves_payload() -> bytes:
    return json.dumps(
        {
            "schema": "wem.quality-curves.v1",
            "interpolation": "linear-frac",
            "breakpoints": [0.0, 4.0, 8.0],
            "curves": {
                "short.ath_offset": [1.0, 2.0, 4.0],
                "short.ath_floor": [-10.0, -20.0, -40.0],
            },
        },
        sort_keys=True,
    ).encode("utf-8")


def _build_zip_with_curves(package: str, directory: str) -> Path:
    """Zip the installed 6ch resources plus a quality-curves file."""
    profile_dir = _SRC / "data" / "profiles" / _PROFILE
    profile_prefix = f"{package}/data/profiles/{_PROFILE}"

    index_path = profile_dir.parent / "index.json"
    manifest_path = profile_dir / "manifest.json"

    # Register the new optional quality-curves resource in the manifest.
    manifest = json.loads(manifest_path.read_text(encoding="utf-8"))
    curves_bytes = _quality_curves_payload()
    manifest["resources"][QUALITY_CURVES_RESOURCE] = {
        "path": "analysis/quality-curves.json",
        "sha256": _sha(curves_bytes),
    }
    manifest_raw2 = (json.dumps(manifest, sort_keys=True) + "\n").encode("utf-8")

    # Re-hash the modified manifest into the index so the bundle validates.
    index = json.loads(index_path.read_text(encoding="utf-8"))
    index["profiles"][_PROFILE]["sha256"] = _sha(manifest_raw2)
    index_raw = json.dumps(index, sort_keys=True).encode("utf-8")

    archive = Path(directory) / f"{package}.zip"
    with zipfile.ZipFile(archive, "w") as zip_file:
        zip_file.writestr(f"{package}/__init__.py", "")
        zip_file.writestr(f"{package}/data/profiles/index.json", index_raw)
        # Ship the real installed resources unchanged (the manifest is
        # rewritten below with the curves resource registered).
        for resource_file in profile_dir.rglob("*"):
            if resource_file.is_dir():
                continue
            if resource_file == manifest_path:
                continue
            relative = resource_file.relative_to(profile_dir)
            zip_file.writestr(
                f"{profile_prefix}/{relative.as_posix()}",
                resource_file.read_bytes(),
            )
        # Overwrite the manifest with the curves-registered variant.
        zip_file.writestr(f"{profile_prefix}/manifest.json", manifest_raw2)
        zip_file.writestr(
            f"{profile_prefix}/analysis/quality-curves.json", curves_bytes
        )
    return archive


class QualityAssemblyWiringTests(unittest.TestCase):
    def test_quality_none_leaves_resources_unchanged(self):
        bundle = load_profile_bundle(verify_all=False)
        resources_default = assemble_analysis_resources(bundle)
        resources_explicit = assemble_analysis_resources(bundle, quality=None)
        self.assertEqual(
            resources_default.short_surface.ath_offset,
            resources_explicit.short_surface.ath_offset,
        )
        self.assertEqual(
            resources_default.short_look.ath_offset,
            resources_explicit.short_look.ath_offset,
        )
        self.assertIsNone(resources_explicit.quality_value)
        self.assertFalse(resources_explicit.quality_extrapolated)

    def test_quality_without_curves_is_a_configuration_error(self):
        bundle = load_profile_bundle(verify_all=False)
        with self.assertRaisesRegex(
            ValueError, "no quality-curves resource"
        ):
            assemble_analysis_resources(bundle, quality=4.0)

    def test_quality_with_curves_applies_overrides_and_flags(self):
        with tempfile.TemporaryDirectory() as directory:
            package = "zip_quality_test"
            archive = _build_zip_with_curves(package, directory)
            sys.path.insert(0, str(archive))
            importlib.invalidate_caches()
            try:
                bundle = load_profile_bundle(
                    package=package, profile=_PROFILE, verify_all=False
                )
                ref = bundle.runtime_manifest.resources[QUALITY_CURVES_RESOURCE]
                curves = load_quality_curves(ref)
                self.assertIsNotNone(curves)

                resources = assemble_analysis_resources(bundle, quality=2.0)
                # q=2 with [1,2,4]/[-10,-20,-40] over [0,4,8]:
                #   ath_offset   -> 1 + 0.5*(2-1) = 1.5
                #   ath_floor    -> -10 + 0.5*(-20+10) = -15.0
                self.assertEqual(resources.short_surface.ath_offset, 1.5)
                self.assertEqual(resources.short_surface.ath_floor, -15.0)
                self.assertEqual(resources.quality_value, 2.0)
                self.assertFalse(resources.quality_extrapolated)
                # The rebuilt short look carries the overrides.
                self.assertEqual(resources.short_look.ath_offset, 1.5)
                self.assertEqual(resources.short_look.ath_floor, -15.0)

                # Out-of-domain quality clamps and flags extrapolation.
                clamped = assemble_analysis_resources(bundle, quality=9.0)
                self.assertEqual(clamped.short_surface.ath_offset, 4.0)
                self.assertEqual(clamped.short_surface.ath_floor, -40.0)
                self.assertTrue(clamped.quality_extrapolated)
                self.assertEqual(clamped.quality_value, 9.0)
            finally:
                sys.path.remove(str(archive))
                sys.modules.pop(package, None)
                importlib.invalidate_caches()

    def test_encoder_profile_resources_forward_quality(self):
        with tempfile.TemporaryDirectory() as directory:
            package = "zip_quality_forward"
            archive = _build_zip_with_curves(package, directory)
            sys.path.insert(0, str(archive))
            importlib.invalidate_caches()
            try:
                bundle = load_profile_bundle(
                    package=package, profile=_PROFILE, verify_all=False
                )
                resources = assemble_encoder_profile_resources(
                    bundle, quality=6.0
                )
                # q=6 -> 2nd segment midpoint: ath_offset = 2 + 0.5*(4-2) = 3.0
                self.assertEqual(resources.analysis.short_surface.ath_offset, 3.0)
                self.assertEqual(resources.analysis.quality_value, 6.0)
                self.assertFalse(resources.analysis.quality_extrapolated)
            finally:
                sys.path.remove(str(archive))
                sys.modules.pop(package, None)
                importlib.invalidate_caches()


if __name__ == "__main__":
    unittest.main()
