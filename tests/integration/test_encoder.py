from __future__ import annotations

import unittest
from dataclasses import replace
from types import SimpleNamespace
from unittest.mock import patch

from wwise2013_wem.application.encoder import Encoder, _ContainerPlan
from wwise2013_wem.application.models import EncodeResult, EncodeStats
from wwise2013_wem.model import PcmBuffer
from wwise2013_wem.profiles.model import EncoderProfile
from wwise2013_wem.profiles.registry import load_wem_profile


PROFILE_NAME = "wwise2013-6ch-44100"


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
        resources = SimpleNamespace(
            analysis=object(),
            setup_packet=b"setup",
            setup={"book_ids": [7]},
            codebooks=("book",),
        )
        return (
            patch.object(EncoderProfile, "setup_packet", return_value=b"setup"),
            patch(
                "wwise2013_wem.application.encoder.assemble_encoder_profile_resources",
                return_value=resources,
            ),
        )

    def test_constructor_owns_manifest_setup_and_books(self) -> None:
        setup_packet, assemble = self._patch_constructor()
        with setup_packet as setup_mock, assemble as assemble_mock:
            encoder = Encoder(self.profile)

        setup_mock.assert_called_once_with()
        self.assertEqual(assemble_mock.call_count, 1)
        self.assertEqual(assemble_mock.call_args.kwargs, {"setup_packet": b"setup"})
        self.assertIs(encoder.profile, self.profile)
        self.assertEqual(encoder._setup_packet, b"setup")
        self.assertEqual(encoder._books, ("book",))

    def test_encode_pcm_returns_typed_result_and_builds_canonical_container(self) -> None:
        patches = self._patch_constructor()
        with (
            patches[0], patches[1],
            patch("wwise2013_wem.application.encoder.AnalysisSession", FakeSession),
            patch(
                "wwise2013_wem.application.encoder.pack_analysis_frame",
                side_effect=_pack_analysis,
            ) as pack,
            patch("wwise2013_wem.application.encoder.build_vorbis_wem", return_value=b"WEM!") as build,
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
            patch("wwise2013_wem.application.encoder.AnalysisSession", FakeSession),
            patch(
                "wwise2013_wem.application.encoder.pack_analysis_frame",
                side_effect=_pack_analysis,
            ),
            patch("wwise2013_wem.application.encoder.build_vorbis_wem", return_value=b"WEM!"),
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
            patch("wwise2013_wem.application.encoder.AnalysisSession", FakeSession),
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

    def test_private_container_plan_is_defensive_and_controls_output_source(self) -> None:
        fmt = {"nChannels": 6, "nSamplesPerSec": 44100, "marker": 1}
        seek = bytearray(b"seek")
        chunk_id = bytearray(b"JUNK")
        payload = bytearray(b"meta")
        plan = _ContainerPlan(
            fmt=fmt,
            endian="be",
            seek_table=seek,
            extra_chunks=((chunk_id, payload),),  # type: ignore[arg-type]
            metadata_source="template:sample.wem",
        )
        fmt["marker"] = 2
        seek[0] = ord("X")
        chunk_id[0] = ord("X")
        payload[0] = ord("X")

        patches = self._patch_constructor()
        with (
            patches[0], patches[1],
            patch("wwise2013_wem.application.encoder.AnalysisSession", FakeSession),
            patch(
                "wwise2013_wem.application.encoder.pack_analysis_frame",
                side_effect=_pack_analysis,
            ),
            patch("wwise2013_wem.application.encoder.build_vorbis_wem", return_value=b"WEM!") as build,
        ):
            result = Encoder(self.profile, _container=plan).encode_pcm(_pcm())

        self.assertEqual(result.stats.metadata_source, "template:sample.wem")
        self.assertEqual(build.call_args.args[0]["marker"], 1)
        self.assertEqual(build.call_args.kwargs["endian"], "be")
        self.assertEqual(build.call_args.kwargs["seek_table"], b"seek")
        self.assertEqual(build.call_args.kwargs["extra_chunks"], [(b"JUNK", b"meta")])

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
            "wwise2013_wem.application.encoder.assemble_encoder_profile_resources"
        ) as assemble:
            with self.assertRaisesRegex(ValueError, "differs from installed profile"):
                Encoder(profile)
        assemble.assert_not_called()


if __name__ == "__main__":
    unittest.main()
