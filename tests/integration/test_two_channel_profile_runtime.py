"""2ch/48000 profile runtime behavior."""
from __future__ import annotations

import unittest

import wwise_wem._core as core_module

_SELECTION = core_module.WwiseProfile(core_module.WwiseVersion.WWISE2013, 2, 48000)
_UNINSTALLED_SELECTION = core_module.WwiseProfile(
    core_module.WwiseVersion.WWISE2013, 2, 44100
)


class TwoChannelProfileRuntimeTests(unittest.TestCase):
    def test_2ch_selection_constructs_the_native_encoder(self):
        enc = core_module.Encoder(_SELECTION)
        self.assertIsNotNone(enc)
        # A selection no installed configuration satisfies still fails as
        # PROFILE_NOT_FOUND.
        with self.assertRaises(core_module.WemEncoderError) as unknown:
            core_module.Encoder(_UNINSTALLED_SELECTION)
        self.assertEqual(unknown.exception.code, "PROFILE_NOT_FOUND")


if __name__ == "__main__":
    unittest.main()
