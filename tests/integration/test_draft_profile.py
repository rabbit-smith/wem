"""Draft profile (2ch/48000, setup pending corpus export) runtime behavior.

The draft profile registers in the native kernel with its static data; every
encode path refuses it with the clear pending error instead of silently
forging a setup packet.
"""
from __future__ import annotations

import unittest

import wwise_wem._core as core_module

_DRAFT_PROFILE_NAME = "wwise2013-2ch-48000"


class DraftProfileRuntimeTests(unittest.TestCase):
    def test_native_kernel_draft_profile_refuses_to_encode(self):
        # The draft 2ch/48000 profile is registered in the native kernel
        # (setup pending corpus export); building its encoder must fail with
        # the clear pending error, not a profile-not-found.
        with self.assertRaises(core_module.WemEncoderError) as ctx:
            core_module.Encoder(_DRAFT_PROFILE_NAME)
        self.assertEqual(ctx.exception.code, "STATE_ERROR")
        message = str(ctx.exception)
        self.assertIn("setup packet pending", message)
        self.assertIn("requires paired Wwise export", message)
        # Unknown profiles still fail as PROFILE_NOT_FOUND.
        with self.assertRaises(core_module.WemEncoderError) as unknown:
            core_module.Encoder("not-a-profile")
        self.assertEqual(unknown.exception.code, "PROFILE_NOT_FOUND")


if __name__ == "__main__":
    unittest.main()
