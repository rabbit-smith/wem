from __future__ import annotations

import unittest
from dataclasses import replace
from types import SimpleNamespace
from unittest.mock import patch

from wwise_wem.application.encoder import Encoder
from wwise_wem.application.models import EncodeResult, EncodeStats
from wwise_wem.model import PcmBuffer
from wwise_wem.profiles.model import EncoderProfile
from wwise_wem.profiles.registry import load_wem_profile


PROFILE_NAME = "wwise2013-6ch-44100"
REFERENCE_ENGINE = "wwise_wem_reference.python_engine"


def _pcm(*, channels: int = 6, rate: int = 44100, frames: int = 4096) -> PcmBuffer:
    return PcmBuffer(
        rate,
        tuple(tuple(float(frame) for frame in range(frames)) for _ in range(channels)),
    )


class FakeSession:
    instances: list["FakeSession"] = []

    def __init__(self, channels, *, sample_rate, blocksizes, resources):
        self.arguments = channels, sample_rate, blocksizes
        self.resources = resources
        self.selected_pcm = None
        self.__class__.instances.append(self)

    def selected_windows(self, pcm):
        self.selected_pcm = pcm
        return (0, 1), iter(("short-window", "long-window"))

    def analyze_window(self, window):
        return f"analysis:{window}"


def _pack_analysis(setup, books, analysis, *, channels):
    return SimpleNamespace(packet=f"packet:{analysis}".encode())


class EncoderCoreTests(unittest.TestCase):
    def setUp(self) -> None:
        FakeSession.instances = []
        self.profile = load_wem_profile(PROFILE_NAME)

    def _patch_constructor(self):
        return (
            patch.object(EncoderProfile, "setup_packet", return_value=b"setup"),
            patch(
                f"{REFERENCE_ENGINE}.assemble_encoder_profile_resources",
                return_value=SimpleNamespace(
                    analysis=object(),
                    setup_packet=b"setup",
                    setup={"book_ids": [7]},
                    codebooks=("book",),
                ),
            ),
        )

    def _python_engine_patch(self):
        """Route facade dispatch to the pure-Python reference path."""
        return patch("wwise_wem._engine.active_engine", return_value="python")

    def test_constructor_owns_profile(self) -> None:
        encoder = Encoder(self.profile)

        self.assertIs(encoder.profile, self.profile)

    def test_python_encode_path_assembles_resources_via_reference_engine(self) -> None:
        assemble = patch(
            f"{REFERENCE_ENGINE}.assemble_encoder_profile_resources",
            return_value=SimpleNamespace(
                analysis=object(),
                setup_packet=b"setup",
                setup={"book_ids": [7]},
                codebooks=("book",),
            ),
        )
        with (
            self._python_engine_patch(),
            patch.object(
                EncoderProfile, "setup_packet", return_value=b"setup"
            ) as setup_mock,
            assemble as assemble_mock,
            patch(f"{REFERENCE_ENGINE}.AnalysisSession", FakeSession),
            patch(
                f"{REFERENCE_ENGINE}.pack_analysis_frame",
                side_effect=_pack_analysis,
            ),
            patch(f"{REFERENCE_ENGINE}.build_vorbis_wem", return_value=b"WEM!"),
        ):
            encoder = Encoder(self.profile)
            encoder.encode_pcm(_pcm())

        setup_mock.assert_called_once_with()
        self.assertEqual(assemble_mock.call_count, 1)
        self.assertEqual(assemble_mock.call_args.kwargs, {"setup_packet": b"setup"})

    def test_encode_pcm_returns_typed_result_and_builds_canonical_container(self) -> None:
        patches = self._patch_constructor()
        with (
            patches[0], patches[1],
            self._python_engine_patch(),
            patch(f"{REFERENCE_ENGINE}.AnalysisSession", FakeSession),
            patch(
                f"{REFERENCE_ENGINE}.pack_analysis_frame",
                side_effect=_pack_analysis,
            ) as pack,
            patch(f"{REFERENCE_ENGINE}.build_vorbis_wem", return_value=b"WEM!") as build,
        ):
            result = Encoder(self.profile).encode_pcm(_pcm())

        self.assertIsInstance(result, EncodeResult)
        self.assertIsInstance(result.stats, EncodeStats)
        self.assertEqual(result.data, b"WEM!")
        self.assertEqual(
            result.stats.to_legacy_dict(),
            {
                "pcm_frames": 4096,
                "channels": 6,
                "audio_packets": 2,
                "short_packets": 1,
                "long_packets": 1,
                "bytes": 4,
                "metadata_source": f"profile:{PROFILE_NAME}",
                "engine": "python",
            },
        )
        self.assertEqual(len(FakeSession.instances), 1)
        session = FakeSession.instances[0]
        self.assertEqual(session.arguments, (6, 44100, (256, 2048)))
        self.assertIsNotNone(session.resources)
        self.assertEqual(session.selected_pcm, _pcm().channels)
        self.assertEqual(pack.call_count, 2)
        self.assertEqual(
            [call.kwargs["channels"] for call in pack.call_args_list],
            [6, 6],
        )

        fmt, packets = build.call_args.args
        self.assertEqual(fmt["dwTotalPCMFrames"], 4096)
        self.assertEqual(
            packets,
            [
                b"setup",
                b"packet:analysis:short-window",
                b"packet:analysis:long-window",
            ],
        )
        self.assertEqual(
            build.call_args.kwargs,
            {
                "seek_table": b"",
                "endian": "le",
                "extra_chunks": [],
                "recompute_sizes": True,
            },
        )

    def test_each_encode_uses_a_fresh_analysis_session(self) -> None:
        patches = self._patch_constructor()
        with (
            patches[0], patches[1],
            self._python_engine_patch(),
            patch(f"{REFERENCE_ENGINE}.AnalysisSession", FakeSession),
            patch(
                f"{REFERENCE_ENGINE}.pack_analysis_frame",
                side_effect=_pack_analysis,
            ),
            patch(f"{REFERENCE_ENGINE}.build_vorbis_wem", return_value=b"WEM!"),
        ):
            encoder = Encoder(self.profile)
            first = encoder.encode_pcm(_pcm())
            second = encoder.encode_pcm(_pcm())

        self.assertEqual(first, second)
        self.assertEqual(len(FakeSession.instances), 2)
        self.assertIsNot(FakeSession.instances[0], FakeSession.instances[1])

    def test_geometry_and_input_type_fail_before_session_creation(self) -> None:
        patches = self._patch_constructor()
        with (
            patches[0], patches[1],
            self._python_engine_patch(),
            patch(f"{REFERENCE_ENGINE}.AnalysisSession", FakeSession),
        ):
            encoder = Encoder(self.profile)
            with self.assertRaisesRegex(TypeError, "PcmBuffer"):
                encoder.encode_pcm("pcm")  # type: ignore[arg-type]
            with self.assertRaisesRegex(ValueError, "differs from encoder profile"):
                encoder.encode_pcm(_pcm(channels=2))
            with self.assertRaisesRegex(ValueError, "differs from encoder profile"):
                encoder.encode_pcm(_pcm(rate=48000))
            with self.assertRaisesRegex(ValueError, "at least 4096 frames"):
                encoder.encode_pcm(_pcm(frames=3))
        self.assertEqual(FakeSession.instances, [])

    def test_unknown_runtime_bundle_fails_before_setup_parse(self) -> None:
        profile = replace(
            self.profile,
            setup_sha256="0" * 64,
            key=replace(
                self.profile.key,
                quality_setup_identity="sha256:" + "0" * 64,
            ),
        )
        with patch(
            f"{REFERENCE_ENGINE}.assemble_encoder_profile_resources"
        ) as assemble:
            with self.assertRaisesRegex(ValueError, "differs from installed profile"):
                Encoder(profile)
        assemble.assert_not_called()


if __name__ == "__main__":
    unittest.main()
