from __future__ import annotations

import hashlib
import unittest
from dataclasses import FrozenInstanceError

from wwise_wem.application.models import EncodeResult, EncodeStats
from wwise_wem.model import (
    ContainerMetadata,
    PacketResult,
    PcmBuffer,
    SetupConfig,
)
from wwise_wem.profiles.registry import WWISE2013_6CH_44100


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
        metadata = ContainerMetadata.from_fmt_dict(WWISE2013_6CH_44100.fmt)
        first = metadata.to_fmt_dict(frame_count=1234)
        second = metadata.to_fmt_dict(frame_count=1234)
        self.assertEqual(set(first), set(WWISE2013_6CH_44100.fmt))
        self.assertEqual(first["dwTotalPCMFrames"], 1234)
        self.assertEqual(second, first)
        self.assertIsNot(first, second)
        first["nChannels"] = 99
        self.assertEqual(metadata.nChannels, 6)
        self.assertEqual(second["nChannels"], 6)
        with self.assertRaises(FrozenInstanceError):
            metadata.nChannels = 2

    def test_rejects_invalid_geometry_and_frame_count(self):
        values = dict(WWISE2013_6CH_44100.fmt)
        values["nChannels"] = 0
        with self.assertRaisesRegex(ValueError, "nChannels"):
            ContainerMetadata.from_fmt_dict(values)
        metadata = ContainerMetadata.from_fmt_dict(WWISE2013_6CH_44100.fmt)
        with self.assertRaisesRegex(ValueError, "frame_count"):
            metadata.to_fmt_dict(frame_count=-1)


class SetupConfigTests(unittest.TestCase):
    def test_copies_and_freezes_parsed_mapping(self):
        source = {"floors": [{"multiplier": 2}], "book_ids": [1, 2]}
        setup = SetupConfig(b"setup", 6, [1, 2], source)
        source["book_ids"].append(3)
        source["floors"][0]["multiplier"] = 4
        self.assertEqual(setup.raw_packet, b"setup")
        self.assertEqual(setup.book_ids, (1, 2))
        self.assertEqual(setup.parsed["book_ids"], (1, 2))
        self.assertEqual(setup.parsed["floors"][0]["multiplier"], 2)
        with self.assertRaises(TypeError):
            setup.parsed["new"] = 1
        with self.assertRaises(TypeError):
            setup.parsed["floors"][0]["multiplier"] = 3
        with self.assertRaises(FrozenInstanceError):
            setup.channels = 2

    def test_rejects_empty_or_invalid_setup(self):
        with self.assertRaisesRegex(ValueError, "packet must not be empty"):
            SetupConfig(b"", 6, (1,), {})
        with self.assertRaisesRegex(ValueError, "at least one book"):
            SetupConfig(b"setup", 6, (), {})
        with self.assertRaisesRegex(ValueError, "channels"):
            SetupConfig(b"setup", 0, (1,), {})


class PacketResultTests(unittest.TestCase):
    def test_packet_metadata_is_immutable_and_hashed(self):
        source = bytearray(b"packet")
        result = PacketResult(source, mode=1, previous_mode=0, following_mode=1)
        source[0] = 0
        self.assertEqual(result.packet, b"packet")
        self.assertEqual(result.size, 6)
        self.assertEqual(result.sha256, hashlib.sha256(b"packet").hexdigest())
        with self.assertRaises(FrozenInstanceError):
            result.mode = 0

    def test_rejects_empty_packet_and_invalid_modes(self):
        with self.assertRaisesRegex(ValueError, "must not be empty"):
            PacketResult(b"", 0, 0, 0)
        for field in ("mode", "previous_mode", "following_mode"):
            values = {"mode": 0, "previous_mode": 0, "following_mode": 0}
            values[field] = 2
            with self.subTest(field=field):
                with self.assertRaisesRegex(ValueError, field):
                    PacketResult(b"packet", **values)


class EncodeResultTests(unittest.TestCase):
    def test_legacy_dict_and_tuple_roundtrip(self):
        legacy_stats = {
            "pcm_frames": 4096,
            "channels": 6,
            "audio_packets": 3,
            "short_packets": 2,
            "long_packets": 1,
            "bytes": 4,
            "metadata_source": "profile:wwise2013-6ch-44100",
            "engine": "python",
        }
        stats = EncodeStats.from_legacy_dict(legacy_stats)
        self.assertEqual(stats.to_legacy_dict(), legacy_stats)
        result = EncodeResult.from_legacy_tuple((b"WEM!", legacy_stats))
        self.assertEqual(result.stats, stats)
        self.assertEqual(result.sha256, hashlib.sha256(b"WEM!").hexdigest())
        data, converted = result.to_legacy_tuple()
        self.assertEqual(data, b"WEM!")
        self.assertEqual(converted, legacy_stats)
        self.assertEqual(EncodeStats.from_legacy(legacy_stats), stats)
        self.assertEqual(stats.to_legacy(), legacy_stats)
        self.assertEqual(EncodeResult.from_legacy((b"WEM!", legacy_stats)), result)
        self.assertEqual(result.to_legacy(), (b"WEM!", legacy_stats))
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
        with self.assertRaisesRegex(ValueError, "data and stats"):
            EncodeResult.from_legacy_tuple((b"only",))


if __name__ == "__main__":
    unittest.main()
