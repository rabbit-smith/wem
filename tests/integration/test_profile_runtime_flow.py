from __future__ import annotations

from tests.analysis_resource_support import installed_analysis_resources

import unittest
from unittest.mock import patch

from wwise_wem import WwiseProfile, WwiseVersion, encode
from wwise_wem.application.models import EncodeResult, EncodeStats
from wwise_wem_reference.analysis.preprocessing.detector_input import iter_detector_quanta
from wwise_wem_reference.analysis.session import AnalysisSession


SELECTION = WwiseProfile(WwiseVersion.WWISE2013, 6, 44100)


class ProfileRuntimeFlowTests(unittest.TestCase):
    def test_explicit_selection_reaches_the_kernel_unresolved(self):
        # Single authority: an explicit selection goes to the kernel as it
        # is.  The package-side selection resolver is oracle/tooling only
        # and must not be consulted on the encoding path.
        payload = b"\0\0" * 6
        captured = []

        class FakeEncoder:
            def __init__(self, selection, *, quality=None):
                captured.append((selection, quality))

            def encode_pcm16_interleaved(self, data, *, sample_rate, channels):
                self.input = (data, sample_rate, channels)
                return EncodeResult(
                    b"wem",
                    EncodeStats(1, 6, 0, 0, 0, 3, "profile:6ch/44100Hz/2013"),
                )

        with (
            patch(
                "wwise_wem.adapters.wav._read_wav_pcm16_bytes",
                return_value=(44100, 6, payload),
            ),
            patch("wwise_wem.profiles.registry.resolve_selection") as resolve,
            patch("wwise_wem.application.encoder.Encoder", FakeEncoder),
        ):
            result = encode("input.wav", profile=SELECTION, quality=3.0)

        resolve.assert_not_called()
        self.assertEqual(captured, [(SELECTION, 3.0)])
        self.assertEqual(result.data, b"wem")
        self.assertEqual(result.stats.metadata_source, "profile:6ch/44100Hz/2013")

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
                    EncodeStats(1, 6, 0, 0, 0, 3, "profile:6ch/44100Hz/2013"),
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


if __name__ == "__main__":
    unittest.main()
