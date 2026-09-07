"""The facade's execution path is the native kernel — byte-for-byte.

Executable definition of the single execution path: the facade's
byte-producing calls (``Encoder.encode_pcm``, ``encode_wav``) run on the
in-package native extension ``wwise_wem._core`` and must produce
byte-identical WEM output to the raw kernel binding and to the reference
oracle (``wwise_wem_reference``, imported directly as a test asset — it
is a fixture, not an engine).  Error paths (short input, geometry
mismatch) and the rejection conditions of the signed-16 sample domain are
facade-level invariants, raised as plain ``ValueError`` before the kernel
is reached.

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
from wwise_wem.application.models import EncodeResult, EncodeStats
from wwise_wem.model import PcmBuffer

ROOT = Path(__file__).resolve().parents[2]
FIXTURES = ROOT / "tests" / "fixtures"
INPUT = FIXTURES / "input.wav"
REFERENCE = FIXTURES / "reference.wem"
PROFILE_NAME = "wwise2013-6ch-44100"
EXPECTED_SHA256 = (
    "17851d26c6210b85e498ae0452d2562d7b9e2c3e9e795c459656b9c9d8d35247"
)
EXPECTED_STATS = {
    "pcm_frames": 139398,
    "channels": 6,
    "audio_packets": 205,
    "short_packets": 77,
    "long_packets": 128,
    "bytes": 108771,
    "metadata_source": f"profile:{PROFILE_NAME}",
}


def _rows_from_pcm(pcm: PcmBuffer) -> list[list[int]]:
    """Kernel-form rows for in-domain PCM (read_pcm16 output is in-domain)."""
    return [[int(sample * 32768.0) for sample in row] for row in pcm.channels]


class FacadeIsCoreTests(unittest.TestCase):
    """The facade is the core: one execution path, identical bytes."""

    def test_facade_bytes_match_the_raw_core_binding_and_golden(self):
        pcm = read_pcm16(INPUT)
        profile = load_wem_profile(PROFILE_NAME)
        reference = REFERENCE.read_bytes()

        result = Encoder(profile).encode_pcm(pcm)
        direct = core_module.Encoder(PROFILE_NAME).encode_pcm(
            pcm.sample_rate, _rows_from_pcm(pcm)
        )

        self.assertEqual(result.data, reference)
        self.assertEqual(
            hashlib.sha256(result.data).hexdigest(), EXPECTED_SHA256
        )
        self.assertEqual(result.sha256, EXPECTED_SHA256)
        self.assertEqual(result.stats.to_legacy_dict(), EXPECTED_STATS)
        self.assertEqual(bytes(direct.data), reference)
        self.assertEqual(direct.sha256(), EXPECTED_SHA256)
        # The facade output is what the in-package core binding produces:
        # there is no second path that could diverge.
        self.assertEqual(result.sha256, direct.sha256())

    def test_facade_binds_the_in_package_core_module(self):
        # The facade's execution-path import IS the in-package extension;
        # lock the binding so a stray fallback module cannot sneak in.
        from wwise_wem.application import encoder as encoder_module

        self.assertIs(encoder_module._core, core_module)
        self.assertEqual(core_module.__name__, "wwise_wem._core")

    def test_stats_surface_has_no_provenance_field(self):
        # The single execution path needs no provenance tag: the result
        # stats expose only encoding facts.
        profile = load_wem_profile(PROFILE_NAME)
        result = Encoder(profile).encode_pcm(read_pcm16(INPUT))
        self.assertIsInstance(result, EncodeResult)
        self.assertIsInstance(result.stats, EncodeStats)
        self.assertFalse(hasattr(result.stats, "engine"))
        self.assertNotIn("engine", result.stats.to_legacy_dict())

    def test_error_paths_are_facade_invariants(self):
        profile = load_wem_profile(PROFILE_NAME)
        short = PcmBuffer(
            44100,
            tuple(tuple(0.0 for _ in range(100)) for _ in range(6)),
        )
        geometry = PcmBuffer(
            44100,
            tuple(tuple(0.0 for _ in range(4096)) for _ in range(2)),
        )

        with self.assertRaisesRegex(ValueError, "at least 4096 frames"):
            Encoder(profile).encode_pcm(short)
        with self.assertRaisesRegex(
            ValueError, "differs from encoder profile"
        ):
            Encoder(profile).encode_pcm(geometry)

    def test_out_of_domain_pcm_raises_plain_value_error(self):
        profile = load_wem_profile(PROFILE_NAME)
        # 4095.0 * 32768 is outside the signed-16 range: out of domain.
        pcm = PcmBuffer(
            44100,
            tuple(tuple(4095.0 for _ in range(4096)) for _ in range(6)),
        )
        with self.assertRaisesRegex(ValueError, "integer signed-16 sample"):
            Encoder(profile).encode_pcm(pcm)
        # A fractional float is out of domain the same way.
        pcm = PcmBuffer(
            44100,
            tuple(tuple(0.1 for _ in range(4096)) for _ in range(6)),
        )
        with self.assertRaisesRegex(ValueError, "integer signed-16 sample"):
            Encoder(profile).encode_pcm(pcm)


if __name__ == "__main__":
    unittest.main()
