from __future__ import annotations

import dataclasses
import json
import unittest
from unittest.mock import patch

from wwise_wem import WwiseProfile, WwiseVersion
from wwise_wem.model import ContainerMetadata
from wwise_wem.profiles.bundle import (
    DEFAULT_INDEX,
    PACKAGE,
    ProfileKey,
    installed_profile_names,
    load_profile_bundle,
)
from wwise_wem.profiles.model import EncoderProfile
from wwise_wem.profiles.registry import resolve_selection
from wwise_wem.profiles.resources import ResourceRef, resource_traversable
import wwise_wem.profiles.registry as registry_module


SELECTION = WwiseProfile(WwiseVersion.WWISE2013, 6, 44100)
TWO_CHANNEL_SELECTION = WwiseProfile(WwiseVersion.WWISE2013, 2, 48000)
UNINSTALLED_SELECTION = WwiseProfile(WwiseVersion.WWISE2013, 2, 44100)


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

    def test_every_index_profile_resolves_by_its_manifest_key(self) -> None:
        index = json.loads(
            resource_traversable(PACKAGE, DEFAULT_INDEX).read_text(encoding="utf-8")
        )
        self.assertEqual(
            set(installed_profile_names()), set(index["profiles"])
        )
        for name in installed_profile_names():
            bundle = load_profile_bundle(profile=name, verify_all=False)
            selection = WwiseProfile(
                WwiseVersion.from_generation(bundle.key.generation),
                bundle.key.channels,
                bundle.key.sample_rate,
            )
            profile = resolve_selection(selection)
            self.assertIsInstance(profile, EncoderProfile)
            self.assertIsInstance(profile.container_metadata, ContainerMetadata)
            self.assertIsInstance(profile.setup_path, ResourceRef)
            self.assertEqual(profile.key, bundle.key)
            self.assertEqual(profile.setup_sha256, bundle.setup.sha256)

    def test_resolution_is_cached_and_geometry_independent(self) -> None:
        self.assertIs(resolve_selection(SELECTION), resolve_selection(SELECTION))
        self.assertIs(
            resolve_selection(TWO_CHANNEL_SELECTION),
            resolve_selection(TWO_CHANNEL_SELECTION),
        )
        self.assertIsNot(
            resolve_selection(SELECTION), resolve_selection(TWO_CHANNEL_SELECTION)
        )

    def test_resolution_errors_are_explicit(self) -> None:
        with self.assertRaisesRegex(
            ValueError, "no installed Wwise 2013 profile for 2ch/44100Hz"
        ):
            resolve_selection(UNINSTALLED_SELECTION)
        with self.assertRaisesRegex(ValueError, "installed: "):
            resolve_selection(WwiseProfile(WwiseVersion.WWISE2013, 4, 48000))

    def test_ambiguous_selection_is_never_resolved_by_first_match(self) -> None:
        base = resolve_selection(TWO_CHANNEL_SELECTION)
        duplicate = dataclasses.replace(base, name="duplicate-2ch-profile")
        with patch.object(
            registry_module,
            "_installed_profiles",
            return_value=(base, duplicate),
        ):
            with self.assertRaisesRegex(ValueError, "ambiguous"):
                resolve_selection(TWO_CHANNEL_SELECTION)

    def test_quality_binding_is_pure_and_finite(self) -> None:
        base = resolve_selection(SELECTION)
        bound = resolve_selection(SELECTION, quality=4.0)
        self.assertIsNot(base, bound)
        self.assertEqual(bound.quality, 4.0)
        self.assertIsNone(base.quality)
        for value in (float("nan"), float("inf")):
            with self.assertRaisesRegex(ValueError, "finite"):
                resolve_selection(SELECTION, quality=value)


if __name__ == "__main__":
    unittest.main()
