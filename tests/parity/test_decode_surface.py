"""What a caller sees when decoding: the exported function, its result object,
and the reconstruction it produces against the artifact it came from.

``tests/fixtures/reference.wem`` is real paired-build 6ch/44100 output and
``tests/fixtures/input.wav`` is the source it was produced from, so the
comparison cases measure the decode against material this repository did not
produce. Source geometry and magnitude checks are complemented by a live
comparison with a separate NumPy synthesis on both carried geometries, using
the explicit tolerances documented in docs/reference/decoding.md.

Three angles, one object: the surface cases pin what the call returns and when
it can fail, the source cases pin which inputs reach it, and the comparison
cases pin that what comes out is the sound that went in.
"""

from __future__ import annotations

import array
import contextlib
import importlib.util
import inspect
import io
import struct
import sys
import unittest
import wave
import wwise_wem
import numpy as np
from wwise_wem_reference.profiles.artifact import resolve_selection

from pathlib import Path
from scripts.check_decode_external import compare_pcm

ROOT = Path(__file__).resolve().parents[2]
FIXTURES = ROOT / "tests" / "fixtures"
REFERENCE = FIXTURES / "reference.wem"
INPUT = FIXTURES / "input.wav"
STEREO_REFERENCE = ROOT / "tests" / "data" / "2ch-reference" / "stereo_noise.wem"

# The delivery block size of the C ABI's PCM callback, which the decode shell
# mirrors: no block a caller is handed is longer than this.
MAX_BLOCK_FRAMES = 1024


def _load_reference_decoder():
    """Load the independent NumPy decoder without making it a package surface."""
    spec = importlib.util.spec_from_file_location("decode_wem", ROOT / "scripts" / "decode_wem.py")
    if spec is None or spec.loader is None:
        raise ImportError("scripts/decode_wem.py is required for decode comparison")
    module = importlib.util.module_from_spec(spec)
    sys.modules[spec.name] = module
    spec.loader.exec_module(module)
    return module


REFERENCE_DECODER = _load_reference_decoder()


def _channel_major(samples: list[float], channels: int) -> np.ndarray:
    """Turn the facade's interleaved samples into ``(channel, frame)`` PCM."""
    pcm = np.asarray(samples, dtype=np.float64)
    if pcm.size % channels:
        raise AssertionError(f"{pcm.size} samples do not contain whole {channels}ch frames")
    return pcm.reshape(-1, channels).T


def _assert_decode_quality(
    test: unittest.TestCase,
    native: np.ndarray,
    reference: np.ndarray,
    *,
    case: str,
) -> None:
    """Reject a non-finite, misaligned, gain/sign, or channel-broken decode.

    The independent reference is a deterministic float64 NumPy synthesis.  Its
    generic 6ch path evaluates the IMDCT definition and derives its hybrid
    window from the mode geometry.  Against the live fixture its largest
    normalized RMS difference is below 2e-7, so 1e-5 is a numerical margin,
    not an exactness or "no worse" claim.  Correlation 0.999999 requires the
    time-varying waveform to remain aligned. Together they reject zero output,
    material gain/sign changes, channel permutation, and one-frame shifts.
    """
    test.assertEqual(native.shape, reference.shape, f"{case}: PCM shape")
    test.assertTrue(np.isfinite(native).all(), f"{case}: native PCM is non-finite")
    test.assertTrue(np.isfinite(reference).all(), f"{case}: reference PCM is non-finite")
    for channel, (actual, expected) in enumerate(zip(native, reference)):
        expected_rms = float(np.sqrt(np.mean(expected * expected)))
        if expected_rms == 0.0:
            test.assertTrue(
                np.array_equal(actual, expected),
                f"{case}: channel {channel} is silent in the reference but differs",
            )
            continue
        difference = actual - expected
        normalized_rms = float(np.sqrt(np.mean(difference * difference)) / expected_rms)
        centered_actual = actual - np.mean(actual)
        centered_expected = expected - np.mean(expected)
        actual_energy = float(np.dot(centered_actual, centered_actual))
        expected_energy = float(np.dot(centered_expected, centered_expected))
        test.assertGreater(actual_energy, 0.0, f"{case}: channel {channel} has no variation")
        test.assertGreater(
            expected_energy, 0.0, f"{case}: reference channel {channel} has no variation"
        )
        correlation = float(
            np.dot(centered_actual, centered_expected)
            / np.sqrt(actual_energy * expected_energy)
        )
        test.assertLessEqual(
            normalized_rms,
            1.0e-5,
            f"{case}: channel {channel} normalized RMS error {normalized_rms:.6g}",
        )
        test.assertGreaterEqual(
            correlation,
            0.999999,
            f"{case}: channel {channel} correlation {correlation:.6g}",
        )


def _source_pcm() -> tuple[int, int, list[float]]:
    """The fixture WAV as ``(channels, sample_rate, interleaved f32 samples)``.

    The samples are the encoder's own input domain, ``value / 32768.0``, so the
    decode's output sample *i* and this list's sample *i* are the same position
    in the stream.
    """
    with wave.open(str(INPUT), "rb") as handle:
        channels = handle.getnchannels()
        sample_rate = handle.getframerate()
        raw = handle.readframes(handle.getnframes())
    values = array.array("h")
    values.frombytes(raw)
    if sys.byteorder != "little":
        values.byteswap()
    return channels, sample_rate, [value / 32768.0 for value in values]


def _blocks(result) -> list[list[float]]:
    """Every block a decode result hands over, in order."""
    return list(result)


def _samples(result) -> list[float]:
    """Every decoded sample, in interleaved order."""
    return [sample for block in result for sample in block]


def _flatten(blocks: list[list[float]]) -> list[float]:
    return [sample for block in blocks for sample in block]


def _peak(samples: list[float]) -> float:
    return max((abs(sample) for sample in samples), default=0.0)


class DecodeSurfaceTests(unittest.TestCase):
    """The shape of the call and of what it returns."""

    def test_decode_is_a_plain_function_not_a_generator_function(self) -> None:
        # A generator function's body does not run until the first next(), so a
        # rejection would surface at iteration time; the facade documents
        # call-time errors, and the header parse is what makes that possible.
        self.assertFalse(
            inspect.isgeneratorfunction(wwise_wem.decode),
            "decode must be a plain function: a generator defers its body, and "
            "with it every call-time rejection",
        )
        result = wwise_wem.decode(REFERENCE.read_bytes())
        self.assertFalse(
            inspect.isgenerator(result),
            "the result object is an iterable value type, not a generator",
        )
        self.assertEqual(type(result).__name__, "DecodeResult")
        result.close()

    def test_geometry_is_readable_before_the_first_block(self) -> None:
        channels, sample_rate, samples = _source_pcm()
        result = wwise_wem.decode(REFERENCE.read_bytes())
        with contextlib.closing(result):
            # Nothing has been iterated yet: the container's own declaration is
            # already here, and it is the source's own geometry.
            self.assertEqual(result.channels, channels)
            self.assertEqual(result.sample_rate, sample_rate)
            self.assertEqual(result.total_frames, len(samples) // channels)

    def test_iteration_delivers_the_declared_frames_in_bounded_blocks(self) -> None:
        channels, _rate, samples = _source_pcm()
        frames = 0
        count = 0
        with contextlib.closing(wwise_wem.decode(REFERENCE.read_bytes())) as result:
            for block in result:
                self.assertIsInstance(block, list)
                self.assertEqual(
                    len(block) % channels,
                    0,
                    "a block must interleave whole frames",
                )
                self.assertLessEqual(
                    len(block) // channels,
                    MAX_BLOCK_FRAMES,
                    "a block is bounded by the C ABI's delivery size",
                )
                frames += len(block) // channels
                count += 1
            self.assertEqual(
                frames,
                result.total_frames,
                "a successful decode delivers exactly the declared frame count",
            )
            self.assertEqual(frames, len(samples) // channels)
        self.assertGreater(count, 1, "the stream arrives in more than one block")

    def test_the_result_is_a_one_shot_iterator_like_a_generator(self) -> None:
        result = wwise_wem.decode(REFERENCE.read_bytes())
        with contextlib.closing(result):
            self.assertIs(iter(result), result)
            first = len(_blocks(result))
            self.assertGreater(first, 0)
            self.assertEqual(_blocks(result), [], "a consumed result is exhausted")

    def test_close_releases_the_session_and_is_idempotent(self) -> None:
        result = wwise_wem.decode(REFERENCE.read_bytes())
        result.close()
        result.close()
        with self.assertRaises(StopIteration):
            next(result)
        # The announced geometry stays readable: it is the container's
        # declaration, not the session's state.
        self.assertEqual(result.channels, 6)

    def test_no_session_type_enters_the_package_root(self) -> None:
        # decode is the only new export; the caller never sees Init, push or
        # Finish. The extension's session and step types stay internal.
        self.assertIn("decode", wwise_wem.__all__)
        for name in ("Decoder", "DecodeStep", "DecodedHeader", "DecodeResult"):
            self.assertNotIn(
                name,
                wwise_wem.__all__,
                f"{name} is an implementation detail of decode, not an export",
            )


class DecodeSourceTests(unittest.TestCase):
    """The accepted input forms, and where their errors surface."""

    def test_bytes_bytearray_and_memoryview_decode_identically(self) -> None:
        raw = REFERENCE.read_bytes()
        expected = _samples(wwise_wem.decode(raw))
        for label, source in (
            ("bytearray", bytearray(raw)),
            ("memoryview", memoryview(raw)),
        ):
            with self.subTest(source=label):
                self.assertEqual(_samples(wwise_wem.decode(source)), expected)

    def test_a_path_is_read_whole_at_call_time(self) -> None:
        expected = _samples(wwise_wem.decode(REFERENCE.read_bytes()))
        self.assertEqual(_samples(wwise_wem.decode(REFERENCE)), expected)
        self.assertEqual(_samples(wwise_wem.decode(str(REFERENCE))), expected)

    def test_a_missing_path_keeps_its_standard_oserror(self) -> None:
        missing = FIXTURES / "no-such-file.wem"
        with self.assertRaises(FileNotFoundError):
            wwise_wem.decode(missing)
        # The error is the file system's, raised where `encode` raises it too.
        self.assertTrue(issubclass(FileNotFoundError, OSError))

    def test_an_unsupported_source_type_is_a_type_error(self) -> None:
        for source in (None, 17, 1.5, ["x"], object()):
            with self.subTest(source=source):
                with self.assertRaises(TypeError):
                    wwise_wem.decode(source)  # type: ignore[arg-type]


class DecodeErrorTests(unittest.TestCase):
    """One case per place a decode can be refused, and what the caller holds."""

    def assertKernelError(self, error: Exception, code: str, case: str) -> None:
        self.assertIsInstance(
            error,
            wwise_wem.WwiseWemError,
            f"{case}: expected WwiseWemError at the facade boundary, got {type(error)!r}",
        )
        self.assertIsInstance(
            error,
            ValueError,
            f"{case}: expected a ValueError subclass so `except ValueError` keep working",
        )
        cause = error.__cause__
        self.assertIsInstance(
            cause,
            wwise_wem._core.WemEncoderError,
            f"{case}: the kernel error must stay reachable as __cause__, got {cause!r}",
        )
        self.assertEqual(error.code, cause.code, f"{case}: the facade reported a rewritten code")
        self.assertEqual(error.code, code, f"{case}: expected {code!r}")
        self.assertEqual(
            str(error),
            str(cause),
            f"{case}: the facade rewrote the kernel's message {str(cause)!r}",
        )

    def test_unparseable_input_is_refused_by_the_call_itself(self) -> None:
        with self.assertRaises(wwise_wem.WwiseWemError) as caught:
            wwise_wem.decode(b"NOTARIFF" + b"\0" * 8)
        self.assertKernelError(caught.exception, "INPUT_MALFORMED", "not a RIFF container")

    def test_an_empty_source_is_refused_by_the_call_itself(self) -> None:
        with self.assertRaises(wwise_wem.WwiseWemError) as caught:
            wwise_wem.decode(b"")
        self.assertKernelError(caught.exception, "INPUT_MALFORMED", "no bytes at all")

    def test_an_uninstalled_geometry_is_unsupported_not_malformed(self) -> None:
        # The container still parses; the fmt chunk names a geometry this build
        # does not carry, which is the unsupported class (include/wem.h
        # section 5). Three channels at the fixture's rate is not installed.
        raw = bytearray(REFERENCE.read_bytes())
        fmt = raw.find(b"fmt ") + 8
        raw[fmt + 0x02 : fmt + 0x04] = struct.pack("<H", 3)
        with self.assertRaises(wwise_wem.WwiseWemError) as caught:
            wwise_wem.decode(bytes(raw))
        self.assertKernelError(caught.exception, "FORMAT_UNSUPPORTED", "3ch/44100 container")

    def test_a_truncated_container_delivers_its_prefix_then_reports(self) -> None:
        # The header region is inside the first block, so the call resolves the
        # geometry; the data payload the bytes promise never arrives, and the
        # refusal is reported by the iteration step that reaches the end —
        # after the frames the packets before it completed.
        raw = REFERENCE.read_bytes()
        result = wwise_wem.decode(raw[: len(raw) // 2])
        delivered = 0
        with contextlib.closing(result):
            with self.assertRaises(wwise_wem.WwiseWemError) as caught:
                for block in result:
                    delivered += len(block)
            self.assertKernelError(
                caught.exception,
                "INPUT_MALFORMED",
                "container cut in half",
            )
            self.assertGreater(delivered, 0, "the completed prefix is delivered, not dropped")
            self.assertLess(
                delivered // result.channels,
                result.total_frames,
                "a refused decode must not look like a complete one",
            )
            with self.assertRaises(StopIteration):
                next(result)

    def test_a_partial_container_that_never_resolves_is_refused_by_the_call(self) -> None:
        # Fewer bytes than the RIFF/WAVE header: no geometry is ever announced,
        # so the rejection belongs to the call and not to iteration.
        with self.assertRaises(wwise_wem.WwiseWemError) as caught:
            wwise_wem.decode(b"RIFF\x00\x00\x00\x00")
        self.assertKernelError(caught.exception, "INPUT_MALFORMED", "12 bytes of RIFF")


class DecodeComparisonTests(unittest.TestCase):
    """Source geometry, bounded output, and repeatability of the same decode."""

    def test_the_real_wem_preserves_source_geometry_and_has_bounded_output(self) -> None:
        channels, sample_rate, source = _source_pcm()
        with contextlib.closing(wwise_wem.decode(REFERENCE.read_bytes())) as result:
            self.assertEqual(result.channels, channels)
            self.assertEqual(result.sample_rate, sample_rate)
            decoded = _samples(result)

        self.assertEqual(
            len(decoded),
            len(source),
            "the decode and its source must cover the same interleaved stream",
        )
        differences = [abs(a - b) for a, b in zip(decoded, source)]
        peak = max(differences, default=0.0)
        rms = (sum(d * d for d in differences) / len(differences)) ** 0.5
        baseline = _peak(source)
        print(
            f"decode: {len(source) // channels} frames x {channels}ch, "
            f"max|error| {peak:.6e} ({peak / baseline * 100.0:.3f}% of the "
            f"source peak {baseline:.6}), rms {rms:.6e}"
        )

        # This only rejects gross magnitudes; silence can pass. The separate
        # NumPy comparison below checks the waveform and its alignment.
        self.assertLessEqual(
            peak,
            2.0 * baseline,
            f"the decode exceeds its magnitude bound: max|error| "
            f"{peak} against a source peak of {baseline}",
        )

    def test_decoding_our_own_encode_is_the_same_decode(self) -> None:
        # The encoder is bit-exact against the paired build, so this is the
        # same measurement over a container this repository produced itself:
        # the two decodes must agree exactly, sample for sample.
        ours = wwise_wem.encode(INPUT)
        asserted = _blocks(wwise_wem.decode(ours.data))
        reference = _blocks(wwise_wem.decode(REFERENCE.read_bytes()))
        self.assertEqual(
            len(asserted),
            len(reference),
            "the two containers decoded to different block counts",
        )
        for index, (left, right) in enumerate(zip(asserted, reference)):
            self.assertEqual(
                left,
                right,
                f"block {index} differs between decode(reference.wem) and "
                f"decode(encode(input.wav))",
            )

    def test_decoding_is_deterministic(self) -> None:
        raw = REFERENCE.read_bytes()
        self.assertEqual(
            _flatten(_blocks(wwise_wem.decode(raw))),
            _flatten(_blocks(wwise_wem.decode(raw))),
            "two decodes of the same bytes must be identical",
        )


class DecodeReferenceQualityTests(unittest.TestCase):
    """Live native-versus-NumPy comparisons over both carried geometries."""

    def _compare_paired_wem(
        self, path: Path, channels: int, sample_rate: int
    ) -> tuple[np.ndarray, np.ndarray]:
        native_result = wwise_wem.decode(path.read_bytes())
        with contextlib.closing(native_result):
            self.assertEqual(native_result.channels, channels)
            self.assertEqual(native_result.sample_rate, sample_rate)
            native = _channel_major(_samples(native_result), channels)
            frames = native_result.total_frames
        selection = wwise_wem.WwiseProfile(
            wwise_wem.WwiseVersion.WWISE2013, channels, sample_rate
        )
        profile = resolve_selection(selection)
        context = REFERENCE_DECODER.build_context(profile, channels, sample_rate)
        reference, diagnostics = REFERENCE_DECODER.decode_numpy_pcm(
            context, path.read_bytes(), frames
        )
        self.assertTrue(diagnostics["bit_closure_ok"], f"{path.name}: packet bit closure")
        self.assertTrue(diagnostics["strict_closure_ok"], f"{path.name}: strict packet closure")
        _assert_decode_quality(self, native, reference, case=path.name)
        return native, reference

    def test_paired_build_wems_agree_with_the_independent_numpy_decoder(self) -> None:
        # These are external paired-build containers, one for each registered
        # geometry.  The 2ch signal has independent noise in both channels;
        # together with the fixture it exercises channel order, sign, gain,
        # origin alignment, finite samples, and both synthesis paths.
        for path, channels, sample_rate in (
            (REFERENCE, 6, 44_100),
            (STEREO_REFERENCE, 2, 48_000),
        ):
            with self.subTest(wem=path.name):
                self._compare_paired_wem(path, channels, sample_rate)

    def test_quality_predicate_rejects_defective_pcm(self) -> None:
        # This deterministic synthetic waveform keeps the mutation proof cheap:
        # the live paired-build test above already proves the predicate accepts
        # a real native/reference pair.  Each defect maps to a regression class
        # that pairwise comparison must catch.
        reference = np.random.default_rng(314159).standard_normal((2, 4096))
        defects = {
            "zero": np.zeros_like(reference),
            "gain": reference * 2.0,
            "sign": -reference,
            "channel-order": reference[::-1].copy(),
            "one-frame-offset": np.roll(reference, 1, axis=1),
            "non-finite": np.full_like(reference, np.nan),
            "missing-frame": reference[:, :-1],
        }
        for name, defective in defects.items():
            with self.subTest(defect=name), self.assertRaises(AssertionError):
                _assert_decode_quality(self, defective, reference, case=name)

    def test_long_synthesis_window_uses_short_transition_spans(self) -> None:
        # This small long/short/long seam guards the hybrid support geometry
        # independently of a carried stream's packet schedule.
        block = np.ones(2048, dtype=np.float64)
        halves = {
            256: np.linspace(0.01, 1.0, 128),
            2048: np.linspace(0.001, 1.0, 1024),
        }
        actual = REFERENCE_DECODER.apply_hybrid_synthesis_window(
            block, (256, 2048), 0, 1, 0, halves
        )
        expected = np.zeros(2048, dtype=np.float64)
        expected[448:576] = halves[256]
        expected[576:1472] = 1.0
        expected[1472:1600] = halves[256][::-1]
        self.assertTrue(np.array_equal(actual, expected))


class ExternalDecodeComparisonTests(unittest.TestCase):
    def compare(self, samples, payload=None, *, channels=2, rate=48000):
        class Blocks(list):
            channels = 2
            sample_rate = 48000
            total_frames = 2

        if payload is None:
            payload = struct.pack("<4f", 0.25, -0.5, 0.125, -0.25)
        fmt = struct.pack("<HHIIHH", 3, channels, rate, rate * channels * 4, channels * 4, 32)
        wav = (b"RIFF" + struct.pack("<I", 36 + len(payload)) + b"WAVEfmt "
               + struct.pack("<I", 16) + fmt + b"data" + struct.pack("<I", 16) + payload)
        return compare_pcm(Blocks(samples), io.BytesIO(wav), atol=1e-6, rtol=1e-6)

    def test_external_pcm_comparison_accepts_different_delivery_block_sizes(self):
        metrics = self.compare([[0.25, -0.5], [0.125, -0.25]])
        self.assertEqual(metrics["frames"], 2)
        self.assertEqual(metrics["maximum_error"], 0.0)
        self.assertEqual(metrics["rms_error"], 0.0)

    def test_external_pcm_comparison_refuses_bad_samples_and_framing(self):
        correct = [[0.25, -0.5, 0.125, -0.25]]
        payload = struct.pack("<4f", *correct[0])
        for name, samples, overrides in (
            ("geometry", correct, {"channels": 1}),
            ("rate", correct, {"rate": 44100}),
            ("truncated reference", correct, {"payload": payload[:-4]}),
            ("extra reference", correct, {"payload": payload + b"x"}),
            ("short native", [[0.25, -0.5]], {}),
            ("native non-finite", [[0.25, float("nan"), 0.125, -0.25]], {}),
            ("reference non-finite", correct, {"payload": struct.pack("<4f", float("inf"), 0, 0, 0)}),
            ("sample mismatch", [[0.25, -0.5, 0.12502, -0.25]], {}),
            ("silence", [[0.0] * 4], {}),
        ):
            with self.subTest(case=name), self.assertRaises(ValueError):
                self.compare(samples, **overrides)


if __name__ == "__main__":
    unittest.main()
