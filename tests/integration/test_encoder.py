"""Facade encoder tests: ownership and validation invariants.

The facade's byte-producing path is the native kernel
(``wwise_wem._core``); byte-exact behavior is covered by the golden,
core-oracle parity, and extended-input suites.  This file keeps the
facade-level invariants that hold before any kernel work: profile
ownership, input validation order, and the installed-profile bundle
check.
"""

from __future__ import annotations

import unittest
from dataclasses import replace

from wwise_wem.application.encoder import Encoder
from wwise_wem.model import PcmBuffer
from wwise_wem.profiles.registry import load_wem_profile


PROFILE_NAME = "wwise2013-6ch-44100"


def _pcm(*, channels: int = 6, rate: int = 44100, frames: int = 4096) -> PcmBuffer:
    # In-domain signed-16 floats: the value / 32768.0 domain that
    # read_pcm16 produces (the public encoder domain).
    return PcmBuffer(
        rate,
        tuple(
            tuple(
                ((frame * 7 + channel * 11 + 3) % 64536 - 32768) / 32768.0
                for frame in range(frames)
            )
            for channel in range(channels)
        ),
    )


class EncoderFacadeTests(unittest.TestCase):
    def setUp(self) -> None:
        self.profile = load_wem_profile(PROFILE_NAME)

    def test_constructor_owns_profile(self) -> None:
        encoder = Encoder(self.profile)

        self.assertIs(encoder.profile, self.profile)

    def test_geometry_and_input_type_fail_before_encode(self) -> None:
        encoder = Encoder(self.profile)
        with self.assertRaisesRegex(TypeError, "PcmBuffer"):
            encoder.encode_pcm("pcm")  # type: ignore[arg-type]
        with self.assertRaisesRegex(ValueError, "differs from encoder profile"):
            encoder.encode_pcm(_pcm(channels=2))
        with self.assertRaisesRegex(ValueError, "differs from encoder profile"):
            encoder.encode_pcm(_pcm(rate=48000))
        with self.assertRaisesRegex(ValueError, "at least 4096 frames"):
            encoder.encode_pcm(_pcm(frames=3))

    def test_out_of_domain_samples_fail_before_encode(self) -> None:
        # Out-of-domain floats are rejected by the facade itself, as a
        # plain ValueError, before the kernel is reached.
        encoder = Encoder(self.profile)
        pcm = PcmBuffer(
            44100,
            tuple(tuple(4095.0 for _ in range(4096)) for _ in range(6)),
        )
        with self.assertRaisesRegex(ValueError, "integer signed-16 sample"):
            encoder.encode_pcm(pcm)

    def test_unknown_runtime_bundle_fails_before_setup_parse(self) -> None:
        profile = replace(
            self.profile,
            setup_sha256="0" * 64,
            key=replace(
                self.profile.key,
                quality_setup_identity="sha256:" + "0" * 64,
            ),
        )
        with self.assertRaisesRegex(ValueError, "differs from installed profile"):
            Encoder(profile)


if __name__ == "__main__":
    unittest.main()
