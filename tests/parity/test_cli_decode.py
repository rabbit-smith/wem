"""The decode direction on the command line, run the way a caller runs it.

``wwise-wem INPUT.wem --decode --output OUTPUT.wav`` is exercised through
``python -m wwise_wem`` — the console script's own entry point — against the
committed paired-build container, and the WAV it writes is compared live
against the WAV that container was encoded from. Two contracts live here
because the library alone cannot state them: the output is a *whole* file (a
refused session hands back a prefix, and the command line must not present that
prefix as one), and the samples it writes are the package's own conversion of
the library's decode, with no numerics of the command line's own.

The invocation shape is pinned too: ``--decode`` is a mode flag, so the
positional argument stays a file name in both directions and a WAV called
``decode`` still encodes. The encode half of the command line is in
``tests/parity/test_public_surface.py``.
"""

from __future__ import annotations

import array
import ast
import os
import re
import subprocess
import struct
import sys
import tempfile
import unittest
import wave

import wwise_wem

from pathlib import Path

from wwise_wem.adapters.sample_conversion import float_to_int16
from wwise_wem.adapters.wav import read_pcm_wav
from wwise_wem.adapters.wav_writer import pcm16_wav_bytes

ROOT = Path(__file__).resolve().parents[2]
FIXTURES = ROOT / "tests" / "fixtures"
REFERENCE = FIXTURES / "reference.wem"
INPUT = FIXTURES / "input.wav"

# What the committed container declares, and what its source WAV holds.
CHANNELS = 6
SAMPLE_RATE = 44_100
FRAMES = 139_398
SAMPLE_WIDTH = 2
WAV_HEADER_BYTES = 44


def _run_cli(*arguments: str, cwd: Path | None = None) -> subprocess.CompletedProcess[str]:
    """The console script's entry point, as a caller invokes it.

    ``PYTHONPATH`` is set to absolute paths rather than inherited, because one
    case runs the command line from a directory that holds a file called
    ``decode`` — which is the point of that case, not an accident.
    """
    environment = dict(os.environ)
    environment["PYTHONPATH"] = os.pathsep.join(
        (str(ROOT / "src"), str(ROOT / "reference"))
    )
    return subprocess.run(
        [sys.executable, "-m", "wwise_wem", *arguments],
        cwd=cwd or ROOT,
        env=environment,
        capture_output=True,
        text=True,
    )


def _read_wav(path: Path) -> tuple[int, int, int, int, list[int]]:
    """``(channels, sample_rate, frames, sample_width, samples)`` of one WAV.

    Read with the standard library's reader, so the file is checked by
    something that is not the writer: a header only its author accepts would
    not be a WAV.
    """
    with wave.open(str(path), "rb") as handle:
        params = handle.getparams()
        raw = handle.readframes(handle.getnframes())
    samples = array.array("h")
    samples.frombytes(raw)
    if sys.byteorder != "little":
        samples.byteswap()
    return (
        params.nchannels,
        params.framerate,
        params.nframes,
        params.sampwidth,
        list(samples),
    )


def _source_samples() -> list[int]:
    """The fixture WAV's interleaved signed-16 samples."""
    return _read_wav(INPUT)[4]


def _report(stdout: str) -> dict[str, object]:
    """The dictionary the command line prints after the ``OK`` label."""
    return ast.literal_eval(stdout[stdout.index("{") :])


class DecodeCommandTests(unittest.TestCase):
    """What the decode invocation writes, reports, and refuses."""

    def test_the_decode_form_writes_a_signed16_wav_of_the_containers_geometry(
        self,
    ) -> None:
        with tempfile.TemporaryDirectory() as directory:
            output = Path(directory) / "decoded.wav"
            result = _run_cli(str(REFERENCE), "--decode", "--output", str(output))
            self.assertEqual(result.returncode, 0, result.stderr)
            channels, sample_rate, frames, width, samples = _read_wav(output)
            self.assertEqual(output.stat().st_size, WAV_HEADER_BYTES + len(samples) * 2)
        self.assertEqual(
            (channels, sample_rate, frames, width),
            (CHANNELS, SAMPLE_RATE, FRAMES, SAMPLE_WIDTH),
        )
        self.assertEqual(len(samples), FRAMES * CHANNELS)

    def test_the_report_names_the_declared_geometry_and_what_was_written(self) -> None:
        with tempfile.TemporaryDirectory() as directory:
            output = Path(directory) / "decoded.wav"
            result = _run_cli(str(REFERENCE), "--decode", "--output", str(output))
            self.assertIn("WEM to WAV OK", result.stdout)
            report = _report(result.stdout)
            written = output.stat().st_size
        self.assertEqual(report["channels"], CHANNELS)
        self.assertEqual(report["sample_rate"], SAMPLE_RATE)
        # The frame count is the container's own declaration; a successful
        # decode delivers exactly it (docs/reference/decoding.md).
        self.assertEqual(report["frames"], FRAMES)
        # The byte count is the command line's own count of the file it holds,
        # as the encode report's is.
        self.assertEqual(report["bytes"], written)
        self.assertEqual(report["bytes"], WAV_HEADER_BYTES + FRAMES * CHANNELS * 2)

    def test_the_wav_is_the_library_decode_mapped_by_the_packages_own_rule(
        self,
    ) -> None:
        # The command line maps samples with the package's own float-to-int16
        # rule and holds no numerics of its own: the comparison is exact, so a
        # second conversion written into the CLI would show up here.
        expected = [
            float_to_int16(sample)
            for block in wwise_wem.decode(REFERENCE)
            for sample in block
        ]
        with tempfile.TemporaryDirectory() as directory:
            output = Path(directory) / "decoded.wav"
            result = _run_cli(str(REFERENCE), "--decode", "--output", str(output))
            self.assertEqual(result.returncode, 0, result.stderr)
            samples = _read_wav(output)[4]
        self.assertEqual(len(samples), len(expected))
        self.assertEqual(samples, expected)

    def test_the_reconstruction_stands_against_the_wav_it_was_encoded_from(
        self,
    ) -> None:
        source = _source_samples()
        with tempfile.TemporaryDirectory() as directory:
            output = Path(directory) / "decoded.wav"
            result = _run_cli(str(REFERENCE), "--decode", "--output", str(output))
            self.assertEqual(result.returncode, 0, result.stderr)
            decoded = _read_wav(output)[4]

        self.assertEqual(
            len(decoded),
            len(source),
            "the decode and its source must cover the same interleaved stream",
        )
        differences = [abs(left - right) for left, right in zip(decoded, source)]
        peak = max(differences)
        rms = (sum(value * value for value in differences) / len(differences)) ** 0.5
        baseline = max(abs(sample) for sample in source)
        print(
            f"cli decode: {len(source) // CHANNELS} frames x {CHANNELS}ch, "
            f"max|error| {peak} of 32768 ({peak / baseline * 100.0:.3f}% of the "
            f"source peak {baseline}), rms {rms:.2f}"
        )
        # The same structural bound the library-level comparison uses, in the
        # signed-16 domain the file carries: a command line that misaligned the
        # stream, dropped a channel or wrote the wrong conversion fails here.
        self.assertLessEqual(
            peak,
            2 * baseline,
            f"the decoded WAV is not a reconstruction of the source: max|error| "
            f"{peak} against a source peak of {baseline}",
        )

    def test_a_refused_decode_writes_nothing_and_says_what_it_left_behind(
        self,
    ) -> None:
        # A session refused part way through has delivered a prefix of the
        # declared frame count. Writing those frames would present the prefix
        # as a whole file, so the command line must not create its output — not
        # even an empty file, and not the directory it would live in.
        with tempfile.TemporaryDirectory() as directory:
            half = Path(directory) / "half.wem"
            half.write_bytes(REFERENCE.read_bytes()[: REFERENCE.stat().st_size // 2])
            output = Path(directory) / "nested" / "decoded.wav"
            result = _run_cli(str(half), "--decode", "--output", str(output))
            self.assertNotEqual(result.returncode, 0, result.stdout)
            self.assertFalse(output.exists(), "a refused decode wrote a file")
            self.assertFalse(output.parent.exists(), "a refused decode made a directory")
        self.assertIn("nothing written", result.stderr)
        self.assertRegex(
            result.stderr,
            re.compile(r"refused after \d+ of 139398 declared frames"),
            "the refusal must say how much of the declared stream it left behind",
        )
        self.assertNotIn("WEM to WAV OK", result.stdout)

    def test_encoder_options_are_refused_in_decode_mode(self) -> None:
        # A WEM is self-describing: there is nothing for these to select or
        # assert, and ignoring one would run a command the caller did not
        # write.
        with tempfile.TemporaryDirectory() as directory:
            output = Path(directory) / "unused.wav"
            for option, value in (
                ("--quality", "5"),
                ("--wwise-version", "2013"),
                ("--channels", "6"),
                ("--sample-rate", "44100"),
            ):
                with self.subTest(option=option):
                    result = _run_cli(
                        str(REFERENCE),
                        "--decode",
                        option,
                        value,
                        "--output",
                        str(output),
                    )
                    self.assertEqual(result.returncode, 2, result.stderr)
                    self.assertIn("encoder options given", result.stderr)
                    self.assertFalse(output.exists())

    def test_a_file_named_decode_is_still_a_file(self) -> None:
        # The shape decision, pinned: `--decode` is a flag, not a subcommand,
        # so the positional argument is a file name in both directions. A WAV
        # called `decode` encodes, and encodes byte-exactly.
        with tempfile.TemporaryDirectory() as directory:
            work = Path(directory)
            (work / "decode").write_bytes(INPUT.read_bytes())
            output = work / "encoded.wem"
            result = _run_cli("decode", "--output", str(output), cwd=work)
            self.assertEqual(result.returncode, 0, result.stderr)
            self.assertEqual(output.read_bytes(), REFERENCE.read_bytes())

    def test_an_encode_invocation_is_unchanged_without_the_flag(self) -> None:
        with tempfile.TemporaryDirectory() as directory:
            output = Path(directory) / "encoded.wem"
            result = _run_cli(str(INPUT), "--output", str(output))
            self.assertEqual(result.returncode, 0, result.stderr)
            self.assertIn("WAV to WEM OK", result.stdout)
            self.assertEqual(output.read_bytes(), REFERENCE.read_bytes())


class WavWriterTests(unittest.TestCase):
    """The adapter the command line hands its samples to."""

    def test_the_header_is_a_canonical_44_byte_riff_header(self) -> None:
        wav = pcm16_wav_bytes(2, 44_100, [[0.0, 0.0, 0.0, 0.0]])
        self.assertEqual(
            struct.unpack("<4sI4s4sIHHIIHH4sI", wav[:WAV_HEADER_BYTES]),
            (
                b"RIFF",
                36 + 8,
                b"WAVE",
                b"fmt ",
                16,
                1,
                2,
                44_100,
                44_100 * 2 * 2,
                2 * 2,
                16,
                b"data",
                8,
            ),
        )

    def test_what_the_writer_writes_is_what_the_encoder_side_reads(self) -> None:
        # The writer and the encode direction's reader are two halves of one
        # contract, and the reader is the older half: a decoded WAV has to be
        # readable by the command line that encodes.
        samples = [0, 1, -1, -32768, 32767, 12345, -12345, 300]
        wav = pcm16_wav_bytes(2, 44_100, [[sample / 32768.0 for sample in samples]])
        with tempfile.TemporaryDirectory() as directory:
            path = Path(directory) / "round-trip.wav"
            path.write_bytes(wav)
            pcm = read_pcm_wav(path)
        self.assertEqual(pcm.sample_rate, 44_100)
        self.assertEqual(pcm.channel_count, 2)
        self.assertEqual(pcm.frame_count, 4)
        for index, sample in enumerate(samples):
            frame, channel = divmod(index, 2)
            self.assertEqual(
                pcm.channels[channel][frame],
                sample / 32768.0,
                f"sample {index} did not survive the round trip",
            )

    def test_the_conversion_is_the_packages_own_rule(self) -> None:
        # The boundaries `sample_conversion.float_to_int16` documents, so the
        # command line's WAV and the library's decode agree by construction.
        wav = pcm16_wav_bytes(
            1,
            44_100,
            [[0.0, 1.0, -1.0, 2.0, -2.0, 0.5 / 32768.0, -0.5 / 32768.0]],
        )
        samples = struct.unpack("<7h", wav[WAV_HEADER_BYTES:])
        self.assertEqual(samples, (0, 32767, -32768, 32767, -32768, 1, -1))

    def test_a_non_finite_sample_is_refused_rather_than_invented(self) -> None:
        for sample in (float("nan"), float("inf"), float("-inf")):
            with self.subTest(sample=sample):
                with self.assertRaises(ValueError):
                    pcm16_wav_bytes(1, 44_100, [[sample]])

    def test_a_block_that_is_not_whole_frames_is_refused(self) -> None:
        with self.assertRaisesRegex(ValueError, "whole number of 2-channel frames"):
            pcm16_wav_bytes(2, 44_100, [[0.0, 0.0, 0.0]])

    def test_impossible_geometry_is_refused(self) -> None:
        with self.assertRaises(ValueError):
            pcm16_wav_bytes(0, 44_100, [])
        with self.assertRaises(ValueError):
            pcm16_wav_bytes(2, 0, [])


if __name__ == "__main__":
    unittest.main()
