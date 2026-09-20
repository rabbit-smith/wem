from __future__ import annotations

from tests.analysis_resource_support import installed_analysis_resources

import unittest
from dataclasses import replace
from unittest.mock import patch

from wwise_wem import encode
from wwise_wem.application.encoder import Encoder
from wwise_wem.application.models import EncodeResult, EncodeStats
from wwise_wem_reference.analysis.preprocessing.detector_input import iter_detector_quanta
from wwise_wem_reference.analysis.session import AnalysisSession
from wwise_wem.profiles.registry import load_wem_profile


class ProfileRuntimeFlowTests(unittest.TestCase):
    def test_selected_profile_manifest_and_blocks_reach_stream(self):
        selected = load_wem_profile("wwise2013-6ch-44100")
        payload = b"\0\0" * 6
        captured = []

        class FakeEncoder:
            def __init__(self, profile):
                captured.append(profile)

            def encode_pcm16_interleaved(self, data, *, sample_rate, channels):
                self.input = (data, sample_rate, channels)
                return EncodeResult(
                    b"wem",
                    EncodeStats(1, 6, 0, 0, 0, 3, f"profile:{selected.name}"),
                )

        with (
            patch(
                "wwise_wem.adapters.wav._read_wav_pcm16_bytes",
                return_value=(44100, 6, payload),
            ),
            patch(
                "wwise_wem.profiles.registry.load_wem_profile",
                return_value=selected,
            ) as load,
            patch("wwise_wem.application.encoder.Encoder", FakeEncoder),
        ):
            result = encode("input.wav", profile="test-profile")

        load.assert_called_once_with("test-profile", quality=None)
        self.assertEqual(captured, [selected])
        self.assertEqual(result.data, b"wem")
        self.assertEqual(result.stats.metadata_source, f"profile:{selected.name}")

    def test_manifest_failure_precedes_setup_and_analysis(self):
        profile = load_wem_profile("wwise2013-6ch-44100")
        with patch(
            "wwise_wem.application.encoder.load_profile_bundle",
            side_effect=ValueError("runtime manifest checksum differs"),
        ):
            with self.assertRaisesRegex(ValueError, "runtime manifest checksum"):
                Encoder(profile)

    def test_unknown_runtime_bundle_is_rejected_before_setup(self):
        profile = replace(
            load_wem_profile("wwise2013-6ch-44100"),
            setup_sha256="0" * 64,
            key=replace(
                load_wem_profile("wwise2013-6ch-44100").key,
                quality_setup_identity="sha256:" + "0" * 64,
            ),
        )
        with self.assertRaisesRegex(ValueError, "differs from installed profile"):
            Encoder(profile)

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

    def test_profile_block_capability_rejection_precedes_setup_parse(self):
        base = load_wem_profile("wwise2013-6ch-44100")
        metadata = replace(
            base.container_metadata,
            uBlocksize0Pow=7,
            uBlocksize1Pow=10,
        )
        profile = replace(base, block_sizes=(128, 1024), container_metadata=metadata)
        with patch("wwise_wem.application.encoder.load_profile_bundle") as load_bundle:
            with self.assertRaisesRegex(ValueError, "supports 256/2048"):
                Encoder(profile)
        load_bundle.assert_not_called()


if __name__ == "__main__":
    unittest.main()
