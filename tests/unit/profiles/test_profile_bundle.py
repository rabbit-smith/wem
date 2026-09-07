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
from wwise_wem.profiles.resources import ResourceRef, normalize_resource_path
from wwise_wem.profiles.registry import load_wem_profile


def _sha(payload: bytes) -> str:
    return hashlib.sha256(payload).hexdigest()


def _zip_package(path: Path, package: str, mutate=None) -> None:
    setup = b"minimal setup packet"
    installed = load_wem_profile("wwise2013-6ch-44100")
    manifest = {
        "schema": "wwise-wem.profile-manifest.v1",
        "name": "zip-profile",
        "key": {
            "generation": "2013.2", "channels": 6, "sample_rate": 44100,
            "channel_layout": "5.1", "quality_setup_identity": f"sha256:{_sha(setup)}",
        },
        "block_sizes": [256, 2048],
        "container_metadata": installed.fmt,
        "resources": {
            "vorbis.setup": {"path": "vorbis/setup.bin", "sha256": _sha(setup)},
        },
    }
    if mutate is not None:
        mutate(manifest)
    manifest_raw = (json.dumps(manifest, sort_keys=True) + "\n").encode()
    index = {
        "schema": "wwise-wem.profile-index.v1",
        "default": "zip-profile",
        "profiles": {
            "zip-profile": {
                "manifest": "zip-profile/manifest.json", "sha256": _sha(manifest_raw)
            }
        },
    }
    with zipfile.ZipFile(path, "w") as archive:
        archive.writestr(f"{package}/__init__.py", "")
        archive.writestr(f"{package}/data/profiles/index.json", json.dumps(index))
        archive.writestr(f"{package}/data/profiles/zip-profile/manifest.json", manifest_raw)
        archive.writestr(f"{package}/data/profiles/zip-profile/vorbis/setup.bin", setup)


class ProfileBundleTests(unittest.TestCase):
    def test_installed_bundle_validates_all_runtime_resources(self) -> None:
        bundle = load_profile_bundle()
        self.assertEqual(bundle.name, "wwise2013-6ch-44100")
        self.assertEqual((bundle.key.channels, bundle.key.sample_rate), (6, 44100))
        self.assertEqual(bundle.block_sizes, (256, 2048))
        self.assertEqual(len(bundle.setup_packet()), 201)
        self.assertEqual(len(bundle.runtime_manifest.resources), 10)
        bundle.verify_all()
        with self.assertRaises(TypeError):
            bundle.runtime_manifest.resources["new"] = bundle.setup  # type: ignore[index]

    def test_resource_paths_and_digests_are_strict(self) -> None:
        for path in ("", ".", "../secret", "/absolute", "data\\table"):
            with self.subTest(path=path):
                with self.assertRaises(ValueError):
                    normalize_resource_path(path)
        with self.assertRaisesRegex(ValueError, "SHA-256"):
            ResourceRef("wwise_wem", normalize_resource_path("data/profiles/index.json"), "bad")
        with self.assertRaisesRegex(ValueError, "SHA-256 differs"):
            ResourceRef("wwise_wem", normalize_resource_path("data/profiles/index.json"), "0" * 64).read_bytes()

    def test_bundle_loads_through_index_from_zip_resources(self) -> None:
        with tempfile.TemporaryDirectory() as directory:
            archive = Path(directory) / "profile.zip"
            package = "zip_profile_ok"
            _zip_package(archive, package)
            sys.path.insert(0, str(archive))
            importlib.invalidate_caches()
            try:
                bundle = load_profile_bundle(package=package, verify_all=True)
                self.assertEqual(bundle.setup_packet(), b"minimal setup packet")
            finally:
                sys.path.remove(str(archive))
                sys.modules.pop(package, None)
                importlib.invalidate_caches()

    def test_manifest_schema_geometry_sha_and_path_errors_are_rejected(self) -> None:
        cases = (
            ("schema", lambda value: value.__setitem__("schema", "unknown"), "schema"),
            ("geometry", lambda value: value["container_metadata"].__setitem__("nChannels", 2), "geometry"),
            ("unsafe_path", lambda value: value["resources"]["vorbis.setup"].__setitem__("path", "../setup.bin"), "unsafe"),
        )
        with tempfile.TemporaryDirectory() as directory:
            for index, (label, mutate, message) in enumerate(cases):
                with self.subTest(case=label):
                    archive = Path(directory) / f"{label}.zip"
                    package = f"zip_profile_bad_{index}"
                    _zip_package(archive, package, mutate)
                    sys.path.insert(0, str(archive))
                    importlib.invalidate_caches()
                    try:
                        with self.assertRaisesRegex(ValueError, message):
                            load_profile_bundle(package=package)
                    finally:
                        sys.path.remove(str(archive))
                        sys.modules.pop(package, None)
                        importlib.invalidate_caches()


if __name__ == "__main__":
    unittest.main()
