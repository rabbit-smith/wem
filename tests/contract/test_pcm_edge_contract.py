"""Deterministic PCM boundary contracts for the encoder input adapter."""

from __future__ import annotations

import struct
import tempfile
import unittest
import wave
from pathlib import Path
from typing import ClassVar, TypedDict

from wwise_wem import EncodeResult, WwiseProfile, WwiseVersion, encode
from wwise_wem_reference.container.wem import load_wem_parts_bytes
from wwise_wem.adapters.wav import read_pcm_wav
from wwise_wem.model import PcmBuffer
from wwise_wem.profiles.registry import resolve_selection
from wwise_wem_reference.python_engine import ContainerPlan, encode_pcm_python
from tests.contract.wem_byte_contract import assert_wem_equal


CHANNELS = 6
SAMPLE_RATE = 44100


def _sample(frame: int, channel: int) -> int:
    """Integer-only signal with deterministic transients at 1024-frame edges."""
    if frame % 1024 < 32:
        return ((frame * 1103 + channel * 7919 + 12345) & 0xFFFF) - 32768
    return ((frame * 37 + channel * 251) & 0x7FF) - 1024


def _write_pcm16(path: Path, frames: int) -> None:
    values = [_sample(frame, channel) for frame in range(frames) for channel in range(CHANNELS)]
    with wave.open(str(path), "wb") as target:
        target.setnchannels(CHANNELS)
        target.setsampwidth(2)
        target.setframerate(SAMPLE_RATE)
        target.writeframes(struct.pack(f"<{len(values)}h", *values))


class _Case(TypedDict):
    """Locked expectations for one frame count."""

    audio_packets: int
    short_packets: int
    long_packets: int
    bytes: int


class PcmLengthEncodeContractTests(unittest.TestCase):
    # NOTE (the round, 2026): ``nAvgBytesPerSec`` is derived, not carried from the
    # profile -- ``floor(data_payload_bytes * nSamplesPerSec /
    # dwTotalPCMFrames)`` (see ``container.packets.recompute_vorbis_fmt_sizes``).
    # The profile constant 34381 is only correct for the 139398-frame golden
    # fixture, whose whole-file byte comparison pins that field; every other
    # input length must derive its own value.
    #
    # NOTE (2026): the packet counts below were re-locked when the
    # mode-selection tail rule was corrected. The paired build emits a frame
    # while the *previous* frame's center still lies inside the PCM, so the plan
    # ends exactly on the source length; the previous ``center < source_len +
    # prefix`` bound overshot by a fixed amount and emitted trailing frames the
    # build does not. Verified mechanically on all three lengths: the new
    # stream's packets are an exact prefix of the old stream's (2, 1 and 2
    # trailing packets removed respectively), every other packet is
    # byte-identical, and the new plan's last frame starts exactly at the source
    # length (4096 -> 4096, 8192 -> 8192). That end-on-the-source-length
    # signature is what all six measured reference streams show (both conversion
    # routes).
    #
    # EOS prediction trains on the final long block (2048 samples), matching the
    # public Vorbis analysis algorithm and the paired build's predicted samples;
    # the terminal overlap excess derived from the emitted mode sequence is
    # carried in fmt fields 0x24 and 0x32 (that header correction leaves packet
    # bytes and lengths unchanged).
    #
    # The payloads at these lengths have no committed counterpart, so the
    # whole-file claim is the byte comparison against the reference oracle below
    # (plus the mode sequence and byte length pinned here) rather than a digest
    # of this implementation's own output.
    EXPECTED: dict[int, _Case] = {
        4096: {
            "audio_packets": 33,
            "short_packets": 33,
            "long_packets": 0,
            "bytes": 10793,
        },
        4097: {
            "audio_packets": 34,
            "short_packets": 34,
            "long_packets": 0,
            "bytes": 11148,
        },
        8192: {
            "audio_packets": 65,
            "short_packets": 65,
            "long_packets": 0,
            "bytes": 21144,
        },
    }

    _directory: ClassVar[tempfile.TemporaryDirectory]
    results: ClassVar[dict[int, EncodeResult]]
    oracle: ClassVar[dict[int, EncodeResult]]

    @classmethod
    def setUpClass(cls) -> None:
        cls._directory = tempfile.TemporaryDirectory()
        cls.results = {}
        cls.oracle = {}
        selection = WwiseProfile(WwiseVersion.WWISE2013, CHANNELS, SAMPLE_RATE)
        profile = resolve_selection(selection)
        container = ContainerPlan.from_profile(profile)
        for frame_count in cls.EXPECTED:
            path = Path(cls._directory.name) / f"edge-{frame_count}.wav"
            _write_pcm16(path, frame_count)
            cls.results[frame_count] = encode(path)
            cls.oracle[frame_count] = encode_pcm_python(
                profile=profile,
                container=container,
                pcm=read_pcm_wav(path),
            )

    @classmethod
    def tearDownClass(cls) -> None:
        cls._directory.cleanup()

    def test_frame_boundaries_lock_modes_packet_counts_and_oracle_bytes(self) -> None:
        for frame_count, expected in self.EXPECTED.items():
            with self.subTest(frame_count=frame_count):
                result = self.results[frame_count]
                stats = result.stats
                self.assertEqual(stats.pcm_frames, frame_count)
                self.assertEqual(stats.channels, CHANNELS)
                self.assertEqual(stats.metadata_source, "profile:6ch/44100Hz/2013")
                for field in ("audio_packets", "short_packets", "long_packets", "bytes"):
                    self.assertEqual(getattr(stats, field), expected[field])
                self.assertEqual(len(result.data), expected["bytes"])

                audio_packets = load_wem_parts_bytes(result.data)["packets"][1:]
                modes = tuple(packet[0] & 1 for packet in audio_packets)
                self.assertEqual(len(modes), expected["audio_packets"])
                self.assertEqual(
                    modes, (0,) * expected["short_packets"] + (1,) * expected["long_packets"]
                )

                assert_wem_equal(
                    self,
                    bytes(self.oracle[frame_count].data),
                    bytes(result.data),
                    f"{frame_count} frames: Python oracle",
                )


class PcmInputAdapterContractTests(unittest.TestCase):
    def test_read_pcm_wav_preserves_geometry_and_signed_value_edges(self) -> None:
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

            decoded = read_pcm_wav(path)

        self.assertEqual(
            (decoded.sample_rate, decoded.frame_count, decoded.channel_count),
            (SAMPLE_RATE, 2, CHANNELS),
        )
        self.assertEqual(
            decoded.channels,
            (
                (-1.0, 32767 / 32768.0),
                (-1 / 32768.0, 1 / 32768.0),
                (0.0, 0.0),
                (1 / 32768.0, -1 / 32768.0),
                (32767 / 32768.0, -1.0),
                (0.5, -0.5),
            ),
        )

    def test_read_pcm_wav_rejects_zero_frames(self) -> None:
        with tempfile.TemporaryDirectory() as directory:
            path = Path(directory) / "empty.wav"
            with wave.open(str(path), "wb") as target:
                target.setnchannels(CHANNELS)
                target.setsampwidth(2)
                target.setframerate(SAMPLE_RATE)
                target.writeframes(b"")

            with self.assertRaisesRegex(ValueError, "at least one frame"):
                read_pcm_wav(path)

    def test_read_pcm_wav_rejects_unsupported_eight_bit_input(self) -> None:
        with tempfile.TemporaryDirectory() as directory:
            path = Path(directory) / "pcm8.wav"
            with wave.open(str(path), "wb") as target:
                target.setnchannels(CHANNELS)
                target.setsampwidth(1)
                target.setframerate(SAMPLE_RATE)
                target.writeframes(b"\x80" * CHANNELS)

            with self.assertRaisesRegex(
                ValueError, "16-bit PCM, 24-bit PCM"
            ):
                read_pcm_wav(path)


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
        source: list[list[float]] = [[-1.0, 0.0, 1.0], [-0.5, 0.5, 32767 / 32768]]
        pcm = PcmBuffer(SAMPLE_RATE, tuple(tuple(row) for row in source))
        source[0][0] = 99.0

        self.assertEqual(
            pcm.channels,
            ((-1.0, 0.0, 1.0), (-0.5, 0.5, 32767 / 32768.0)),
        )
        self.assertEqual((pcm.channel_count, pcm.frame_count), (2, 3))


if __name__ == "__main__":
    unittest.main()
