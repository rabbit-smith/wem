from __future__ import annotations

import dataclasses
import unittest

from wwise_wem.model import ContainerMetadata
from wwise_wem.profiles.bundle import ProfileKey, load_profile_bundle
from wwise_wem.profiles.resources import ResourceRef
from wwise_wem.profiles.model import EncoderProfile, WwiseVorbisProfile
from wwise_wem.profiles.registry import (
    PROFILES,
    PROFILE_REGISTRY,
    ProfileRegistry,
    load_wem_profile,
    resolve_wem_profile,
)


class ProfileRegistryTests(unittest.TestCase):
    def test_profile_key_keeps_geometry_constructor_and_full_identity(self) -> None:
        key = ProfileKey(6, 44100)
        self.assertEqual((key.channels, key.sample_rate), (6, 44100))
        self.assertEqual(key.generation, "2013.2")
        self.assertEqual(key.channel_layout, "5.1")
        self.assertTrue(key.quality_setup_identity)
        with self.assertRaises(dataclasses.FrozenInstanceError):
            key.channels = 2  # type: ignore[misc]
        with self.assertRaisesRegex(ValueError, "requires explicit"):
            ProfileKey(2, 48000)
        explicit = ProfileKey(
            2,
            48000,
            generation="2013.2",
            channel_layout="stereo",
            quality_setup_identity="sha256:example",
        )
        self.assertEqual(explicit.channel_layout, "stereo")

    def test_encoder_profile_uses_typed_metadata_and_fresh_fmt_adapter(self) -> None:
        profile = resolve_wem_profile(6, 44100)
        self.assertIsInstance(profile, EncoderProfile)
        self.assertIsInstance(profile.container_metadata, ContainerMetadata)
        self.assertIs(WwiseVorbisProfile, EncoderProfile)
        first = profile.fmt
        second = profile.fmt
        self.assertEqual(first, second)
        self.assertIsNot(first, second)
        first["nChannels"] = 2
        self.assertEqual(profile.fmt["nChannels"], 6)
        self.assertEqual(profile.key, ProfileKey(6, 44100))
        self.assertEqual(profile.block_sizes, (256, 2048))
        manifest = profile.runtime_manifest()
        self.assertEqual(manifest["schema"], "wwise-wem.profile-manifest.v1")
        self.assertEqual(
            manifest["resources"]["vorbis.codebooks.t97"]["path"],
            "vorbis/codebooks/t97.json",
        )
        with self.assertRaises(TypeError):
            manifest["files"]["new"] = "digest"  # type: ignore[index]
        broken = dataclasses.replace(
            profile,
            setup_sha256="0" * 64,
            key=dataclasses.replace(
                profile.key,
                quality_setup_identity="sha256:" + "0" * 64,
            ),
        )
        with self.assertRaisesRegex(ValueError, "differs from installed"):
            broken.runtime_manifest()

    def test_installed_profile_is_constructed_from_the_checked_bundle(self) -> None:
        bundle = load_profile_bundle(verify_all=False)
        profile = load_wem_profile(bundle.name)
        self.assertEqual(profile.key, bundle.key)
        self.assertEqual(profile.container_metadata, bundle.container_metadata)
        self.assertEqual(profile.block_sizes, bundle.block_sizes)
        self.assertIsInstance(profile.setup_path, ResourceRef)
        self.assertEqual(profile.setup_path, bundle.setup)
        self.assertEqual(profile.setup_sha256, bundle.setup.sha256)

    def test_registry_resolve_get_list_and_mapping_compatibility(self) -> None:
        profile = load_wem_profile("wwise2013-6ch-44100")
        self.assertIs(PROFILE_REGISTRY.resolve(6, 44100), profile)
        self.assertIs(PROFILE_REGISTRY.resolve(ProfileKey(6, 44100)), profile)
        self.assertIs(PROFILE_REGISTRY.get(profile.name), profile)
        self.assertIs(PROFILE_REGISTRY.get(profile.key), profile)
        self.assertIs(PROFILE_REGISTRY[ProfileKey(6, 44100)], profile)
        self.assertEqual(PROFILE_REGISTRY.list(), (profile,))
        self.assertEqual(PROFILES[profile.name], profile)
        sentinel = object()
        self.assertIs(PROFILE_REGISTRY.get("missing", sentinel), sentinel)
        self.assertIs(PROFILE_REGISTRY.get(ProfileKey(6, 44100)), profile)
        with self.assertRaises(TypeError):
            PROFILES["other"] = profile  # type: ignore[index]
        self.assertIs(
            PROFILE_REGISTRY.resolve_setup(6, 44100, profile.setup_sha256),
            profile,
        )
        with self.assertRaisesRegex(ValueError, "setup SHA-256.*6ch/44100Hz"):
            PROFILE_REGISTRY.resolve_setup(6, 44100, "0" * 64)

    def test_registry_copies_input_and_rejects_duplicate_identity(self) -> None:
        profile = load_wem_profile("wwise2013-6ch-44100")
        source = [profile]
        registry = ProfileRegistry(source)
        source.clear()
        self.assertEqual(registry.list(), (profile,))
        with self.assertRaisesRegex(ValueError, "duplicate profile key"):
            ProfileRegistry([profile, dataclasses.replace(profile, name="alias")])
        with self.assertRaisesRegex(ValueError, "duplicate profile name"):
            ProfileRegistry(
                [
                    profile,
                    dataclasses.replace(
                        profile,
                        key=dataclasses.replace(
                            profile.key, channel_layout="custom"
                        ),
                    ),
                ]
            )

    def test_geometry_resolution_detects_multiple_quality_identities(self) -> None:
        profile = load_wem_profile("wwise2013-6ch-44100")
        alternate = dataclasses.replace(
            profile,
            name="wwise2013-6ch-44100-alternate",
            key=dataclasses.replace(profile.key, channel_layout="5.1-alternate"),
        )
        registry = ProfileRegistry([profile, alternate])
        with self.assertRaisesRegex(ValueError, "ambiguous.*complete ProfileKey"):
            registry.resolve(6, 44100)
        self.assertIs(registry.resolve(alternate.key), alternate)

    def test_legacy_resolution_errors_and_setup_checksum_remain_stable(self) -> None:
        profile = load_wem_profile("wwise2013-6ch-44100")
        self.assertEqual(len(profile.setup_packet()), 201)
        with self.assertRaisesRegex(ValueError, "unknown WEM profile"):
            load_wem_profile("missing")
        with self.assertRaisesRegex(ValueError, "no Wwise 2013.2 profile"):
            resolve_wem_profile(2, 48000)


if __name__ == "__main__":
    unittest.main()
