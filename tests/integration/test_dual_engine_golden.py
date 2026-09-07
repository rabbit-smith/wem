"""Dual-engine golden parity: reference oracle vs native kernel, byte-for-byte.

This is the first-class dual-implementation test form: the same PCM input
must produce byte-identical WEM bytes whether encoded by the reference
oracle (``wwise_wem_reference``, reachable through the pinned Python facade
path and directly through the reference engine module) or by the native
kernel, and the reference-input case must also match the golden WEM file.
Boundary-length PCM inputs (around the short/long frame plan edges) exercise
the same requirement on synthetic streams.

The whole class skips (with an install hint, mirroring the engine parity
precedent) in environments without the native extension.
"""

from __future__ import annotations

import hashlib
import os
import unittest
from contextlib import contextmanager
from pathlib import Path

from wwise_wem import _engine
from wwise_wem import Encoder, load_wem_profile
from wwise_wem.adapters.wav import read_pcm16
from wwise_wem.model import PcmBuffer
from wwise_wem_reference import python_engine
from wwise_wem_reference.container.model import ContainerPlan

try:
    import _wwise_wem_native
except ImportError:
    _wwise_wem_native = None

ROOT = Path(__file__).resolve().parents[2]
FIXTURES = ROOT / "tests" / "fixtures"
INPUT = FIXTURES / "input.wav"
REFERENCE = FIXTURES / "reference.wem"
PROFILE_NAME = "wwise2013-6ch-44100"
EXPECTED_SHA256 = (
    "17851d26c6210b85e498ae0452d2562d7b9e2c3e9e795c459656b9c9d8d35247"
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


@contextmanager
def _pinned_engine(name: str):
    previous = os.environ.get(_engine.ENGINE_ENV_VAR)
    os.environ[_engine.ENGINE_ENV_VAR] = name
    try:
        yield
    finally:
        if previous is None:
            os.environ.pop(_engine.ENGINE_ENV_VAR, None)
        else:
            os.environ[_engine.ENGINE_ENV_VAR] = previous


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


@unittest.skipIf(
    _wwise_wem_native is None,
    "native extension _wwise_wem_native is not installed; "
    "build it with: cd crates/wem-python && maturin develop -F extension-module",
)
class DualEngineGoldenTests(unittest.TestCase):
    def test_reference_input_matches_golden_on_every_engine_path(self):
        pcm = read_pcm16(INPUT)
        profile = load_wem_profile(PROFILE_NAME)
        golden = REFERENCE.read_bytes()

        encoders = {
            "facade-python": Encoder(profile),
            "facade-native": Encoder(profile),
        }

        with _pinned_engine("python"):
            facade_python = encoders["facade-python"].encode_pcm(pcm)
        with _pinned_engine("native"):
            facade_native = encoders["facade-native"].encode_pcm(pcm)
        direct_reference = python_engine.encode_pcm_python(
            profile=profile,
            container=ContainerPlan.from_profile(profile),
            pcm=pcm,
        )
        direct_native = _wwise_wem_native.Encoder(PROFILE_NAME).encode_pcm(
            pcm.sample_rate, _rows_from_pcm(pcm)
        )

        self.assertEqual(facade_python.data, golden)
        self.assertEqual(facade_native.data, golden)
        self.assertEqual(bytes(direct_reference.data), golden)
        self.assertEqual(bytes(direct_native.data), golden)
        self.assertEqual(
            hashlib.sha256(golden).hexdigest(), EXPECTED_SHA256
        )
        self.assertEqual(facade_python.sha256, EXPECTED_SHA256)
        self.assertEqual(facade_native.sha256, EXPECTED_SHA256)
        self.assertEqual(
            hashlib.sha256(direct_reference.data).hexdigest(), EXPECTED_SHA256
        )
        self.assertEqual(direct_native.sha256(), EXPECTED_SHA256)
        self.assertEqual(facade_python.data, facade_native.data)
        self.assertEqual(facade_python.sha256, direct_native.sha256())
        python_stats = {
            key: value
            for key, value in facade_python.stats.to_legacy_dict().items()
            if key != "engine"
        }
        native_stats = {
            key: value
            for key, value in facade_native.stats.to_legacy_dict().items()
            if key != "engine"
        }
        self.assertEqual(python_stats, native_stats)

    def test_boundary_length_inputs_are_byte_identical_across_engines(self):
        profile = load_wem_profile(PROFILE_NAME)
        for frames in BOUNDARY_FRAME_COUNTS:
            with self.subTest(frames=frames):
                pcm = _synthetic_pcm(frames)

                oracle = python_engine.encode_pcm_python(
                    profile=profile,
                    container=ContainerPlan.from_profile(profile),
                    pcm=pcm,
                )
                with _pinned_engine("native"):
                    native = Encoder(profile).encode_pcm(pcm)

                self.assertEqual(oracle.data, native.data)
                self.assertEqual(oracle.sha256, native.sha256)
                self.assertGreater(len(oracle.data), 0)
                self.assertEqual(oracle.stats.pcm_frames, frames)
                oracle_stats = {
                    key: value
                    for key, value in oracle.stats.to_legacy_dict().items()
                    if key != "engine"
                }
                native_stats = {
                    key: value
                    for key, value in native.stats.to_legacy_dict().items()
                    if key != "engine"
                }
                self.assertEqual(oracle_stats, native_stats)

    def test_reference_path_matches_pinned_python_facade_on_boundaries(self):
        profile = load_wem_profile(PROFILE_NAME)
        for frames in (4097, 8191, 12288):
            with self.subTest(frames=frames):
                pcm = _synthetic_pcm(frames)
                with _pinned_engine("python"):
                    facade_python = Encoder(profile).encode_pcm(pcm)
                direct_reference = python_engine.encode_pcm_python(
                    profile=profile,
                    container=ContainerPlan.from_profile(profile),
                    pcm=pcm,
                )
                self.assertEqual(facade_python.data, direct_reference.data)


if __name__ == "__main__":
    unittest.main()
