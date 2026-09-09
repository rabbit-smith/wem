"""2ch/48000 profile runtime behavior.

The 2ch/48000 profile now ships its setup packet, codebooks, and the full
psychoacoustic calibration (registered from the reverse-engineered paired-build
sample), so it is fully ready: building its encoder through the Rust
kernel succeeds instead of being refused.
"""
from __future__ import annotations

import unittest

import wwise_wem._core as core_module

_T2CH_PROFILE_NAME = "wwise2013-2ch-48000"


class TwoChannelProfileRuntimeTests(unittest.TestCase):
    def test_2ch_profile_encodes_once_analysis_is_ready(self):
        # The 2ch/48000 profile is registered with its setup, codebooks, and
        # psychoacoustic analysis resources; building its encoder must now
        # succeed.
        enc = core_module.Encoder(_T2CH_PROFILE_NAME)
        self.assertIsNotNone(enc)
        # Unknown profiles still fail as PROFILE_NOT_FOUND.
        with self.assertRaises(core_module.WemEncoderError) as unknown:
            core_module.Encoder("not-a-profile")
        self.assertEqual(unknown.exception.code, "PROFILE_NOT_FOUND")


if __name__ == "__main__":
    unittest.main()
