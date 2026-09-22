"""The kernel's stable error codes survive the facade boundary.

The native extension raises ``WemEncoderError`` carrying a stable ``.code``
(``PROFILE_NOT_FOUND``, ``GEOMETRY_MISMATCH``, ``INPUT_TOO_SHORT``,
``FORMAT_UNSUPPORTED``, ``STATE_ERROR``, ``INTERNAL``) — the same classes
every cross-language shell maps.  The facade owns the last hop: a caller must
be able to branch on that code without digging into ``__cause__``, and the
kernel's diagnostic text must arrive unchanged.

Every rejection below is produced by calling the kernel with input it refuses;
no error object is constructed by this suite.  The public entry points
pre-validate geometry, frame count and byte alignment with their own
``ValueError``s, so the cases those checks cover reach the kernel through
``Encoder._encode_pcm_core`` — the facade's raise site — with the Python-side
check bypassed, which is what makes the kernel's own code observable there.
"""
from __future__ import annotations

import unittest

import wwise_wem._core as _core
from wwise_wem import PcmBuffer, RawPcm, WwiseProfile, WwiseVersion, encode
from wwise_wem.application.encoder import Encoder
from wwise_wem.model import WwiseWemError


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
