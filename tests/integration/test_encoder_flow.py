"""How a caller's selection reaches the kernel.

The facade owns the selection and validates geometry, frame count and the
sample domain before any bytes exist; the free ``encode`` entry derives the
selection from the input when the caller names none; the 2ch configuration
resolves to a native encoder while an uninstalled geometry is refused. Three
files about that one call path. Each class keeps its own test names and
failure messages.
"""

from __future__ import annotations

import unittest
import wwise_wem._core as core_module

from tests.analysis_resource_support import installed_analysis_resources
from unittest.mock import patch
from wwise_wem import WwiseProfile, WwiseVersion, encode
from wwise_wem.application.encoder import Encoder
from wwise_wem.application.models import EncodeResult, EncodeStats
from wwise_wem.model import PcmBuffer
from wwise_wem_reference.analysis.preprocessing.detector_input import (
    iter_detector_quanta,
)
from wwise_wem_reference.analysis.session import AnalysisSession

# --------------------------------------------------------------------------
# merged from tests/integration/test_encoder.py
# --------------------------------------------------------------------------

# Facade encoder tests: ownership and validation invariants.
#
# The facade's byte-producing path is the native kernel
# (``wwise_wem._core``); byte-exact behavior is covered by the reference,
# core-oracle parity, and extended-input suites.  This file keeps the
# facade-level invariants that hold before any kernel work: selection
# ownership, input validation order, and an unsatisfiable selection.

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


# --------------------------------------------------------------------------
# merged from tests/integration/test_profile_runtime_flow.py
# --------------------------------------------------------------------------

SELECTION = WwiseProfile(WwiseVersion.WWISE2013, 6, 44100)


class ProfileRuntimeFlowTests(unittest.TestCase):
    def test_explicit_selection_reaches_the_kernel_unresolved(self):
        # Single authority: an explicit selection goes to the kernel as it
        # is.  The oracle-side selection resolver is test/tooling only and
        # must not be consulted on the encoding path.
        payload = b"\0\0" * 6
        captured = []

        class FakeEncoder:
            def __init__(self, selection, *, quality=None):
                captured.append((selection, quality))

            def encode_pcm16_interleaved(self, data, *, sample_rate, channels):
                self.input = (data, sample_rate, channels)
                return EncodeResult(
                    b"wem",
                    EncodeStats(1, 6, 0, 0, 0),
                )

        with (
            patch(
                "wwise_wem.adapters.wav._read_wav_pcm16_bytes",
                return_value=(44100, 6, payload),
            ),
            patch("wwise_wem_reference.profiles.artifact.resolve_selection") as resolve,
            patch("wwise_wem.application.encoder.Encoder", FakeEncoder),
        ):
            result = encode("input.wav", profile=SELECTION, quality=3.0)

        resolve.assert_not_called()
        self.assertEqual(captured, [(SELECTION, 3.0)])
        self.assertEqual(result.data, b"wem")
        self.assertEqual(len(result), 3)

    def test_automatic_selection_is_the_input_geometry(self):
        # No explicit profile: the installed generation plus the geometry
        # read from the input is the whole selection.
        payload = b"\0\0" * 6
        captured = []

        class FakeEncoder:
            def __init__(self, selection, *, quality=None):
                captured.append((selection, quality))

            def encode_pcm16_interleaved(self, data, *, sample_rate, channels):
                self.input = (data, sample_rate, channels)
                return EncodeResult(
                    b"wem",
                    EncodeStats(1, 6, 0, 0, 0),
                )

        with (
            patch(
                "wwise_wem.adapters.wav._read_wav_pcm16_bytes",
                return_value=(44100, 6, payload),
            ),
            patch("wwise_wem.application.encoder.Encoder", FakeEncoder),
        ):
            result = encode("input.wav")

        self.assertEqual(
            captured,
            [(WwiseProfile(WwiseVersion.DEFAULT, 6, 44100), None)],
        )
        self.assertEqual(result.data, b"wem")

    def test_detector_quanta_forwards_explicit_blocksizes(self):
        streams = ((0.0,) * 128,)
        with patch(
            "wwise_wem_reference.analysis.preprocessing.detector_input.detector_pcm_streams",
            return_value=streams,
        ) as detector:
            rows = tuple(
                iter_detector_quanta(
                    [[0.0] * 4096],
                    count=1,
                    blocksizes=(256, 2048),
                )
            )
        self.assertEqual(rows, (streams,))
        detector.assert_called_once_with(
            [[0.0] * 4096],
            terminal_samples=8192,
            tail_training=None,
            blocksizes=(256, 2048),
        )

    def test_stream_forwards_its_blocksizes_to_detector(self):
        stream = AnalysisSession(
            1,
            sample_rate=44100,
            blocksizes=(256, 2048),
            resources=installed_analysis_resources(),
        )
        pcm = [[0.0] * 4096]
        with patch(
            "wwise_wem_reference.analysis.session.detector_pcm_streams",
            return_value=((0.0,) * 128,),
        ) as detector:
            modes = stream.select_modes(pcm)
        self.assertTrue(modes)
        self.assertEqual(detector.call_count, 2)
        for call in detector.call_args_list:
            self.assertEqual(call.kwargs["blocksizes"], (256, 2048))

    def test_unsupported_block_geometry_is_rejected_at_session_creation(self):
        with self.assertRaisesRegex(ValueError, "256/2048"):
            AnalysisSession(
                1,
                sample_rate=44100,
                blocksizes=(128, 1024),
                resources=installed_analysis_resources(),
            )


# --------------------------------------------------------------------------
# merged from tests/integration/test_two_channel_profile_runtime.py
# --------------------------------------------------------------------------

# 2ch/48000 profile runtime behavior.

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
