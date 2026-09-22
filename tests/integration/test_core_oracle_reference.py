"""Core-oracle reference parity: facade (native kernel) vs reference oracle.

The single execution path, executable: the facade's byte-producing calls
run on the in-package native extension (``wwise_wem._core``), and the
reference oracle — the ``wwise_wem_reference`` package, imported
directly as a **test fixture** (a test asset, not an engine: nothing in
the distributed package imports it at runtime) — must produce
byte-identical WEM bytes for the same PCM.  The reference-input case must
also match the reference WEM file: three-way, facade == oracle ==
``reference.wem``.  Boundary-length PCM inputs (around the short/long
frame-plan edges) exercise the same requirement on synthetic streams.

The native extension is a required runtime asset: this suite imports
``wwise_wem._core`` unconditionally and deliberately designs no skip case
for native-absent environments.
"""

from __future__ import annotations

import array
import unittest
from pathlib import Path

from wwise_wem import WwiseProfile, WwiseVersion
from wwise_wem.application.encoder import Encoder
from wwise_wem_reference.profiles.artifact import resolve_selection
from wwise_wem import _core as core_module
from wwise_wem.adapters.wav import read_pcm_wav
from wwise_wem.model import PcmBuffer
from wwise_wem_reference import python_engine
from wwise_wem_reference.container.model import ContainerPlan

ROOT = Path(__file__).resolve().parents[2]
FIXTURES = ROOT / "tests" / "fixtures"
INPUT = FIXTURES / "input.wav"
REFERENCE = FIXTURES / "reference.wem"
SELECTION = WwiseProfile(WwiseVersion.WWISE2013, 6, 44100)
PROFILE = resolve_selection(SELECTION)

STEREO_SELECTION = WwiseProfile(WwiseVersion.WWISE2013, 2, 48000)
STEREO_PROFILE = resolve_selection(STEREO_SELECTION)

# Frame counts probing the frame-plan edges: the minimum length, both sides
# of each short/long boundary, and a long-stream value.
BOUNDARY_FRAME_COUNTS = (
    4096,
    4097,
    8191,
    8192,
    12287,
    12288,
)


def _rows_from_pcm(pcm: PcmBuffer) -> list[list[int]]:
    """Kernel-form rows for in-domain PCM (read_pcm_wav output is in-domain)."""
    return [[int(sample * 32768.0) for sample in row] for row in pcm.channels]


def _channel_major_view(pcm: PcmBuffer) -> memoryview:
    """The same PCM as the 2-D ``(channels, frames)`` signed-16 view.

    ``Encoder.encode_pcm`` accepts either a list of per-channel rows or this
    buffer form; the buffer form is what a caller holding PCM in an array
    already has, so it is the one that must be proved identical.
    """
    flat = array.array("h")
    for row in _rows_from_pcm(pcm):
        flat.extend(row)
    channels = len(pcm.channels)
    frames = len(pcm.channels[0])
    # `memoryview.cast` reshapes only through a byte format, so go via 'B'.
    raw = memoryview(flat).cast("B")
    return raw.cast("h", (channels, frames))


def _oracle_encode(pcm: PcmBuffer, profile) -> "object":
    """Direct reference-oracle import: the test-fixture encode entry."""
    return python_engine.encode_pcm_python(
        profile=profile,
        container=ContainerPlan.from_profile(profile),
        pcm=pcm,
    )


def _synthetic_pcm(frames: int) -> PcmBuffer:
    """Deterministic in-domain signed-16 PCM of ``frames`` length."""
    channels = []
    for channel in range(6):
        channels.append(
            tuple(
                ((frame * 7 + channel * 11 + 3) % 64536 - 32768) / 32768.0
                for frame in range(frames)
            )
        )
    return PcmBuffer(44100, tuple(channels))


def _synthetic_stereo_pcm(frames: int = 16384) -> PcmBuffer:
    """Deterministic 2ch input covering short, long, and terminal frames."""
    channels = tuple(
        tuple(
            ((frame * 7 + channel * 11 + 3) % 64536 - 32768) / 32768.0
            for frame in range(frames)
        )
        for channel in range(2)
    )
    return PcmBuffer(48000, channels)


class CoreOracleReferenceTests(unittest.TestCase):
    def test_reference_input_matches_reference_on_facade_and_oracle(self):
        pcm = read_pcm_wav(INPUT)
        reference = REFERENCE.read_bytes()

        # Single execution path: the facade runs the native kernel.
        facade = Encoder(SELECTION).encode_pcm(pcm)
        oracle_result = _oracle_encode(pcm, PROFILE)
        direct_core = core_module.Encoder(SELECTION).encode_pcm(
            pcm.sample_rate, _rows_from_pcm(pcm)
        )

        self.assertEqual(facade.data, reference)
        self.assertEqual(bytes(oracle_result.data), reference)
        self.assertEqual(bytes(direct_core.data), reference)
        # One execution path: the stats surface carries no provenance tag.
        self.assertNotIn("engine", facade.stats.to_dict())
        # The whole stats surface agrees: the oracle and the kernel observe
        # the same things about the same encode.
        self.assertEqual(
            facade.stats.to_dict(),
            oracle_result.stats.to_dict(),
        )


    def test_memoryview_pcm_is_the_same_container_as_the_list_form(self):
        """Both accepted PCM forms reach the same bytes.

        The buffer form is decoded channel-by-channel from one raw copy; it
        must agree with the list form and with the reference file exactly.
        """
        pcm = read_pcm_wav(INPUT)
        reference = REFERENCE.read_bytes()
        encoder = core_module.Encoder(SELECTION)

        from_list = encoder.encode_pcm(pcm.sample_rate, _rows_from_pcm(pcm))
        from_view = encoder.encode_pcm(pcm.sample_rate, _channel_major_view(pcm))

        self.assertEqual(bytes(from_view.data), reference)
        self.assertEqual(bytes(from_view.data), bytes(from_list.data))
        # Same accounting as the list form (the binding exposes the
        # fields individually; the facade is what groups them).
        for field in (
            "pcm_frames",
            "channels",
            "audio_packets",
            "short_packets",
            "long_packets",
        ):
            self.assertEqual(
                getattr(from_view, field), getattr(from_list, field), field
            )

    def test_memoryview_pcm_rejects_what_it_cannot_decode(self):
        pcm = read_pcm_wav(INPUT)
        encoder = core_module.Encoder(SELECTION)
        frames = len(pcm.channels[0])
        channels = len(pcm.channels)

        # 1-D: not the documented (channels, frames) geometry.
        with self.assertRaisesRegex(core_module.WemEncoderError, "must be 2-D"):
            encoder.encode_pcm(pcm.sample_rate, memoryview(b"\x00\x00"))

        # 2-D but unsigned bytes: not the signed-16 domain.
        wide = memoryview(bytearray(channels * frames * 2)).cast(
            "B", (channels, frames * 2)
        )
        with self.assertRaisesRegex(core_module.WemEncoderError, "must be signed-16"):
            encoder.encode_pcm(pcm.sample_rate, wide)

        # 2-D and signed-16 but strided: its bytes are not one run per channel.
        strided = _channel_major_view(pcm)[::2]
        with self.assertRaisesRegex(core_module.WemEncoderError, "must be C-contiguous"):
            encoder.encode_pcm(pcm.sample_rate, strided)

    def test_boundary_length_inputs_are_byte_identical_between_facade_and_oracle(
        self,
    ):
        for frames in BOUNDARY_FRAME_COUNTS:
            with self.subTest(frames=frames):
                pcm = _synthetic_pcm(frames)

                oracle = _oracle_encode(pcm, PROFILE)
                native = Encoder(SELECTION).encode_pcm(pcm)

                self.assertEqual(oracle.data, native.data)
                self.assertGreater(len(oracle.data), 0)
                self.assertEqual(oracle.stats.pcm_frames, frames)
                self.assertEqual(
                    oracle.stats.to_dict(),
                    native.stats.to_dict(),
                )

    def test_stereo_profile_has_a_pinned_native_oracle_byte_identity(self):
        # The synthetic stream has no committed counterpart; the 2ch/48 kHz
        # configuration's absolute byte pin is the committed real-build
        # corpora (tests/data/2ch-reference, tests/data/2ch-stress), so this
        # case pins the three implementations to each other byte-for-byte.
        pcm = _synthetic_stereo_pcm()

        oracle = _oracle_encode(pcm, STEREO_PROFILE)
        native = Encoder(STEREO_SELECTION).encode_pcm(pcm)
        direct_core = core_module.Encoder(STEREO_SELECTION).encode_pcm(
            pcm.sample_rate, _rows_from_pcm(pcm)
        )

        self.assertEqual(bytes(native.data), bytes(oracle.data))
        self.assertEqual(bytes(direct_core.data), bytes(oracle.data))
        self.assertEqual(native.stats.audio_packets, 26)
        self.assertEqual(native.stats.short_packets, 10)
        self.assertEqual(native.stats.long_packets, 16)


if __name__ == "__main__":
    unittest.main()
