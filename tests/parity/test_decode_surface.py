"""What a caller sees when decoding: the exported function, its result object,
and the reconstruction it produces against the artifact it came from.

``tests/fixtures/reference.wem`` is real paired-build 6ch/44100 output and
``tests/fixtures/input.wav`` is the source it was produced from, so the
comparison cases measure the decode against material this repository did not
produce. That comparison is live and relative — the reconstruction's error
against the source's own peak — and never a recorded threshold
(docs/reference/standards.md, Bit-exactness).

Three angles, one object: the surface cases pin what the call returns and when
it can fail, the source cases pin which inputs reach it, and the comparison
cases pin that what comes out is the sound that went in.
"""

from __future__ import annotations

import array
import contextlib
import inspect
import struct
import sys
import unittest
import wave
import wwise_wem

from pathlib import Path

ROOT = Path(__file__).resolve().parents[2]
FIXTURES = ROOT / "tests" / "fixtures"
REFERENCE = FIXTURES / "reference.wem"
INPUT = FIXTURES / "input.wav"

# The delivery block size of the C ABI's PCM callback, which the decode shell
# mirrors: no block a caller is handed is longer than this.
MAX_BLOCK_FRAMES = 1024


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
    """The reconstruction, compared live against the material it came from."""

    def test_the_real_wem_decodes_to_its_source(self) -> None:
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

        # A structural bound, not a quality threshold: the codec's own loss is
        # far below the source's full scale, so a decoder that misread the
        # bitstream or misaligned the output is caught here at once.
        self.assertLessEqual(
            peak,
            2.0 * baseline,
            f"the decode is not a reconstruction of the source: max|error| "
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


if __name__ == "__main__":
    unittest.main()
