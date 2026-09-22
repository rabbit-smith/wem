from __future__ import annotations

import subprocess
import sys
import tempfile
import unittest
from pathlib import Path
from unittest.mock import patch

from wwise_wem import WwiseProfile, WwiseVersion
from wwise_wem.application.models import EncodeResult, EncodeStats
from wwise_wem.model import PcmBuffer


FIXTURES = Path(__file__).resolve().parents[1] / "fixtures"
ROOT = Path(__file__).resolve().parents[2]
SELECTION = WwiseProfile(WwiseVersion.WWISE2013, 6, 44100)


def _result() -> EncodeResult:
    return EncodeResult(
        b"WEM!",
        EncodeStats(
            pcm_frames=16,
            channels=6,
            audio_packets=2,
            short_packets=1,
            long_packets=1,
        ),
    )


def _pcm() -> PcmBuffer:
    return PcmBuffer(44100, ((0.0,),) * 6)


class CliTests(unittest.TestCase):
    def test_geometry_assertion_fails_before_encoding(self):
        result = subprocess.run(
            [
                sys.executable,
                "-m",
                "wwise_wem",
                str(FIXTURES / "input.wav"),
                "--channels",
                "2",
                "--output",
                "unused.wem",
            ],
            capture_output=True,
            text=True,
        )
        self.assertNotEqual(result.returncode, 0)
        self.assertIn("differs from WAV", result.stderr)
        self.assertFalse(Path("unused.wem").exists())

    def test_cli_calls_typed_api_and_writes_result(self) -> None:
        result = _result()
        pcm = _pcm()
        with tempfile.TemporaryDirectory() as directory:
            output = Path(directory) / "nested" / "output.wem"
            argv = [
                "wwise-wem",
                "input.wav",
                "--wwise-version",
                "2013.2",
                "--channels",
                "6",
                "--sample-rate",
                "44100",
                "--output",
                str(output),
            ]
            with (
                patch("sys.argv", argv),
                patch("wwise_wem.cli.read_pcm_wav", return_value=pcm),
                patch("wwise_wem.cli.encode", return_value=result) as encode,
                patch("builtins.print") as printed,
            ):
                from wwise_wem.cli import main

                main()

            self.assertEqual(output.read_bytes(), result.data)
        encode.assert_called_once_with(
            pcm,
            profile=SELECTION,
            quality=None,
        )
        printed.assert_called_once()
        label, values = printed.call_args.args
        self.assertEqual(label, "WAV to WEM OK")
        # The byte count in the report is the CLI's own count of the bytes it
        # holds — the library returns no length of bytes the caller has.
        self.assertEqual(values["bytes"], len(result))
        self.assertEqual(values["output"], str(output))
        self.assertEqual(values["audio_packets"], 2)
        self.assertNotIn("sha256", values)

    def test_cli_quality_is_passed_through(self) -> None:
        result = _result()
        pcm = _pcm()
        with tempfile.TemporaryDirectory() as directory:
            output = Path(directory) / "output.wem"
            argv = [
                "wwise-wem",
                "input.wav",
                "--wwise-version",
                "2013",
                "--quality",
                "6",
                "--output",
                str(output),
            ]
            with (
                patch("sys.argv", argv),
                patch("wwise_wem.cli.read_pcm_wav", return_value=pcm),
                patch("wwise_wem.cli.encode", return_value=result) as encode,
                patch("builtins.print"),
            ):
                from wwise_wem.cli import main

                main()
            self.assertEqual(output.read_bytes(), result.data)
        encode.assert_called_once_with(pcm, profile=SELECTION, quality=6.0)

    def test_bad_wwise_version_is_an_argument_error_before_the_wav_is_read(
        self,
    ) -> None:
        with (
            patch(
                "sys.argv",
                [
                    "wwise-wem",
                    "input.wav",
                    "--wwise-version",
                    "2014",
                    "--output",
                    "unused.wem",
                ],
            ),
            patch("wwise_wem.cli.read_pcm_wav") as read,
        ):
            from wwise_wem.cli import main

            with self.assertRaises(SystemExit) as exit_status:
                main()
        self.assertEqual(exit_status.exception.code, 2)
        read.assert_not_called()

    def test_there_is_no_digest_flag_to_assert(self) -> None:
        # A digest is the caller's to compute from the bytes it received, so
        # the CLI carries no flag that would make the library produce one:
        # the removed option is now an ordinary unknown-argument error, and
        # nothing is written.
        with tempfile.TemporaryDirectory() as directory:
            output = Path(directory) / "unused.wem"
            with (
                patch(
                    "sys.argv",
                    [
                        "wwise-wem",
                        "input.wav",
                        "--output",
                        str(output),
                        "--expect-sha256",
                        "0" * 64,
                    ],
                ),
                patch("wwise_wem.cli.read_pcm_wav", return_value=_pcm()),
                patch("wwise_wem.cli.encode") as encode,
            ):
                from wwise_wem.cli import main

                with self.assertRaises(SystemExit) as exit_status:
                    main()
            self.assertEqual(exit_status.exception.code, 2)
            encode.assert_not_called()
            self.assertFalse(output.exists())

    def test_console_script_points_to_canonical_cli(self) -> None:
        pyproject = (ROOT / "pyproject.toml").read_text(encoding="utf-8")
        self.assertIn(
            'wwise-wem = "wwise_wem.cli:main"',
            pyproject,
        )


if __name__ == "__main__":
    unittest.main()
