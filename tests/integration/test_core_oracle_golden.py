"""Core-oracle golden parity: facade (native kernel) vs reference oracle.

The single execution path, executable: the facade's byte-producing calls
run on the in-package native extension (``wwise_wem._core``), and the
reference oracle — the ``wwise_wem_reference`` package, imported
directly as a **test fixture** (a test asset, not an engine: nothing in
the distributed package imports it at runtime) — must produce
byte-identical WEM bytes for the same PCM.  The reference-input case must
also match the golden WEM file: three-way, facade == oracle ==
``reference.wem``.  Boundary-length PCM inputs (around the short/long
frame-plan edges) exercise the same requirement on synthetic streams.

The native extension is a required runtime asset: this suite imports
``wwise_wem._core`` unconditionally and deliberately designs no skip case
for native-absent environments.
"""

from __future__ import annotations

import hashlib
import unittest
from pathlib import Path

from wwise_wem import Encoder, load_wem_profile
from wwise_wem import _core as core_module
from wwise_wem.adapters.wav import read_pcm16
from wwise_wem.model import PcmBuffer
from wwise_wem_reference import python_engine
from wwise_wem_reference.container.model import ContainerPlan

ROOT = Path(__file__).resolve().parents[2]
FIXTURES = ROOT / "tests" / "fixtures"
INPUT = FIXTURES / "input.wav"
REFERENCE = FIXTURES / "reference.wem"
PROFILE_NAME = "wwise2013-6ch-44100"
EXPECTED_SHA256 = (
    "17851d26c6210b85e498ae0452d2562d7b9e2c3e9e795c459656b9c9d8d35247"
)
STEREO_PROFILE_NAME = "wwise2013-2ch-48000"
STEREO_EXPECTED_SHA256 = (
    "4e944dd43000e6738e4399af8851789cd8f90a0004100ff38e8e2b2ed61f3654"
)

# Frame counts probing the frame-plan edges: the minimum length, both sides
# of each short/long boundary, and a long-stream value.
BOUNDARY_FRAME_COUNTS = (
    4096,
    4097,
    8191,
    8192,
    12287,
    12288,
)


def _rows_from_pcm(pcm: PcmBuffer) -> list[list[int]]:
    """Kernel-form rows for in-domain PCM (read_pcm16 output is in-domain)."""
    return [[int(sample * 32768.0) for sample in row] for row in pcm.channels]


def _oracle_encode(pcm: PcmBuffer, profile) -> "object":
    """Direct reference-oracle import: the test-fixture encode entry."""
    return python_engine.encode_pcm_python(
        profile=profile,
        container=ContainerPlan.from_profile(profile),
        pcm=pcm,
    )


def _synthetic_pcm(frames: int) -> PcmBuffer:
    """Deterministic in-domain signed-16 PCM of ``frames`` length."""
    channels = []
    for channel in range(6):
        channels.append(
            tuple(
                ((frame * 7 + channel * 11 + 3) % 64536 - 32768) / 32768.0
                for frame in range(frames)
            )
        )
    return PcmBuffer(44100, tuple(channels))


def _synthetic_stereo_pcm(frames: int = 16384) -> PcmBuffer:
    """Deterministic 2ch input covering short, long, and terminal frames."""
    channels = tuple(
        tuple(
            ((frame * 7 + channel * 11 + 3) % 64536 - 32768) / 32768.0
            for frame in range(frames)
        )
        for channel in range(2)
    )
    return PcmBuffer(48000, channels)


class CoreOracleGoldenTests(unittest.TestCase):
    def test_reference_input_matches_golden_on_facade_and_oracle(self):
        pcm = read_pcm16(INPUT)
        profile = load_wem_profile(PROFILE_NAME)
        golden = REFERENCE.read_bytes()

        # Single execution path: the facade runs the native kernel.
        facade = Encoder(profile).encode_pcm(pcm)
        oracle_result = _oracle_encode(pcm, profile)
        direct_core = core_module.Encoder(PROFILE_NAME).encode_pcm(
            pcm.sample_rate, _rows_from_pcm(pcm)
        )

        self.assertEqual(facade.data, golden)
        self.assertEqual(bytes(oracle_result.data), golden)
        self.assertEqual(bytes(direct_core.data), golden)
        self.assertEqual(hashlib.sha256(golden).hexdigest(), EXPECTED_SHA256)
        self.assertEqual(facade.sha256, EXPECTED_SHA256)
        self.assertEqual(
            hashlib.sha256(oracle_result.data).hexdigest(), EXPECTED_SHA256
        )
        self.assertEqual(direct_core.sha256(), EXPECTED_SHA256)
        self.assertEqual(facade.sha256, direct_core.sha256())
        # One execution path: the stats surface carries no provenance tag.
        self.assertNotIn("engine", facade.stats.to_legacy_dict())
        self.assertEqual(
            facade.stats.to_legacy_dict(),
            oracle_result.stats.to_legacy_dict(),
        )

    def test_boundary_length_inputs_are_byte_identical_between_facade_and_oracle(
        self,
    ):
        profile = load_wem_profile(PROFILE_NAME)
        for frames in BOUNDARY_FRAME_COUNTS:
            with self.subTest(frames=frames):
                pcm = _synthetic_pcm(frames)

                oracle = _oracle_encode(pcm, profile)
                native = Encoder(profile).encode_pcm(pcm)

                self.assertEqual(oracle.data, native.data)
                self.assertEqual(oracle.sha256, native.sha256)
                self.assertGreater(len(oracle.data), 0)
                self.assertEqual(oracle.stats.pcm_frames, frames)
                self.assertEqual(
                    oracle.stats.to_legacy_dict(),
                    native.stats.to_legacy_dict(),
                )

    def test_stereo_profile_has_a_pinned_native_oracle_byte_contract(self):
        profile = load_wem_profile(STEREO_PROFILE_NAME)
        pcm = _synthetic_stereo_pcm()

        oracle = _oracle_encode(pcm, profile)
        native = Encoder(profile).encode_pcm(pcm)
        direct_core = core_module.Encoder(STEREO_PROFILE_NAME).encode_pcm(
            pcm.sample_rate, _rows_from_pcm(pcm)
        )

        self.assertEqual(bytes(native.data), bytes(oracle.data))
        self.assertEqual(bytes(direct_core.data), bytes(oracle.data))
        self.assertEqual(native.sha256, STEREO_EXPECTED_SHA256)
        self.assertEqual(oracle.sha256, STEREO_EXPECTED_SHA256)
        self.assertEqual(direct_core.sha256(), STEREO_EXPECTED_SHA256)
        self.assertEqual(native.stats.audio_packets, 26)
        self.assertEqual(native.stats.short_packets, 10)
        self.assertEqual(native.stats.long_packets, 16)


if __name__ == "__main__":
    unittest.main()
