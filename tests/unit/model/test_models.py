from __future__ import annotations

import hashlib
import unittest
from dataclasses import FrozenInstanceError

from wwise_wem.application.models import EncodeResult, EncodeStats
from wwise_wem.model import (
    ContainerMetadata,
    PcmBuffer,
    RawPcm,
)
from wwise_wem import WwiseProfile, WwiseVersion
from wwise_wem.profiles.registry import resolve_selection


SIX_CHANNEL_PROFILE = resolve_selection(
    WwiseProfile(WwiseVersion.WWISE2013, 6, 44100)
)


class PcmBufferTests(unittest.TestCase):
    def test_normalizes_to_immutable_equal_length_channels(self):
        source = [[0, 0.5], [-0.5, 1]]
        pcm = PcmBuffer(44100, source)
        source[0][0] = 99
        self.assertEqual(pcm.channels, ((0.0, 0.5), (-0.5, 1.0)))
        self.assertEqual((pcm.channel_count, pcm.frame_count), (2, 2))
        with self.assertRaises(FrozenInstanceError):
            pcm.sample_rate = 48000

    def test_rejects_invalid_geometry(self):
        for rate, channels, message in (
            (0, ((0.0,),), "sample_rate"),
            (44100, (), "at least one channel"),
            (44100, ((),), "at least one frame"),
            (44100, ((0.0,), (0.0, 1.0)), "equal frame counts"),
        ):
            with self.subTest(rate=rate, channels=channels):
                with self.assertRaisesRegex((TypeError, ValueError), message):
                    PcmBuffer(rate, channels)


class ContainerMetadataTests(unittest.TestCase):
    def test_profile_fmt_roundtrip_returns_fresh_dict(self):
        profile_fmt = SIX_CHANNEL_PROFILE.container_metadata.to_fmt_dict()
        metadata = ContainerMetadata.from_fmt_dict(profile_fmt)
        first = metadata.to_fmt_dict(frame_count=1234)
        second = metadata.to_fmt_dict(frame_count=1234)
        self.assertEqual(set(first), set(profile_fmt))
        self.assertEqual(first["dwTotalPCMFrames"], 1234)
        self.assertEqual(second, first)
        self.assertIsNot(first, second)
        first["nChannels"] = 99
        self.assertEqual(metadata.nChannels, 6)
        self.assertEqual(second["nChannels"], 6)
        with self.assertRaises(FrozenInstanceError):
            metadata.nChannels = 2

    def test_rejects_invalid_geometry_and_frame_count(self):
        values = SIX_CHANNEL_PROFILE.container_metadata.to_fmt_dict()
        values["nChannels"] = 0
        with self.assertRaisesRegex(ValueError, "nChannels"):
            ContainerMetadata.from_fmt_dict(values)
        metadata = ContainerMetadata.from_fmt_dict(
            SIX_CHANNEL_PROFILE.container_metadata.to_fmt_dict()
        )
        with self.assertRaisesRegex(ValueError, "frame_count"):
            metadata.to_fmt_dict(frame_count=-1)


class RawPcmTests(unittest.TestCase):
    def test_copies_bytes_and_validates_format(self):
        source = bytearray(b"\0\0")
        raw = RawPcm(source, 48000, 2, "s16le")
        source[0] = 1
        self.assertEqual(raw.data, b"\0\0")
        with self.assertRaisesRegex(ValueError, "sample_format"):
            RawPcm(b"\0", 48000, 2, "u8")  # type: ignore[arg-type]


class EncodeResultTests(unittest.TestCase):
    def test_stats_dictionary_is_an_independent_value(self):
        expected = {
            "pcm_frames": 4096,
            "channels": 6,
            "audio_packets": 3,
            "short_packets": 2,
            "long_packets": 1,
            "bytes": 4,
            "metadata_source": "profile:6ch/44100Hz/2013",
        }
        stats = EncodeStats(4096, 6, 3, 2, 1, 4, "profile:6ch/44100Hz/2013")
        self.assertEqual(stats.to_dict(), expected)
        result = EncodeResult(b"WEM!", stats)
        self.assertEqual(result.stats, stats)
        self.assertEqual(result.sha256, hashlib.sha256(b"WEM!").hexdigest())
        converted = stats.to_dict()
        converted["bytes"] = 100
        self.assertEqual(result.stats.bytes, 4)
        with self.assertRaises(FrozenInstanceError):
            result.data = b"other"

    def test_rejects_inconsistent_counts_and_data_size(self):
        with self.assertRaisesRegex(ValueError, "packet counts"):
            EncodeStats(1, 1, 2, 1, 0, 4, "profile:test")
        stats = EncodeStats(1, 1, 1, 1, 0, 4, "profile:test")
        with self.assertRaisesRegex(ValueError, "byte count"):
            EncodeResult(b"bad", stats)


if __name__ == "__main__":
    unittest.main()
