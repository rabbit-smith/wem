"""2ch/48000 profile runtime behavior.

The 2ch/48000 profile now ships its setup packet and codebooks (structure
registered from the reverse-engineered paired-build sample), so it resolves by
geometry and setup digest. Its psychoacoustic calibration is still pending,
however: every encode path refuses it with the clear analysis-pending error
instead of silently encoding with incomplete psychoacoustics.
"""
from __future__ import annotations

import unittest

import wwise_wem._core as core_module

_T2CH_PROFILE_NAME = "wwise2013-2ch-48000"


class TwoChannelProfileRuntimeTests(unittest.TestCase):
    def test_2ch_profile_refuses_to_encode_until_analysis_ready(self):
        # The 2ch/48000 profile is registered with its setup and codebooks;
        # building its encoder must still fail because its psychoacoustic
        # analysis resources are pending, not because the setup is missing.
        with self.assertRaises(core_module.WemEncoderError) as ctx:
            core_module.Encoder(_T2CH_PROFILE_NAME)
        self.assertEqual(ctx.exception.code, "STATE_ERROR")
        message = str(ctx.exception)
        self.assertIn("analysis", message)
        self.assertIn("psychoacoustic", message)
        self.assertIn("pending", message)
        self.assertIn("encoding unavailable", message)
        # Unknown profiles still fail as PROFILE_NOT_FOUND.
        with self.assertRaises(core_module.WemEncoderError) as unknown:
            core_module.Encoder("not-a-profile")
        self.assertEqual(unknown.exception.code, "PROFILE_NOT_FOUND")


if __name__ == "__main__":
    unittest.main()
