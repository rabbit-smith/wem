from __future__ import annotations

import dataclasses
import json
import unittest
from unittest.mock import patch

from wwise_wem.model import ContainerMetadata
from wwise_wem.profiles.bundle import DEFAULT_INDEX, PACKAGE, ProfileKey, load_profile_bundle
from wwise_wem.profiles.model import EncoderProfile
from wwise_wem.profiles.registry import (
    WWISE2013_2CH_48000,
    WWISE2013_6CH_44100,
    load_wem_profile,
    profile_names,
    resolve_wem_profile,
)
from wwise_wem.profiles.resources import ResourceRef, resource_traversable
import wwise_wem.profiles.registry as registry_module


class InstalledProfileTests(unittest.TestCase):
    def test_profile_key_keeps_complete_identity(self) -> None:
        key = ProfileKey(6, 44100, "2013.2", "5.1", "sha256:example")
        self.assertEqual((key.channels, key.sample_rate), (6, 44100))
        self.assertEqual(key.generation, "2013.2")
        self.assertEqual(key.channel_layout, "5.1")
        with self.assertRaises(dataclasses.FrozenInstanceError):
            key.channels = 2  # type: ignore[misc]
        with self.assertRaises(TypeError):
            ProfileKey(2, 48000)  # type: ignore[call-arg]

    def test_installed_profiles_come_from_the_checked_index(self) -> None:
        index = json.loads(
            resource_traversable(PACKAGE, DEFAULT_INDEX).read_text(encoding="utf-8")
        )
        self.assertEqual(profile_names(), tuple(sorted(index["profiles"])))
        for name in profile_names():
            bundle = load_profile_bundle(profile=name, verify_all=False)
            profile = load_wem_profile(name)
            self.assertIsInstance(profile, EncoderProfile)
            self.assertIsInstance(profile.container_metadata, ContainerMetadata)
            self.assertIsInstance(profile.setup_path, ResourceRef)
            self.assertEqual(profile.key, bundle.key)
            self.assertEqual(profile.setup_sha256, bundle.setup.sha256)

    def test_name_and_geometry_resolution_share_cached_profiles(self) -> None:
        self.assertIs(load_wem_profile("wwise2013-6ch-44100"), resolve_wem_profile(6, 44100))
        self.assertIs(load_wem_profile("wwise2013-2ch-48000"), resolve_wem_profile(2, 48000))
        self.assertIs(WWISE2013_6CH_44100, resolve_wem_profile(6, 44100))
        self.assertIs(WWISE2013_2CH_48000, resolve_wem_profile(2, 48000))

    def test_resolution_errors_are_explicit(self) -> None:
        with self.assertRaisesRegex(ValueError, "unknown WEM profile"):
            load_wem_profile("missing")
        with self.assertRaisesRegex(ValueError, "no Wwise 2013.2 profile"):
            resolve_wem_profile(2, 44100)

    def test_geometry_resolution_rejects_ambiguous_profiles(self) -> None:
        base = load_wem_profile("wwise2013-2ch-48000")
        duplicate = dataclasses.replace(base, name="duplicate-2ch-profile")
        with patch.dict(
            registry_module._INSTALLED_PROFILES,
            {duplicate.name: duplicate},
        ):
            with self.assertRaisesRegex(ValueError, "multiple .* profiles match"):
                resolve_wem_profile(2, 48000)

    def test_quality_binding_is_pure_and_finite(self) -> None:
        base = load_wem_profile("wwise2013-6ch-44100")
        bound = load_wem_profile(base.name, quality=4.0)
        self.assertIsNot(base, bound)
        self.assertEqual(bound.quality, 4.0)
        self.assertIsNone(base.quality)
        for value in (float("nan"), float("inf")):
            with self.assertRaisesRegex(ValueError, "finite"):
                load_wem_profile(base.name, quality=value)


if __name__ == "__main__":
    unittest.main()
