"""The compiled profile carrier: its stream, its identity, and its tables.

The carrier is one object seen three ways. The stream cases decode the
kernel's own little-endian dump and reject what this reader does not
understand; the resolution cases pin that a selection resolves by identity
and that an ambiguous or uninstalled one is refused rather than guessed at;
the table cases check the resident tables load and agree with the kernel's
own seq-0 packet. Each class keeps its own test names and failure messages.
"""

from __future__ import annotations

import dataclasses
import importlib.util
import struct
import unittest
import wwise_wem._core as _core
import wwise_wem_reference.profiles.artifact as artifact

from pathlib import Path
from tests.analysis_resource_support import installed_profile
from unittest.mock import patch
from wwise_wem import WwiseProfile, WwiseVersion
from wwise_wem.model import ContainerMetadata
from wwise_wem.profiles.key import ProfileKey
from wwise_wem_reference.profiles.artifact import (
    CompiledProfile,
    MAGIC,
    ProfileBlobError,
    VERSION,
    compiled_profiles,
    decode,
    resolve_selection,
)
from wwise_wem_reference.profiles.psychoacoustics.long_tables import (
    load_long_psy_tables,
)
from wwise_wem_reference.profiles.psychoacoustics.long_variants import load_long_variant
from wwise_wem_reference.profiles.psychoacoustics.short_tables import (
    load_short_psy_profiles,
)
from wwise_wem_reference.profiles.transform import load_mdct_looks
from wwise_wem_reference.profiles.transient import load_transient_tables

# --------------------------------------------------------------------------
# merged from tests/unit/profiles/test_profile_artifact.py
# --------------------------------------------------------------------------

# The compiled profile stream: decoding, rejection, identity reconstruction.
#
# The profile values are Rust constants in the kernel and reach every other
# language as one canonical little-endian stream
# (``wwise_wem._core.profile_tables()``). This suite is the reader's own test:
# it holds the decoder against the kernel's resolution, and it pins that a
# stream this reader does not understand is rejected instead of guessed at.
# There is no document and no resource tree behind it any more.

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
                # The identity is the key the stream carries — generation,
                # geometry and layout — reconstructed from those fields, with
                # nothing derived from the setup packet beside it.
                self.assertEqual(
                    profile.key,
                    ProfileKey(
                        profile.channels,
                        profile.sample_rate,
                        profile.generation,
                        profile.channel_layout,
                    ),
                )
                self.assertTrue(profile.setup_packet)
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


# --------------------------------------------------------------------------
# merged from tests/unit/profiles/test_profile_registry.py
# --------------------------------------------------------------------------

SELECTION = WwiseProfile(WwiseVersion.WWISE2013, 6, 44100)
TWO_CHANNEL_SELECTION = WwiseProfile(WwiseVersion.WWISE2013, 2, 48000)
UNINSTALLED_SELECTION = WwiseProfile(WwiseVersion.WWISE2013, 2, 44100)


class InstalledProfileTests(unittest.TestCase):
    def test_profile_key_keeps_complete_identity(self) -> None:
        key = ProfileKey(6, 44100, "2013.2", "5.1")
        self.assertEqual((key.channels, key.sample_rate), (6, 44100))
        self.assertEqual(key.generation, "2013.2")
        self.assertEqual(key.channel_layout, "5.1")
        self.assertEqual(key.label(), "6ch/44100Hz/2013.2")
        self.assertEqual(key.describe(), "6ch/44100Hz/2013.2/5.1")
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


# --------------------------------------------------------------------------
# merged from tests/unit/profiles/test_tables.py
# --------------------------------------------------------------------------

# The compiled carrier's tables: identity, geometry and loader agreement.

#: Enough frames for the kernel's stream session to emit its seq-0 packet.
MIN_FRAMES = 4096


class CarrierTableTests(unittest.TestCase):
    profile = installed_profile(6, 44100)

    def test_carried_setup_packet_is_the_kernel_packet(self):
        # The carrier used to record the packet's digest beside it and the
        # suite compared digests; the packet is compiled into the kernel, so
        # the comparison is made against the bytes themselves — the seq-0
        # packet the kernel emits for this profile's own selection.
        profile = self.profile
        selection = WwiseProfile(
            WwiseVersion.from_generation(profile.generation),
            profile.channels,
            profile.sample_rate,
        )
        packets = _core.StreamSession.for_selection(selection).push(
            bytes(MIN_FRAMES * profile.channels * 2)
        )
        self.assertTrue(packets, "the kernel emitted no seq-0 packet")
        self.assertEqual(bytes(profile.setup_packet), bytes(packets[0].data))

    def test_analysis_tables_load_and_validate(self):
        self.assertEqual(load_transient_tables(self.profile).n, 128)
        base = load_long_psy_tables(self.profile)
        self.assertEqual(base.n, 1024)
        self.assertEqual(load_long_variant(2, self.profile, base).n, 1024)
        self.assertEqual(load_long_variant(3, self.profile, base).n, 1024)
        self.assertEqual(len(load_short_psy_profiles(self.profile)), 2)

    def test_static_mdct_trig_banks(self):
        looks = load_mdct_looks(self.profile)
        self.assertEqual(looks[128].n, 128)
        self.assertEqual(looks[1024].n, 1024)


if __name__ == "__main__":
    unittest.main()
