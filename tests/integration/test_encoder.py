"""Facade encoder tests: ownership and validation invariants.

The facade's byte-producing path is the native kernel
(``wwise_wem._core``); byte-exact behavior is covered by the golden,
core-oracle parity, and extended-input suites.  This file keeps the
facade-level invariants that hold before any kernel work: selection
ownership, input validation order, and an unsatisfiable selection.
"""

from __future__ import annotations

import unittest

from wwise_wem import WwiseProfile, WwiseVersion
from wwise_wem.application.encoder import Encoder
from wwise_wem.model import PcmBuffer


SELECTION = WwiseProfile(WwiseVersion.WWISE2013, 6, 44100)
UNINSTALLED_SELECTION = WwiseProfile(WwiseVersion.WWISE2013, 2, 44100)


def _pcm(*, channels: int = 6, rate: int = 44100, frames: int = 4096) -> PcmBuffer:
    # In-domain signed-16 floats: the value / 32768.0 domain that
    # read_pcm_wav produces (the public encoder domain).
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
    def test_constructor_owns_the_selection(self) -> None:
        encoder = Encoder(SELECTION)

        self.assertIs(encoder.selection, SELECTION)

    def test_constructor_rejects_a_non_selection(self) -> None:
        # The literal IS the subject here: a profile *name* is not a selection,
        # and the facade must refuse it rather than resolve it.
        with self.assertRaisesRegex(TypeError, "selection must be WwiseProfile"):
            Encoder("wwise2013-6ch-44100")  # type: ignore[arg-type]

    def test_geometry_and_input_type_fail_before_encode(self) -> None:
        encoder = Encoder(SELECTION)
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
        encoder = Encoder(SELECTION)
        pcm = PcmBuffer(
            44100,
            tuple(tuple(4095.0 for _ in range(4096)) for _ in range(6)),
        )
        with self.assertRaisesRegex(ValueError, "integer signed-16 sample"):
            encoder.encode_pcm(pcm)

    def test_unsatisfiable_selection_fails_before_any_bytes(self) -> None:
        # The kernel owns profile resolution: a selection no installed
        # configuration satisfies is a plain ValueError before any encode.
        encoder = Encoder(UNINSTALLED_SELECTION)
        with self.assertRaisesRegex(ValueError, "2ch/44100Hz"):
            encoder.encode_pcm(_pcm(channels=2, rate=44100))


if __name__ == "__main__":
    unittest.main()
