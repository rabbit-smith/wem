"""2ch/48000 profile runtime behavior."""
from __future__ import annotations

import unittest

import wwise_wem._core as core_module

_T2CH_PROFILE_NAME = "wwise2013-2ch-48000"


class TwoChannelProfileRuntimeTests(unittest.TestCase):
    def test_2ch_profile_constructs_the_native_encoder(self):
        enc = core_module.Encoder(_T2CH_PROFILE_NAME)
        self.assertIsNotNone(enc)
        # Unknown profiles still fail as PROFILE_NOT_FOUND.
        with self.assertRaises(core_module.WemEncoderError) as unknown:
            core_module.Encoder("not-a-profile")
        self.assertEqual(unknown.exception.code, "PROFILE_NOT_FOUND")


if __name__ == "__main__":
    unittest.main()
