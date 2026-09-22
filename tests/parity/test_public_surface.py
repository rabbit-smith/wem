"""What a caller sees at the boundary: exports, dispatch, and failures.

The public-surface cases drive the documented entry points and the CLI they
back; the typed-API cases pin which form dispatches to which adapter and that
a fresh import keeps the runtime modules lazy; the error cases pin that a
kernel code arrives with its class, its code and its message intact. One
surface, three angles. Each class keeps its own test names and failure
messages.
"""

from __future__ import annotations

import json
import os
import subprocess
import sys
import tempfile
import unittest
import wave
import wwise_wem
import wwise_wem._core as _core

from pathlib import Path
from unittest.mock import patch
from wwise_wem import (
    EncodeResult,
    EncodeStats,
    PcmBuffer,
    RawPcm,
    WwiseProfile,
    WwiseVersion,
    encode,
)
from wwise_wem.application.encoder import Encoder
from wwise_wem.model import WwiseWemError

# --------------------------------------------------------------------------
# merged from tests/parity/test_public_api.py
# --------------------------------------------------------------------------

# Tests for the intentionally supported public surface.
#
# These cover user-visible behavior, not the current internal module
# layout.  Refactors may freely replace the implementation behind this API.

ROOT = Path(__file__).resolve().parents[2]
FIXTURES = Path(__file__).resolve().parents[1] / "fixtures"
INPUT = FIXTURES / "input.wav"
SELECTION = WwiseProfile(WwiseVersion.WWISE2013, 6, 44100)


class PublicEncodeTests(unittest.TestCase):
    @classmethod
    def setUpClass(cls):
        cls.directory = tempfile.TemporaryDirectory()
        cls.short_input = Path(cls.directory.name) / "short.wav"
        with wave.open(str(INPUT), "rb") as source:
            params = source.getparams()
            pcm = source.readframes(4096)
        with wave.open(str(cls.short_input), "wb") as target:
            target.setparams(params)
            target.writeframes(pcm)
        cls.automatic_result = encode(cls.short_input)
        cls.explicit_result = encode(cls.short_input, profile=SELECTION)

    @classmethod
    def tearDownClass(cls):
        cls.directory.cleanup()

    def test_automatic_and_explicit_encoding_are_identical(self):
        self.assertEqual(
            self.automatic_result.data, self.explicit_result.data
        )
        self.assertEqual(
            self.automatic_result.stats, self.explicit_result.stats
        )

    def test_packet_counts_are_internally_consistent(self):
        stats = self.automatic_result.stats
        self.assertEqual(
            stats.short_packets + stats.long_packets,
            stats.audio_packets,
        )

    def test_unsupported_wav_geometry_is_an_error(self):
        # 2ch/44100 is not a registered profile (the 2ch profile is 48000).
        with tempfile.TemporaryDirectory() as directory:
            path = Path(directory) / "stereo.wav"
            with wave.open(str(path), "wb") as target:
                target.setnchannels(2)
                target.setsampwidth(2)
                target.setframerate(44100)
                target.writeframes(b"\0" * (4096 * 2 * 2))
            with self.assertRaisesRegex(ValueError, "2ch/44100Hz") as caught:
                encode(path)
        # The rejection is a kernel one, so it arrives with the kernel's
        # stable code; the ValueError subclass keeps `except ValueError`
        # callers working.
        self.assertIsInstance(caught.exception, WwiseWemError)
        self.assertEqual(caught.exception.code, "PROFILE_NOT_FOUND")
        self.assertIn("2ch/44100Hz", caught.exception.message)

    def test_unsupported_eight_bit_wav_is_an_error(self):
        with tempfile.TemporaryDirectory() as directory:
            path = Path(directory) / "pcm8.wav"
            with wave.open(str(path), "wb") as target:
                target.setnchannels(6)
                target.setsampwidth(1)
                target.setframerate(44100)
                target.writeframes(b"\x80" * (32 * 6))
            with self.assertRaisesRegex(ValueError, "16-bit PCM, 24-bit PCM"):
                encode(path)


class PublicCliTests(unittest.TestCase):
    def run_cli(self, *arguments: str) -> subprocess.CompletedProcess[str]:
        return subprocess.run(
            [sys.executable, "-m", "wwise_wem", *arguments],
            cwd=ROOT,
            capture_output=True,
            text=True,
        )

    def test_help_exposes_supported_options(self):
        result = self.run_cli("--help")
        self.assertEqual(result.returncode, 0, result.stderr)
        self.assertIn("usage:", result.stdout)
        for option in (
            "--wwise-version",
            "--quality",
            "--channels",
            "--sample-rate",
            "--output",
        ):
            self.assertIn(option, result.stdout)
        self.assertNotIn("--profile", result.stdout)

    def test_output_is_required(self):
        result = self.run_cli(str(INPUT))
        self.assertNotEqual(result.returncode, 0)
        self.assertIn("--output", result.stderr)

    def test_cli_geometry_assertion_fails_before_output(self):
        with tempfile.TemporaryDirectory() as directory:
            output = Path(directory) / "unused.wem"
            result = self.run_cli(
                str(INPUT), "--channels", "2", "--output", str(output)
            )
            self.assertNotEqual(result.returncode, 0)
            self.assertIn("differs from WAV", result.stderr)
            self.assertFalse(output.exists())


class PublicImportBoundaryTests(unittest.TestCase):
    def test_fresh_root_import_keeps_encoder_and_codec_runtime_lazy(self):
        code = """
import json
import sys
import wwise_wem
names = [
    'wwise_wem.application.encoder',
    'wwise_wem_reference.python_engine',
    'wwise_wem_reference.analysis.dsp.transform',
    'wwise_wem_reference.analysis.psychoacoustics.pipeline',
    'wwise_wem_reference.vorbis.floor',
    'wwise_wem_reference.vorbis.residue',
    'wwise_wem_reference.analysis.session',
]
print(json.dumps([name for name in names if name in sys.modules]))
"""
        environment = dict(os.environ)
        environment["PYTHONPATH"] = str(ROOT / "src") + os.pathsep + str(ROOT / "reference")
        completed = subprocess.run(
            [sys.executable, "-c", code],
            cwd=ROOT,
            env=environment,
            capture_output=True,
            text=True,
            check=True,
        )
        self.assertEqual(json.loads(completed.stdout), [])


# --------------------------------------------------------------------------
# merged from tests/parity/test_typed_api.py
# --------------------------------------------------------------------------

# Parity tests for the small, import-light package facade.

ROOT = Path(__file__).resolve().parents[2]


def _result() -> EncodeResult:
    return EncodeResult(
        b"RIFF",
        EncodeStats(16, 6, 2, 1, 1),
    )


class TypedApiTests(unittest.TestCase):
    def test_path_input_reads_header_and_forwards_the_selection(self) -> None:
        payload = b"\0\0" * 6
        selection = WwiseProfile(WwiseVersion.WWISE2013, 6, 44100)
        expected = _result()
        with (
            patch(
                "wwise_wem.adapters.wav._read_wav_pcm16_bytes",
                return_value=(44100, 6, payload),
            ) as read,
            patch("wwise_wem.application.encoder.Encoder") as encoder_type,
        ):
            encoder_type.return_value.encode_pcm16_interleaved.return_value = expected
            result = encode(Path("input.audio"), profile=selection, quality=5.0)

        read.assert_called_once_with(Path("input.audio"))
        encoder_type.assert_called_once_with(selection, quality=5.0)
        encoder_type.return_value.encode_pcm16_interleaved.assert_called_once_with(
            payload,
            sample_rate=44100,
            channels=6,
        )
        self.assertIs(result, expected)

    def test_automatic_selection_comes_from_the_input_geometry(self) -> None:
        payload = b"\0\0" * 6
        expected = _result()
        with (
            patch(
                "wwise_wem.adapters.wav._read_wav_pcm16_bytes",
                return_value=(44100, 6, payload),
            ),
            patch("wwise_wem.application.encoder.Encoder") as encoder_type,
        ):
            encoder_type.return_value.encode_pcm16_interleaved.return_value = expected
            result = encode(Path("input.audio"))

        encoder_type.assert_called_once_with(
            WwiseProfile(WwiseVersion.DEFAULT, 6, 44100), quality=None
        )
        self.assertIs(result, expected)

    def test_pcm_buffer_bypasses_input_adapters(self) -> None:
        pcm = PcmBuffer(44100, ((0.0,),) * 6)
        expected = _result()
        with (
            patch("wwise_wem.adapters.wav._read_wav_pcm16_bytes") as read_wav,
            patch("wwise_wem.adapters.raw._normalize_pcm16_bytes") as read_raw,
            patch("wwise_wem.application.encoder.Encoder") as encoder_type,
        ):
            encoder_type.return_value.encode_pcm.return_value = expected
            self.assertIs(encode(pcm), expected)
        read_wav.assert_not_called()
        read_raw.assert_not_called()

    def test_raw_pcm_carries_explicit_format_and_geometry(self) -> None:
        source = RawPcm(b"\0\0" * 6, 44100, 6, "s16le")
        with (
            patch(
                "wwise_wem.adapters.raw._normalize_pcm16_bytes",
                return_value=source.data,
            ) as read,
            patch("wwise_wem.application.encoder.Encoder") as encoder_type,
        ):
            encoder_type.return_value.encode_pcm16_interleaved.return_value = _result()
            encode(source)
        read.assert_called_once_with(
            source.data,
            sample_rate=44100,
            channels=6,
            bits_per_sample=16,
        )
        encoder_type.return_value.encode_pcm16_interleaved.assert_called_once_with(
            source.data,
            sample_rate=44100,
            channels=6,
        )

    def test_untyped_bytes_are_rejected(self) -> None:
        with self.assertRaisesRegex(TypeError, "path, PcmBuffer, or RawPcm"):
            encode(b"\0\0")  # type: ignore[arg-type]

    def test_package_root_has_one_small_public_surface(self) -> None:
        self.assertEqual(
            wwise_wem.__all__,
            [
                "EncodeResult",
                "EncodeStats",
                "PcmBuffer",
                "RawPcm",
                "WwiseProfile",
                "WwiseVersion",
                "WwiseWemError",
                "encode",
            ],
        )

    def test_fresh_package_import_keeps_runtime_modules_lazy(self) -> None:
        code = """
import json
import sys
import wwise_wem
names = [
    'wwise_wem.application.encoder',
    'wwise_wem.adapters.wav',
    'wwise_wem.adapters.raw',
    'wwise_wem_reference.python_engine',
]
print(json.dumps([name for name in names if name in sys.modules]))
"""
        environment = dict(os.environ)
        environment["PYTHONPATH"] = str(ROOT / "src") + os.pathsep + str(ROOT / "reference")
        completed = subprocess.run(
            [sys.executable, "-c", code],
            cwd=ROOT,
            env=environment,
            capture_output=True,
            text=True,
            check=True,
        )
        self.assertEqual(json.loads(completed.stdout), [])


# --------------------------------------------------------------------------
# merged from tests/parity/test_error_surface.py
# --------------------------------------------------------------------------

# The kernel's stable error codes survive the facade boundary.
#
# The native extension raises ``WemEncoderError`` carrying a stable ``.code``
# (``PROFILE_NOT_FOUND``, ``GEOMETRY_MISMATCH``, ``INPUT_TOO_SHORT``,
# ``FORMAT_UNSUPPORTED``, ``STATE_ERROR``, ``INTERNAL``) — the same classes
# every cross-language shell maps.  The facade owns the last hop: a caller must
# be able to branch on that code without digging into ``__cause__``, and the
# kernel's diagnostic text must arrive unchanged.
#
# Every rejection below is produced by calling the kernel with input it refuses;
# no error object is constructed by this suite.  The public entry points
# pre-validate geometry, frame count and byte alignment with their own
# ``ValueError``s, so the cases those checks cover reach the kernel through
# ``Encoder._encode_pcm_core`` — the facade's raise site — with the Python-side
# check bypassed, which is what makes the kernel's own code observable there.

SIX_CHANNEL = WwiseProfile(WwiseVersion.WWISE2013, 6, 44100)
UNINSTALLED = WwiseProfile(WwiseVersion.WWISE2013, 2, 44100)
SIX_CHANNEL_FRAMES = 4096
SIX_CHANNEL_PCM = b"\0" * (6 * 2 * SIX_CHANNEL_FRAMES)


def _buffer(sample_rate: int, channels: int, frames: int) -> PcmBuffer:
    """A buffer of silence in the signed-16 domain."""
    return PcmBuffer(sample_rate, ((0.0,) * frames,) * channels)


def _kernel_error(kernel_call) -> _core.WemEncoderError:
    """The error the kernel itself raises for ``kernel_call``'s input."""
    try:
        kernel_call()
    except _core.WemEncoderError as error:
        return error
    raise AssertionError(
        "the kernel accepted input that must be rejected; "
        "the rejection case no longer drives a real failure"
    )


class FacadeErrorCodeTests(unittest.TestCase):
    """One case per kernel code a Python caller can reach."""

    def assertBoundaryError(self, error, code: str, case: str) -> None:
        """The boundary result: public type, stable code, untouched message."""
        self.assertIsInstance(
            error,
            WwiseWemError,
            f"{case}: expected WwiseWemError at the facade boundary, "
            f"got {type(error).__module__}.{type(error).__name__}",
        )
        self.assertIsInstance(
            error,
            ValueError,
            f"{case}: expected a ValueError subclass so existing "
            f"`except ValueError` callers keep working, got {type(error)!r}",
        )
        cause = error.__cause__
        self.assertIsInstance(
            cause,
            _core.WemEncoderError,
            f"{case}: the kernel error must stay reachable as __cause__, "
            f"got {cause!r}",
        )
        self.assertEqual(
            error.code,
            cause.code,
            f"{case}: the facade reported {error.code!r} for a kernel error "
            f"carrying {cause.code!r}",
        )
        self.assertEqual(
            error.code,
            code,
            f"{case}: kernel code {error.code!r} != expected {code!r}",
        )
        self.assertEqual(
            str(error),
            str(cause),
            f"{case}: the facade rewrote the kernel message "
            f"{str(cause)!r} into {str(error)!r}",
        )

    def test_profile_not_found_code_reaches_the_caller(self):
        # No installed configuration satisfies 2ch/44100, and the facade's
        # own geometry check agrees with the selection, so the rejection is
        # the kernel's resolution failure.
        raw = RawPcm(b"\0" * (2 * 2 * 4096), 44100, 2, "s16le")
        with self.assertRaises(WwiseWemError) as caught:
            encode(raw)
        self.assertBoundaryError(
            caught.exception, "PROFILE_NOT_FOUND", "encode(2ch/44100)"
        )

    def test_geometry_mismatch_code_reaches_the_caller(self):
        encoder = Encoder(SIX_CHANNEL)
        with self.assertRaises(WwiseWemError) as caught:
            encoder._encode_pcm_core(
                _buffer(48000, 6, SIX_CHANNEL_FRAMES),
                [[0] * SIX_CHANNEL_FRAMES] * 6,
            )
        self.assertBoundaryError(
            caught.exception,
            "GEOMETRY_MISMATCH",
            "6ch/44100 encoder fed 6ch/48000 PCM",
        )

    def test_input_too_short_code_reaches_the_caller(self):
        encoder = Encoder(SIX_CHANNEL)
        frames = 4095
        with self.assertRaises(WwiseWemError) as caught:
            encoder._encode_pcm_core(
                _buffer(44100, 6, frames), [[0] * frames] * 6
            )
        self.assertBoundaryError(
            caught.exception,
            "INPUT_TOO_SHORT",
            f"6ch/44100 encoder fed {frames} frames (kernel minimum 4096)",
        )

    def test_state_error_code_reaches_the_caller(self):
        encoder = Encoder(SIX_CHANNEL)
        rows = [[70000] * SIX_CHANNEL_FRAMES] * 6
        with self.assertRaises(WwiseWemError) as caught:
            encoder._encode_pcm_core(_buffer(44100, 6, SIX_CHANNEL_FRAMES), rows)
        self.assertBoundaryError(
            caught.exception,
            "STATE_ERROR",
            "6ch/44100 encoder fed samples outside the signed-16 range",
        )

    def test_internal_code_reaches_the_caller(self):
        # The 6ch/44100 configuration ships no quality-curves resource, so a
        # quality request is a kernel configuration fault, not caller input.
        encoder = Encoder(SIX_CHANNEL, quality=0.5)
        with self.assertRaises(WwiseWemError) as caught:
            encoder.encode_pcm16_interleaved(
                SIX_CHANNEL_PCM, sample_rate=44100, channels=6
            )
        self.assertBoundaryError(
            caught.exception, "INTERNAL", "quality request on 6ch/44100"
        )

    def test_absent_profile_is_still_reported_as_a_plain_message(self):
        # The historical spelling of the same rejection: str() is the kernel
        # diagnostic, so message-matching callers read what they always read.
        with self.assertRaisesRegex(ValueError, "2ch/44100Hz"):
            encode(RawPcm(b"\0" * (2 * 2 * 4096), 44100, 2, "s16le"))

    def test_expected_code_names_are_the_extension_s_own(self):
        # Cross-surface pin: the spellings asserted above are the codes the
        # extension itself reports for the same refusals, so a kernel rename
        # fails here instead of silently passing a stale literal.
        cases = (
            ("PROFILE_NOT_FOUND", lambda: _core.Encoder(UNINSTALLED)),
            (
                "GEOMETRY_MISMATCH",
                lambda: _core.Encoder(SIX_CHANNEL).encode_pcm16_interleaved(
                    44100, 6, b"\0" * 5
                ),
            ),
            (
                "INPUT_TOO_SHORT",
                lambda: _core.Encoder(SIX_CHANNEL).encode_pcm16_interleaved(
                    44100, 6, b"\0" * (6 * 2 * (SIX_CHANNEL_FRAMES - 1))
                ),
            ),
            (
                "STATE_ERROR",
                lambda: _core.Encoder(SIX_CHANNEL).encode_pcm(
                    44100, [[70000] * SIX_CHANNEL_FRAMES] * 6
                ),
            ),
            ("INTERNAL", lambda: _core.Encoder(SIX_CHANNEL, 0.5)),
        )
        for code, call in cases:
            with self.subTest(code=code):
                self.assertEqual(
                    _kernel_error(call).code,
                    code,
                    f"the extension no longer reports {code!r} for this "
                    f"refusal; the facade case above asserts a stale code",
                )

    def test_format_unsupported_has_no_python_reachable_site(self):
        # FORMAT_UNSUPPORTED is raised for a Wwise generation outside this
        # revision's selector, and the extension exposes exactly one
        # generation with no way to name another: no Python call can drive
        # it, so the case is pinned in the kernel's own mapping
        # (crates/wem-python error_to_pyerr) rather than faked here.
        self.assertEqual(
            [version.code for version in _core.WwiseVersion.ALL],
            [0],
            "a second selectable generation appeared; FORMAT_UNSUPPORTED "
            "may now be reachable from Python and needs a real case here",
        )
        with self.assertRaises(ValueError) as caught:
            _core.WwiseVersion.from_code(1)
        self.assertNotIsInstance(
            caught.exception,
            _core.WemEncoderError,
            "an unknown version code became a kernel encoder error; it now "
            "needs a facade case instead of this note",
        )


if __name__ == "__main__":
    unittest.main()
