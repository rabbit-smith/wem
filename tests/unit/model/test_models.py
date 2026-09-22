from __future__ import annotations

import pickle
import tempfile
import unittest
from dataclasses import FrozenInstanceError
from pathlib import Path

from wwise_wem.application.models import EncodeResult, EncodeStats
from wwise_wem.model import (
    ContainerMetadata,
    PcmBuffer,
    RawPcm,
    WwiseWemError,
)
from wwise_wem import WwiseProfile, WwiseVersion
from wwise_wem_reference.profiles.artifact import resolve_selection


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
        # The statistics are the observations alone: no container length
        # (that is ``len(result)``) and no label naming the selection the
        # caller itself passed.
        expected = {
            "pcm_frames": 4096,
            "channels": 6,
            "audio_packets": 3,
            "short_packets": 2,
            "long_packets": 1,
        }
        stats = EncodeStats(4096, 6, 3, 2, 1)
        self.assertEqual(stats.to_dict(), expected)
        result = EncodeResult(b"WEM!", stats)
        self.assertEqual(result.stats, stats)
        self.assertEqual(len(result), 4)
        converted = stats.to_dict()
        converted["pcm_frames"] = 100
        self.assertEqual(result.stats.pcm_frames, 4096)
        with self.assertRaises(FrozenInstanceError):
            result.data = b"other"

    def test_result_reports_its_length_and_writes_itself(self):
        stats = EncodeStats(2, 1, 1, 1, 0)
        result = EncodeResult(b"wem!", stats)

        self.assertEqual(len(result), 4)
        with tempfile.TemporaryDirectory() as directory:
            target = Path(directory) / "out.wem"
            result.write_to(target)
            self.assertEqual(target.read_bytes(), result.data)
            # str paths work the same way (PathLike is not required).
            other = Path(directory) / "out-str.wem"
            result.write_to(str(other))
            self.assertEqual(other.read_bytes(), result.data)

    def test_rejects_inconsistent_packet_counts(self):
        with self.assertRaisesRegex(ValueError, "packet counts"):
            EncodeStats(1, 1, 2, 1, 0)
        stats = EncodeStats(1, 1, 1, 1, 0)
        self.assertEqual(stats.audio_packets, 1)
        # No byte count is cross-checked any more: the statistics carry none,
        # and the container's length is whatever `data` already is.
        self.assertEqual(len(EncodeResult(b"bad", stats)), 3)


class WwiseWemErrorTests(unittest.TestCase):
    def test_carries_the_kernel_code_and_the_kernel_message(self):
        error = WwiseWemError("GEOMETRY_MISMATCH", "PCM geometry differs")

        self.assertEqual(error.code, "GEOMETRY_MISMATCH")
        self.assertEqual(error.message, "PCM geometry differs")
        self.assertEqual(
            str(error),
            "PCM geometry differs",
            "str() must stay the kernel's diagnostic text",
        )
        self.assertIsInstance(error, ValueError)

    def test_error_values_survive_a_pickle_round_trip(self):
        error = WwiseWemError("INTERNAL", "encoder fault: analysis: bad curve")
        restored = pickle.loads(pickle.dumps(error))

        self.assertEqual(
            (restored.code, restored.message, str(restored)),
            (error.code, error.message, str(error)),
        )


if __name__ == "__main__":
    unittest.main()
