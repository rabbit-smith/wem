"""The facade's execution path: the native kernel against the reference oracle.

Four files, one claim from four angles. The fixture cases prove the facade,
the raw binding and the oracle produce the committed reference bytes; the
oracle cases walk boundary PCM lengths, the memoryview PCM form and the 2ch
configuration; the extended-input cases drive 24-bit, float32 and raw PCM
through the same comparison; the quality cases prove a bound quality reaches
the kernel and that no quality keeps the reference bytes. Each class keeps
its own test names and failure messages.
"""

from __future__ import annotations

import array
import struct
import unittest
import wave
import wwise_wem as W

from pathlib import Path
from wwise_wem import WwiseProfile, WwiseVersion
from wwise_wem import _core as core_mod
from wwise_wem import _core as core_module
from wwise_wem.adapters.raw import read_raw_pcm
from wwise_wem.adapters.wav import read_pcm_wav
from wwise_wem.application.encoder import Encoder
from wwise_wem.application.models import EncodeResult, EncodeStats
from wwise_wem.model import PcmBuffer
from wwise_wem_reference import python_engine
from wwise_wem_reference.container.model import ContainerPlan
from wwise_wem_reference.profiles.artifact import resolve_selection
from wwise_wem_reference.profiles.quality import (
    QUALITY_NORMALIZE_CLAMP,
    _linear_frac,
    normalize_quality_factor,
)

# --------------------------------------------------------------------------
# merged from tests/integration/test_facade_core.py
# --------------------------------------------------------------------------

# The facade's execution path is the native kernel — byte-for-byte.
#
# Executable definition of the single execution path: the facade's
# byte-producing calls (``Encoder.encode_pcm``, ``encode``) run on the
# in-package native extension ``wwise_wem._core`` and must produce
# byte-identical WEM output to the raw kernel binding and to the reference
# oracle (``wwise_wem_reference``, imported directly as a test asset — it
# is a fixture, not an engine).  Error paths (short input, geometry
# mismatch) and the rejection conditions of the signed-16 sample domain are
# facade-level invariants, raised as plain ``ValueError`` before the kernel
# is reached.
#
# The native extension is a required runtime asset: this suite imports
# ``wwise_wem._core`` unconditionally and deliberately designs no skip case
# for native-absent environments.

ROOT = Path(__file__).resolve().parents[2]
FIXTURES = ROOT / "tests" / "fixtures"
INPUT = FIXTURES / "input.wav"
REFERENCE = FIXTURES / "reference.wem"
SELECTION = WwiseProfile(WwiseVersion.WWISE2013, 6, 44100)
EXPECTED_STATS = {
    "pcm_frames": 139398,
    "channels": 6,
    "audio_packets": 205,
    "short_packets": 77,
    "long_packets": 128,
}
#: The container length, counted by the caller from the bytes it holds — the
#: library returns observations, not a length of bytes the caller already has.
EXPECTED_CONTAINER_BYTES = 108771


def _rows_from_pcm(pcm: PcmBuffer) -> list[list[int]]:
    """Kernel-form rows for in-domain PCM (read_pcm_wav output is in-domain)."""
    return [[int(sample * 32768.0) for sample in row] for row in pcm.channels]


class FacadeIsCoreTests(unittest.TestCase):
    """The facade is the core: one execution path, identical bytes."""

    def test_facade_bytes_match_the_raw_core_binding_and_reference(self):
        pcm = read_pcm_wav(INPUT)
        reference = REFERENCE.read_bytes()

        result = Encoder(SELECTION).encode_pcm(pcm)
        direct = core_module.Encoder(SELECTION).encode_pcm(
            pcm.sample_rate, _rows_from_pcm(pcm)
        )

        self.assertEqual(result.data, reference)
        self.assertEqual(result.stats.to_dict(), EXPECTED_STATS)
        self.assertEqual(len(result), EXPECTED_CONTAINER_BYTES)
        self.assertEqual(bytes(direct.data), reference)
        # The facade output is what the in-package core binding produces:
        # there is no second path that could diverge.
        self.assertEqual(len(bytes(direct.data)), len(result.data))

    def test_facade_binds_the_in_package_core_module(self):
        # The facade's execution-path import IS the in-package extension;
        # lock the binding so a stray fallback module cannot sneak in.
        from wwise_wem.application import encoder as encoder_module

        self.assertIs(encoder_module._core, core_module)
        self.assertEqual(core_module.__name__, "wwise_wem._core")

    def test_stats_surface_has_no_provenance_field(self):
        # The single execution path needs no provenance tag: the result
        # stats expose only encoding facts.
        result = Encoder(SELECTION).encode_pcm(read_pcm_wav(INPUT))
        self.assertIsInstance(result, EncodeResult)
        self.assertIsInstance(result.stats, EncodeStats)
        self.assertFalse(hasattr(result.stats, "engine"))
        self.assertNotIn("engine", result.stats.to_dict())

    def test_error_paths_are_facade_invariants(self):
        short = PcmBuffer(
            44100,
            tuple(tuple(0.0 for _ in range(100)) for _ in range(6)),
        )
        geometry = PcmBuffer(
            44100,
            tuple(tuple(0.0 for _ in range(4096)) for _ in range(2)),
        )

        with self.assertRaisesRegex(ValueError, "at least 4096 frames"):
            Encoder(SELECTION).encode_pcm(short)
        with self.assertRaisesRegex(
            ValueError, "differs from encoder profile"
        ):
            Encoder(SELECTION).encode_pcm(geometry)

    def test_out_of_domain_pcm_raises_plain_value_error(self):
        # 4095.0 * 32768 is outside the signed-16 range: out of domain.
        pcm = PcmBuffer(
            44100,
            tuple(tuple(4095.0 for _ in range(4096)) for _ in range(6)),
        )
        with self.assertRaisesRegex(ValueError, "integer signed-16 sample"):
            Encoder(SELECTION).encode_pcm(pcm)
        # A fractional float is out of domain the same way.
        pcm = PcmBuffer(
            44100,
            tuple(tuple(0.1 for _ in range(4096)) for _ in range(6)),
        )
        with self.assertRaisesRegex(ValueError, "integer signed-16 sample"):
            Encoder(SELECTION).encode_pcm(pcm)


# --------------------------------------------------------------------------
# merged from tests/integration/test_core_oracle_reference.py
# --------------------------------------------------------------------------

# Core-oracle reference parity: facade (native kernel) vs reference oracle.
#
# The single execution path, executable: the facade's byte-producing calls
# run on the in-package native extension (``wwise_wem._core``), and the
# reference oracle — the ``wwise_wem_reference`` package, imported
# directly as a **test fixture** (a test asset, not an engine: nothing in
# the distributed package imports it at runtime) — must produce
# byte-identical WEM bytes for the same PCM.  The reference-input case must
# also match the reference WEM file: three-way, facade == oracle ==
# ``reference.wem``.  Boundary-length PCM inputs (around the short/long
# frame-plan edges) exercise the same requirement on synthetic streams.
#
# The native extension is a required runtime asset: this suite imports
# ``wwise_wem._core`` unconditionally and deliberately designs no skip case
# for native-absent environments.

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


# --------------------------------------------------------------------------
# merged from tests/integration/test_extended_input_formats.py
# --------------------------------------------------------------------------

# Extended input forms: single-path consistency and short-input semantics.
#
# New-format inputs (24-bit PCM, 32-bit float32 WAV, raw PCM) are converted at
# the adapter boundary into the signed-16 domain.  These tests lock two
# properties:
#
# * converted inputs land on exactly the in-domain floats of the equivalent
#   signed-16 stream, so the encoder consumes format-agnostic values;
# * the facade path (the native kernel, ``wwise_wem._core``) and the direct
#   ``wwise_wem_reference`` import (the test-time reference oracle, a pure
#   test asset and not an engine) produce byte-identical WEM output for them,
#   and encoding is deterministic.
#
# Single-path environment: the native extension ``wwise_wem._core`` is a
# required runtime asset; this suite deliberately does not design skip cases
# for native-absent environments.  Short inputs (< 4096 frames) keep the
# explicit rejection (INPUT_TOO_SHORT semantics) on every entry point.

ROOT = Path(__file__).resolve().parents[2]
FIXTURES = ROOT / "tests" / "fixtures"
INPUT = FIXTURES / "input.wav"
REFERENCE = FIXTURES / "reference.wem"
SELECTION = WwiseProfile(WwiseVersion.WWISE2013, 6, 44100)
PROFILE = resolve_selection(SELECTION)


def _synthetic_int16(frames: int, channels: int = 6) -> list[int]:
    """Deterministic signed-16 stream; one value per (channel, frame)."""
    return [
        ((frame * 7 + channel * 11 + 3) % 64536 - 32768)
        for channel in range(channels)
        for frame in range(frames)
    ]


def _int16_bytes(values: list[int]) -> bytes:
    return b"".join(v.to_bytes(2, "little", signed=True) for v in values)


def _int24_bytes(values: list[int]) -> bytes:
    return b"".join((v << 8).to_bytes(3, "little", signed=True) for v in values)


def _float32_bytes(values: list[int]) -> bytes:
    return struct.pack(f"<{len(values)}f", *[v / 32768.0 for v in values])


def _write_int16_wav(path: Path, values: list[int], channels: int, rate: int) -> None:
    with wave.open(str(path), "wb") as target:
        target.setnchannels(channels)
        target.setsampwidth(2)
        target.setframerate(rate)
        target.writeframes(_int16_bytes(values))


def _write_int24_wav(path: Path, values: list[int], channels: int, rate: int) -> None:
    with wave.open(str(path), "wb") as target:
        target.setnchannels(channels)
        target.setsampwidth(3)
        target.setframerate(rate)
        target.writeframes(_int24_bytes(values))


def _write_float32_wav(path: Path, values: list[int], channels: int, rate: int) -> None:
    body = _float32_bytes(values)
    fmt = struct.pack(
        "<HHIIHH", 3, channels, rate, rate * channels * 4, channels * 4, 32
    )
    with open(path, "wb") as handle:
        handle.write(b"RIFF")
        handle.write(struct.pack("<I", 36 + len(body)))
        handle.write(b"WAVE")
        handle.write(b"fmt ")
        handle.write(struct.pack("<I", len(fmt)))
        handle.write(fmt)
        handle.write(b"data")
        handle.write(struct.pack("<I", len(body)))
        handle.write(body)


class ExtendedInputDomainTests(unittest.TestCase):
    """Conversion lands in-domain; encoding is deterministic on the facade."""

    def test_converted_inputs_match_the_int16_domain(self):
        values = _synthetic_int16(5000)
        pcm16 = read_raw_pcm(
            _int16_bytes(values), sample_rate=44100, channels=6, bits_per_sample=16
        )
        pcm24 = read_raw_pcm(
            _int24_bytes(values), sample_rate=44100, channels=6, bits_per_sample=24
        )
        pcm32 = read_raw_pcm(
            _float32_bytes(values), sample_rate=44100, channels=6, bits_per_sample=32
        )
        self.assertEqual(pcm16, pcm24)
        self.assertEqual(pcm24, pcm32)

    def test_converted_wav_forms_match_the_int16_domain(self):
        values = _synthetic_int16(5000)
        import tempfile

        with tempfile.TemporaryDirectory() as directory:
            base = Path(directory)
            wav16 = base / "a16.wav"
            wav24 = base / "a24.wav"
            wav32 = base / "a32f.wav"
            _write_int16_wav(wav16, values, 6, 44100)
            _write_int24_wav(wav24, values, 6, 44100)
            _write_float32_wav(wav32, values, 6, 44100)
            pcm16 = read_pcm_wav(wav16)
            pcm24 = read_pcm_wav(wav24)
            pcm32 = read_pcm_wav(wav32)
            self.assertEqual(pcm16, pcm24)
            self.assertEqual(pcm24, pcm32)

    def test_packed_wav_path_matches_typed_pcm(self):
        packed = W.encode(INPUT)
        typed = W.encode(read_pcm_wav(INPUT))
        self.assertEqual(packed.data, typed.data)

    def test_encoding_is_deterministic_across_runs_and_forms(self):
        import tempfile

        values = _synthetic_int16(4096)
        with tempfile.TemporaryDirectory() as directory:
            base = Path(directory)
            wav24 = base / "a24.wav"
            wav32 = base / "a32f.wav"
            _write_int24_wav(wav24, values, 6, 44100)
            _write_float32_wav(wav32, values, 6, 44100)

            first = W.encode(wav24)
            again = W.encode(wav32)
            raw24 = W.encode(W.RawPcm(_int24_bytes(values), 44100, 6, "s24le"))
            raw32 = W.encode(W.RawPcm(_float32_bytes(values), 44100, 6, "f32le"))
            # Same samples through four entry forms: one byte stream.
            self.assertEqual(first.data, again.data)
            self.assertEqual(first.data, raw24.data)
            self.assertEqual(first.data, raw32.data)
            self.assertEqual(first.stats.pcm_frames, 4096)

    def test_short_input_keeps_the_explicit_rejection(self):
        import tempfile

        values = _synthetic_int16(4095)
        with tempfile.TemporaryDirectory() as directory:
            base = Path(directory)
            wav24 = base / "a24.wav"
            wav32 = base / "a32f.wav"
            _write_int24_wav(wav24, values, 6, 44100)
            _write_float32_wav(wav32, values, 6, 44100)

            cases = (
                lambda: W.encode(wav24),
                lambda: W.encode(wav32),
                lambda: W.encode(W.RawPcm(_int24_bytes(values), 44100, 6, "s24le")),
                lambda: W.encode(W.RawPcm(_int16_bytes(values), 44100, 6, "s16le")),
            )
            for index, action in enumerate(cases):
                with self.subTest(form=index):
                    with self.assertRaisesRegex(
                        ValueError, "at least 4096 frames"
                    ):
                        action()

    def test_geometry_mismatch_keeps_the_explicit_rejection(self):
        # 2ch/44100 is not a registered profile (the 2ch profile is 48000):
        # unsupported geometry must still fail with the explicit rejection.
        values = _synthetic_int16(4096, channels=2)
        with self.assertRaisesRegex(ValueError, "2ch/44100Hz"):
            W.encode(W.RawPcm(_int16_bytes(values), 44100, 2, "s16le"))


class ExtendedInputConsistencyTests(unittest.TestCase):
    """Facade (native kernel) vs the direct reference import: byte identity.

    ``wwise_wem_reference`` is imported directly as a test-time oracle; it
    is a pure test asset, not a runtime engine.  The facade runs its single
    execution path — the native kernel — unconditionally.  Both must
    produce identical WEM bytes.
    """

    def _oracle_encode(self, pcm) -> EncodeResult:
        return python_engine.encode_pcm_python(
            profile=PROFILE,
            container=ContainerPlan.from_profile(PROFILE),
            pcm=pcm,
        )

    def test_whole_file_input_via_extended_entry_matches_reference_and_oracle(self):
        pcm = read_pcm_wav(INPUT)
        reference = REFERENCE.read_bytes()

        facade = W.encode(INPUT, profile=SELECTION)
        oracle = self._oracle_encode(pcm)
        direct_core = core_mod.Encoder(SELECTION).encode_pcm(
            44100,
            [[int(sample * 32768.0) for sample in row] for row in pcm.channels],
        )
        self.assertEqual(facade.data, reference)
        self.assertEqual(bytes(oracle.data), reference)
        self.assertEqual(bytes(direct_core.data), reference)
        self.assertEqual(facade.data, oracle.data)
        self.assertEqual(bytes(facade.data), bytes(direct_core.data))

    def test_converted_inputs_are_byte_identical_between_facade_and_oracle(self):
        import tempfile

        values = _synthetic_int16(4096)
        with tempfile.TemporaryDirectory() as directory:
            base = Path(directory)
            wav24 = base / "a24.wav"
            wav32 = base / "a32f.wav"
            _write_int24_wav(wav24, values, 6, 44100)
            _write_float32_wav(wav32, values, 6, 44100)
            forms = [
                ("wav-24", lambda: W.encode(wav24)),
                ("wav-32f", lambda: W.encode(wav32)),
                (
                    "raw-24",
                    lambda: W.encode(W.RawPcm(_int24_bytes(values), 44100, 6, "s24le")),
                ),
                (
                    "raw-32f",
                    lambda: W.encode(W.RawPcm(_float32_bytes(values), 44100, 6, "f32le")),
                ),
                (
                    "raw-16",
                    lambda: W.encode(W.RawPcm(_int16_bytes(values), 44100, 6, "s16le")),
                ),
            ]
            # The oracle encodes the one in-domain PCM every form converts
            # to; all facade results must equal it byte-for-byte.
            pcm = read_raw_pcm(
                _int16_bytes(values),
                sample_rate=44100,
                channels=6,
                bits_per_sample=16,
            )
            oracle_result = self._oracle_encode(pcm)
            for label, action in forms:
                with self.subTest(form=label):
                    facade_result = action()
                    self.assertEqual(facade_result.data, oracle_result.data, label)
                    self.assertGreater(len(facade_result.data), 0)
                    self.assertEqual(
                        facade_result.stats.to_dict(),
                        oracle_result.stats.to_dict(),
                        label,
                    )


# --------------------------------------------------------------------------
# merged from tests/integration/test_quality_cross_impl.py
# --------------------------------------------------------------------------

# Cross-implementation quality semantics: Python facade vs native kernel.
#
# Proves the Python/Rust quality mechanism stays aligned end to end:
#
# * the shared pinned test vectors (the same doubles the Rust kernel pins in
#   ``crates/wem-profiles`` tests) are reproduced by the Python reference;
# * a quality value bound to a profile is forwarded from the Python facade
#   into the native kernel's analysis assembly (the 6ch profile carries no
#   quality-curves resource, so a quality request must surface the kernel's
#   configuration error — with a stale or bypassed kernel the encode would
#   silently ignore the quality);
# * with no quality the facade keeps the reference bytes (compared byte-for-byte
#   against ``tests/fixtures/reference.wem``);
# * a bound quality is an additive copy: the resolved profile is never
#   mutated.

_SELECTION = WwiseProfile(WwiseVersion.WWISE2013, 6, 44100)
_FIXTURES = Path(__file__).resolve().parents[2] / "tests" / "fixtures"
_FIXTURE_WAV = _FIXTURES / "input.wav"
_REFERENCE_WEM = _FIXTURES / "reference.wem"


class QualityCrossImplementationTests(unittest.TestCase):
    def test_shared_pinned_vectors_are_implementation_stable(self):
        # The Rust kernel pins these exact values (quality_parity.rs); the
        # Python reference must return identical doubles.
        self.assertEqual(normalize_quality_factor(0.0), 1e-07)
        self.assertEqual(normalize_quality_factor(4.0), 0.4000001)
        self.assertEqual(normalize_quality_factor(10.0), QUALITY_NORMALIZE_CLAMP)
        self.assertEqual(QUALITY_NORMALIZE_CLAMP, 0.9998999834060669)

        bp = (0.5, 0.9)
        samples = (0.0, 1.0)
        self.assertEqual(_linear_frac(bp, samples, 0.7), (0.4999999999999999, False))
        self.assertEqual(_linear_frac(bp, samples, 0.9), (0.999, False))
        self.assertEqual(_linear_frac(bp, samples, 1.0), (0.999, True))

        bp = (0.0, 4.0, 8.0)
        samples = (10.0, 20.0, 30.0)
        self.assertEqual(_linear_frac(bp, samples, 2.0), (15.0, False))
        self.assertEqual(_linear_frac(bp, samples, 8.0), (29.990000000000002, False))
        self.assertEqual(_linear_frac(bp, samples, 9.0), (29.990000000000002, True))

    def test_facade_quality_is_forwarded_to_the_kernel(self):
        # The 6ch profile carries no quality-curves resource; requesting a
        # quality through the facade must therefore surface the kernel's
        # configuration error (proof the quality reaches the kernel's
        # assembly — a kernel that ignores quality would encode instead).
        pcm = read_pcm_wav(_FIXTURE_WAV)
        encoder = Encoder(_SELECTION, quality=4.0)
        with self.assertRaisesRegex(ValueError, "quality-curves"):
            encoder.encode_pcm(pcm)

    def test_facade_quality_none_keeps_the_reference_bytes(self):
        pcm = read_pcm_wav(_FIXTURE_WAV)
        result = Encoder(_SELECTION).encode_pcm(pcm)
        self.assertEqual(result.data, _REFERENCE_WEM.read_bytes())

    def test_selection_quality_copy_never_mutates_the_resolved_profile(self):
        base = resolve_selection(_SELECTION)
        bound = resolve_selection(_SELECTION, quality=9.0)
        self.assertEqual(bound.quality, 9.0)
        self.assertIsNone(base.quality)
        self.assertIs(resolve_selection(_SELECTION), base)
        self.assertEqual(resolve_selection(_SELECTION, quality=3.0).quality, 3.0)


if __name__ == "__main__":
    unittest.main()
