"""Deterministic PCM boundary contracts for the encoder input adapter."""

from __future__ import annotations

import hashlib
import struct
import tempfile
import unittest
import wave
from pathlib import Path

from wwise2013_wem.container.wem import load_wem_parts_bytes
from wwise2013_wem.application.compat import encode_wav_to_wem, read_pcm16_wav
from wwise2013_wem.model import PcmBuffer


CHANNELS = 6
SAMPLE_RATE = 44100


def _sample(frame: int, channel: int) -> int:
    """Integer-only signal with deterministic transients at 1024-frame edges."""
    if frame % 1024 < 32:
        return ((frame * 1103 + channel * 7919 + 12345) & 0xFFFF) - 32768
    return ((frame * 37 + channel * 251) & 0x7FF) - 1024


def _write_pcm16(path: Path, frames: int) -> None:
    values = [
        _sample(frame, channel)
        for frame in range(frames)
        for channel in range(CHANNELS)
    ]
    with wave.open(str(path), "wb") as target:
        target.setnchannels(CHANNELS)
        target.setsampwidth(2)
        target.setframerate(SAMPLE_RATE)
        target.writeframes(struct.pack(f"<{len(values)}h", *values))


class PcmLengthEncodeContractTests(unittest.TestCase):
    EXPECTED = {
        4096: {
            "audio_packets": 35,
            "short_packets": 34,
            "long_packets": 1,
            "bytes": 11002,
            "sha256": "95dd9c2ef83b28c0c36868b75a9f5c9a82a34d9e12ae502ea1a5c096b572cd56",
        },
        4097: {
            "audio_packets": 35,
            "short_packets": 34,
            "long_packets": 1,
            "bytes": 11171,
            "sha256": "9800cfb68948783333a3389c0b9734df9862bcc0275dc1185c0ba37325315e47",
        },
        8192: {
            "audio_packets": 67,
            "short_packets": 66,
            "long_packets": 1,
            "bytes": 21353,
            "sha256": "681dc8ce2767bf887ced6059f1ae0f145832159851ebde8f28b3723a237173bf",
        },
    }

    @classmethod
    def setUpClass(cls) -> None:
        cls._directory = tempfile.TemporaryDirectory()
        cls.results: dict[int, tuple[bytes, dict[str, int | str]]] = {}
        for frame_count in cls.EXPECTED:
            path = Path(cls._directory.name) / f"edge-{frame_count}.wav"
            _write_pcm16(path, frame_count)
            cls.results[frame_count] = encode_wav_to_wem(path)

    @classmethod
    def tearDownClass(cls) -> None:
        cls._directory.cleanup()

    def test_frame_boundaries_lock_modes_packet_counts_bytes_and_sha(self) -> None:
        for frame_count, expected in self.EXPECTED.items():
            with self.subTest(frame_count=frame_count):
                encoded, stats = self.results[frame_count]
                self.assertEqual(stats["pcm_frames"], frame_count)
                self.assertEqual(stats["channels"], CHANNELS)
                self.assertEqual(stats["metadata_source"], "profile:wwise2013-6ch-44100")
                for field in ("audio_packets", "short_packets", "long_packets", "bytes"):
                    self.assertEqual(stats[field], expected[field])
                self.assertEqual(len(encoded), expected["bytes"])
                self.assertEqual(hashlib.sha256(encoded).hexdigest(), expected["sha256"])

                audio_packets = load_wem_parts_bytes(encoded)["packets"][1:]
                modes = tuple(packet[0] & 1 for packet in audio_packets)
                self.assertEqual(len(modes), expected["audio_packets"])
                self.assertEqual(modes, (0,) * expected["short_packets"] + (1,))


class PcmInputAdapterContractTests(unittest.TestCase):
    def test_read_pcm16_wav_preserves_geometry_and_signed_value_edges(self) -> None:
        interleaved = [
            -32768,
            -1,
            0,
            1,
            32767,
            16384,
            32767,
            1,
            0,
            -1,
            -32768,
            -16384,
        ]
        with tempfile.TemporaryDirectory() as directory:
            path = Path(directory) / "signed-edges.wav"
            with wave.open(str(path), "wb") as target:
                target.setnchannels(CHANNELS)
                target.setsampwidth(2)
                target.setframerate(SAMPLE_RATE)
                target.writeframes(struct.pack("<12h", *interleaved))

            sample_rate, frames, pcm = read_pcm16_wav(path)

        self.assertEqual((sample_rate, frames, len(pcm)), (SAMPLE_RATE, 2, CHANNELS))
        self.assertEqual(
            pcm,
            [
                [-1.0, 32767 / 32768.0],
                [-1 / 32768.0, 1 / 32768.0],
                [0.0, 0.0],
                [1 / 32768.0, -1 / 32768.0],
                [32767 / 32768.0, -1.0],
                [0.5, -0.5],
            ],
        )

    def test_read_pcm16_wav_returns_empty_channel_rows_for_zero_frames(self) -> None:
        with tempfile.TemporaryDirectory() as directory:
            path = Path(directory) / "empty.wav"
            with wave.open(str(path), "wb") as target:
                target.setnchannels(CHANNELS)
                target.setsampwidth(2)
                target.setframerate(SAMPLE_RATE)
                target.writeframes(b"")

            self.assertEqual(
                read_pcm16_wav(path),
                (SAMPLE_RATE, 0, [[] for _ in range(CHANNELS)]),
            )

    def test_read_pcm16_wav_rejects_non_signed16_input(self) -> None:
        with tempfile.TemporaryDirectory() as directory:
            path = Path(directory) / "pcm8.wav"
            with wave.open(str(path), "wb") as target:
                target.setnchannels(CHANNELS)
                target.setsampwidth(1)
                target.setframerate(SAMPLE_RATE)
                target.writeframes(b"\x80" * CHANNELS)

            with self.assertRaisesRegex(
                ValueError, "encoder input must be uncompressed signed-16 PCM WAV"
            ):
                read_pcm16_wav(path)


class PcmBufferEdgeContractTests(unittest.TestCase):
    def test_empty_and_unequal_channels_keep_existing_errors(self) -> None:
        cases = (
            ((), "PCM buffer needs at least one channel"),
            (((),), "PCM buffer needs at least one frame"),
            (((0.0,), (0.0, 1.0)), "PCM channels must have equal frame counts"),
        )
        for channels, message in cases:
            with self.subTest(channels=channels):
                with self.assertRaisesRegex(ValueError, f"^{message}$"):
                    PcmBuffer(SAMPLE_RATE, channels)

    def test_signed_boundaries_are_normalized_to_immutable_floats(self) -> None:
        source = [[-1, 0, 1], [-0.5, 0.5, 32767 / 32768]]
        pcm = PcmBuffer(SAMPLE_RATE, source)
        source[0][0] = 99

        self.assertEqual(
            pcm.channels,
            ((-1.0, 0.0, 1.0), (-0.5, 0.5, 32767 / 32768.0)),
        )
        self.assertEqual((pcm.channel_count, pcm.frame_count), (2, 3))


if __name__ == "__main__":
    unittest.main()
