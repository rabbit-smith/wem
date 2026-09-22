from __future__ import annotations

import dataclasses
import unittest
from unittest.mock import patch

from wwise_wem import WwiseProfile, WwiseVersion
from wwise_wem.model import ContainerMetadata
from wwise_wem.profiles.key import ProfileKey
import wwise_wem_reference.profiles.artifact as artifact
from wwise_wem_reference.profiles.artifact import CompiledProfile, resolve_selection


SELECTION = WwiseProfile(WwiseVersion.WWISE2013, 6, 44100)
TWO_CHANNEL_SELECTION = WwiseProfile(WwiseVersion.WWISE2013, 2, 48000)
UNINSTALLED_SELECTION = WwiseProfile(WwiseVersion.WWISE2013, 2, 44100)


class InstalledProfileTests(unittest.TestCase):
    def test_profile_key_keeps_complete_identity(self) -> None:
        key = ProfileKey(6, 44100, "2013.2", "5.1", "sha256:example")
        self.assertEqual((key.channels, key.sample_rate), (6, 44100))
        self.assertEqual(key.generation, "2013.2")
        self.assertEqual(key.channel_layout, "5.1")
        self.assertEqual(key.label(), "6ch/44100Hz/2013.2")
        self.assertEqual(key.describe(), "6ch/44100Hz/2013.2/5.1(sha256:example)")
        with self.assertRaises(dataclasses.FrozenInstanceError):
            key.channels = 2  # type: ignore[misc]
        with self.assertRaises(TypeError):
            ProfileKey(2, 48000)  # type: ignore[call-arg]

    def test_every_installed_profile_resolves_by_its_own_identity(self) -> None:
        for installed in artifact.compiled_profiles():
            with self.subTest(profile=installed.label()):
                selection = WwiseProfile(
                    WwiseVersion.from_generation(installed.generation),
                    installed.channels,
                    installed.sample_rate,
                )
                profile = resolve_selection(selection)
                self.assertIsInstance(profile, CompiledProfile)
                self.assertIsInstance(profile.container_metadata, ContainerMetadata)
                self.assertIs(profile, installed)
                self.assertEqual(profile.key, installed.key)

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
        # Two profiles that share generation and geometry are two identities a
        # selection cannot tell apart, whatever their layout says.
        duplicate = dataclasses.replace(base, channel_layout="stereo-twin")
        with patch.object(
            artifact,
            "compiled_profiles",
            return_value=(base, duplicate),
        ):
            with self.assertRaisesRegex(ValueError, "ambiguous") as caught:
                resolve_selection(TWO_CHANNEL_SELECTION)
        self.assertIn("stereo-twin", str(caught.exception))

    def test_quality_binding_is_pure_and_finite(self) -> None:
        base = resolve_selection(SELECTION)
        bound = resolve_selection(SELECTION, quality=4.0)
        self.assertIsNot(base, bound)
        self.assertEqual(bound.quality, 4.0)
        self.assertIsNone(base.quality)
        # The carrier is never mutated by a binding.
        self.assertIsNone(resolve_selection(SELECTION).quality)
        for value in (float("nan"), float("inf")):
            with self.assertRaisesRegex(ValueError, "finite"):
                resolve_selection(SELECTION, quality=value)


if __name__ == "__main__":
    unittest.main()
