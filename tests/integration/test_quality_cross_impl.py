"""Cross-implementation quality semantics: Python facade vs native kernel.

Proves the Python/Rust quality mechanism stays aligned end to end:

* the shared pinned test vectors (the same doubles the Rust kernel pins in
  ``crates/wem-profiles`` tests) are reproduced by the Python reference;
* a quality value bound to a profile is forwarded from the Python facade
  into the native kernel's analysis assembly (the 6ch profile carries no
  quality-curves resource, so a quality request must surface the kernel's
  configuration error — with a stale or bypassed kernel the encode would
  silently ignore the quality);
* with no quality the facade keeps the reference bytes (compared byte-for-byte
  against ``tests/fixtures/reference.wem``);
* a bound quality is an additive copy: the resolved profile is never
  mutated.
"""
from __future__ import annotations

import unittest
from pathlib import Path

from wwise_wem import WwiseProfile, WwiseVersion
from wwise_wem.adapters.wav import read_pcm_wav
from wwise_wem.application.encoder import Encoder
from wwise_wem_reference.profiles.artifact import resolve_selection
from wwise_wem_reference.profiles.quality import (
    QUALITY_NORMALIZE_CLAMP,
    _linear_frac,
    normalize_quality_factor,
)

_SELECTION = WwiseProfile(WwiseVersion.WWISE2013, 6, 44100)
_FIXTURES = Path(__file__).resolve().parents[2] / "tests" / "fixtures"
_FIXTURE_WAV = _FIXTURES / "input.wav"
_REFERENCE_WEM = _FIXTURES / "reference.wem"


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
        pcm = read_pcm_wav(_FIXTURE_WAV)
        encoder = Encoder(_SELECTION, quality=4.0)
        with self.assertRaisesRegex(ValueError, "quality-curves"):
            encoder.encode_pcm(pcm)

    def test_facade_quality_none_keeps_the_reference_bytes(self):
        pcm = read_pcm_wav(_FIXTURE_WAV)
        result = Encoder(_SELECTION).encode_pcm(pcm)
        self.assertEqual(result.data, _REFERENCE_WEM.read_bytes())

    def test_selection_quality_copy_never_mutates_the_resolved_profile(self):
        base = resolve_selection(_SELECTION)
        bound = resolve_selection(_SELECTION, quality=9.0)
        self.assertEqual(bound.quality, 9.0)
        self.assertIsNone(base.quality)
        self.assertIs(resolve_selection(_SELECTION), base)
        self.assertEqual(resolve_selection(_SELECTION, quality=3.0).quality, 3.0)


if __name__ == "__main__":
    unittest.main()
