"""The compiled profile stream: decoding, rejection, identity reconstruction.

The profile values are Rust constants in the kernel and reach every other
language as one canonical little-endian stream
(``wwise_wem._core.profile_tables()``). This suite is the reader's own test:
it holds the decoder against the kernel's resolution, and it pins that a
stream this reader does not understand is rejected instead of guessed at.
There is no document and no resource tree behind it any more.
"""

from __future__ import annotations

import importlib.util
import struct
import unittest
from pathlib import Path

import wwise_wem._core as _core
from wwise_wem import WwiseProfile, WwiseVersion
from wwise_wem.model import ContainerMetadata
from wwise_wem.profiles.key import ProfileKey
from wwise_wem_reference.profiles.artifact import (
    MAGIC,
    VERSION,
    ProfileBlobError,
    compiled_profiles,
    decode,
    resolve_selection,
)

PACKAGE_ROOT = Path(__file__).resolve().parents[3] / "src" / "wwise_wem"
SELECTION = WwiseProfile(WwiseVersion.WWISE2013, 6, 44100)


def _dump() -> bytes:
    return bytes(_core.profile_tables())


class ProfileStreamTests(unittest.TestCase):
    def test_every_installed_profile_carries_a_complete_identity(self) -> None:
        profiles = compiled_profiles()
        self.assertEqual(len(profiles), 2)
        for profile in profiles:
            with self.subTest(profile=profile.label()):
                self.assertIsInstance(profile.key, ProfileKey)
                self.assertIsInstance(profile.container_metadata, ContainerMetadata)
                self.assertEqual(profile.block_sizes, (256, 2048))
                self.assertEqual(
                    (profile.container_metadata.nChannels, profile.container_metadata.nSamplesPerSec),
                    (profile.channels, profile.sample_rate),
                )
                self.assertEqual(
                    profile.key.quality_setup_identity, f"sha256:{profile.setup_sha256}"
                )
                # A compiled profile is always complete: no draft state exists
                # without a manifest to declare one.
                self.assertTrue(profile.setup_available)
                self.assertIsNone(profile.pending_reason)

    def test_decoded_profiles_resolve_to_themselves(self) -> None:
        for profile in compiled_profiles():
            selection = WwiseProfile(
                WwiseVersion.from_generation(profile.generation),
                profile.channels,
                profile.sample_rate,
            )
            self.assertIs(resolve_selection(selection), profile)

    def test_the_human_label_is_derived_from_the_identity(self) -> None:
        profile = resolve_selection(SELECTION)
        self.assertEqual(profile.label(), "6ch/44100Hz/2013.2")
        self.assertEqual(profile.key.label(), profile.label())
        self.assertNotIn(
            "2013-6ch",
            profile.label(),
            "the label is geometry plus generation, never a directory name",
        )

    def test_a_wheel_carries_no_profile_resource_tree(self) -> None:
        # The transport this reader replaced shipped an index, a manifest and
        # one payload per table; nothing of it may come back.
        self.assertIsNone(importlib.util.find_spec("wwise_wem.data"))
        for module in (
            "wwise_wem.profiles.bundle",
            "wwise_wem.profiles.registry",
            "wwise_wem.profiles.resources",
            "wwise_wem.profiles.model",
        ):
            with self.subTest(module=module):
                self.assertIsNone(importlib.util.find_spec(module))

    def test_a_foreign_magic_is_rejected(self) -> None:
        with self.assertRaisesRegex(ProfileBlobError, "magic"):
            decode(b"NOTPROF\0" + _dump()[8:])

    def test_an_unknown_version_is_rejected(self) -> None:
        payload = bytearray(_dump())
        payload[8:12] = struct.pack("<I", VERSION + 1)
        with self.assertRaisesRegex(ProfileBlobError, "unsupported profile dump version"):
            decode(bytes(payload))

    def test_a_truncated_stream_is_rejected(self) -> None:
        with self.assertRaisesRegex(ProfileBlobError, "ends before its last field"):
            decode(_dump()[: len(_dump()) // 2])

    def test_the_version_the_reader_accepts_is_the_one_the_kernel_writes(self) -> None:
        payload = _dump()
        self.assertEqual(payload[: len(MAGIC)], MAGIC)
        self.assertEqual(struct.unpack("<I", payload[len(MAGIC) : len(MAGIC) + 4])[0], VERSION)


if __name__ == "__main__":
    unittest.main()
