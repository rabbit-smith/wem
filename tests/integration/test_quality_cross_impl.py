"""Cross-implementation quality semantics: Python facade vs native kernel.

Proves the Python/Rust quality mechanism stays aligned end to end:

* the shared pinned test vectors (the same doubles the Rust kernel pins in
  ``crates/wem-profiles`` tests) are reproduced by the Python reference;
* a quality value bound to a profile is forwarded from the Python facade
  into the native kernel's analysis assembly (the 6ch profile carries no
  quality-curves resource, so a quality request must surface the kernel's
  configuration error — with a stale or bypassed kernel the encode would
  silently ignore the quality);
* with no quality the facade keeps the historical golden bytes;
* quality-bound profiles are additive copies: the registry instance is
  never mutated.
"""
from __future__ import annotations

import hashlib
import unittest
from pathlib import Path

from wwise_wem.adapters.wav import read_pcm_wav
from wwise_wem.application.encoder import Encoder
from wwise_wem.profiles.registry import load_wem_profile, resolve_wem_profile
from wwise_wem_reference.profiles.quality import (
    QUALITY_NORMALIZE_CLAMP,
    _linear_frac,
    normalize_quality_factor,
)

_PROFILE_NAME = "wwise2013-6ch-44100"
_GOLDEN_WEM_SHA256 = "17851d26c6210b85e498ae0452d2562d7b9e2c3e9e795c459656b9c9d8d35247"
_FIXTURE_WAV = (
    Path(__file__).resolve().parents[2] / "tests" / "fixtures" / "input.wav"
)


class QualityCrossImplementationTests(unittest.TestCase):
    def test_shared_pinned_vectors_are_implementation_stable(self):
        # The Rust kernel pins these exact values (quality_parity.rs); the
        # Python reference must return identical doubles.
        self.assertEqual(normalize_quality_factor(0.0), 1e-07)
        self.assertEqual(normalize_quality_factor(4.0), 0.4000001)
        self.assertEqual(normalize_quality_factor(10.0), QUALITY_NORMALIZE_CLAMP)
        self.assertEqual(QUALITY_NORMALIZE_CLAMP, 0.9998999834060669)

        bp = (0.5, 0.9)
        samples = (0.0, 1.0)
        self.assertEqual(_linear_frac(bp, samples, 0.7), (0.4999999999999999, False))
        self.assertEqual(_linear_frac(bp, samples, 0.9), (0.999, False))
        self.assertEqual(_linear_frac(bp, samples, 1.0), (0.999, True))

        bp = (0.0, 4.0, 8.0)
        samples = (10.0, 20.0, 30.0)
        self.assertEqual(_linear_frac(bp, samples, 2.0), (15.0, False))
        self.assertEqual(_linear_frac(bp, samples, 8.0), (29.990000000000002, False))
        self.assertEqual(_linear_frac(bp, samples, 9.0), (29.990000000000002, True))

    def test_facade_quality_is_forwarded_to_the_kernel(self):
        # The 6ch profile carries no quality-curves resource; requesting a
        # quality through the facade must therefore surface the kernel's
        # configuration error (proof the quality reaches the kernel's
        # assembly — a kernel that ignores quality would encode instead).
        profile = load_wem_profile(_PROFILE_NAME, quality=4.0)
        self.assertEqual(profile.quality, 4.0)
        pcm = read_pcm_wav(_FIXTURE_WAV)
        encoder = Encoder(profile)
        with self.assertRaisesRegex(ValueError, "quality-curves"):
            encoder.encode_pcm(pcm)

    def test_facade_quality_none_keeps_the_golden_bytes(self):
        profile = load_wem_profile(_PROFILE_NAME)
        self.assertIsNone(profile.quality)
        pcm = read_pcm_wav(_FIXTURE_WAV)
        result = Encoder(profile).encode_pcm(pcm)
        self.assertEqual(
            hashlib.sha256(result.data).hexdigest(), _GOLDEN_WEM_SHA256
        )

    def test_profile_quality_copy_never_mutates_the_registry(self):
        base = load_wem_profile(_PROFILE_NAME)
        bound = load_wem_profile(_PROFILE_NAME, quality=9.0)
        self.assertEqual(bound.quality, 9.0)
        self.assertIsNone(base.quality)
        self.assertIs(load_wem_profile(_PROFILE_NAME), base)
        geometry = resolve_wem_profile(6, 44100, quality=3.0)
        self.assertEqual(geometry.quality, 3.0)
        self.assertIsNone(resolve_wem_profile(6, 44100).quality)


if __name__ == "__main__":
    unittest.main()
