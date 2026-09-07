from __future__ import annotations

from tests.analysis_resource_support import installed_analysis_resources

import unittest
from unittest.mock import patch

from wwise_wem_reference.scheduling.model import FramePlan
from wwise_wem_reference.analysis.preprocessing.windowing import WindowedFrame
from wwise_wem_reference.analysis.session import (
    AnalysisSession,
)


def _stream(channels: int = 1, *, sample_rate: int = 44100) -> AnalysisSession:
    return AnalysisSession(
        channels,
        sample_rate=sample_rate,
        blocksizes=(256, 2048),
        resources=installed_analysis_resources(),
    )


def _window(
    index: int,
    previous: int,
    current: int,
    following: int,
    samples: int,
) -> WindowedFrame:
    plan = FramePlan(
        index=index,
        previous=previous,
        current=current,
        following=following,
        sample_start=0,
        sample_end=samples,
        required_filled=samples,
        advance=1,
    )
    return WindowedFrame(
        plan=plan,
        center=512,
        samples=(tuple([0.0] * samples),),
    )


class AnalysisSessionRuntimeTests(unittest.TestCase):
    def test_constructor_and_transient_ingestion_validation(self):
        with self.assertRaises(TypeError):
            AnalysisSession(1)
        with self.assertRaisesRegex(ValueError, "at least one channel"):
            _stream(0)
        with self.assertRaisesRegex(ValueError, "sample rate must be positive"):
            _stream(1, sample_rate=0)
        with self.assertRaisesRegex(ValueError, "expects 256/2048 blocks"):
            AnalysisSession(
                1,
                sample_rate=44100,
                blocksizes=(128, 1024),
                resources=installed_analysis_resources(),
            )

        stream = _stream()
        self.assertIsInstance(
            stream.ingest_transient_quantum([[0.0] * stream.short_bins]), int
        )
        self.assertEqual(stream._mode_selector.generated, stream._mode_selector.hop)
        self.assertEqual(stream.transient_quanta, 1)
        with self.assertRaisesRegex(ValueError, "channel count"):
            stream.ingest_transient_quantum([])
        with self.assertRaisesRegex(ValueError, "128 samples"):
            stream.ingest_transient_quantum([[0.0] * 127])

    def test_analysis_dispatches_by_scheduled_mode(self):
        short_stream = _stream()
        long_stream = _stream()
        short_window = _window(0, 0, 0, 1, short_stream.blocksizes[0])
        long_window = _window(0, 0, 1, 1, long_stream.blocksizes[1])
        short_result = object()
        long_result = object()
        with patch.object(short_stream, "_analyze_short", return_value=short_result) as short_call:
            self.assertIs(short_stream.analyze_window(short_window, short_variant=1), short_result)
            short_call.assert_called_once_with(short_window, short_variant=1)
        with patch.object(long_stream, "_analyze_long", return_value=long_result) as long_call:
            self.assertIs(long_stream.analyze_window(long_window, short_variant=1), long_result)
            long_call.assert_called_once_with(long_window)

    def test_analysis_enforces_contiguous_non_repeated_frames_and_modes(self):
        first = _window(0, 0, 0, 1, 256)
        second = _window(1, 0, 1, 1, 2048)
        stream = _stream()
        with (
            patch.object(stream, "_analyze_short", return_value=object()),
            patch.object(stream, "_analyze_long", return_value=object()),
        ):
            stream.analyze_window(first)
            stream.analyze_window(second)
            with self.assertRaisesRegex(RuntimeError, "expected index 2, got 1"):
                stream.analyze_window(second)

        skipped = _stream()
        with self.assertRaisesRegex(RuntimeError, "expected index 0, got 1"):
            skipped.analyze_window(second)

        inconsistent = _stream()
        wrong_second = _window(1, 0, 0, 1, 256)
        with patch.object(inconsistent, "_analyze_short", return_value=object()):
            inconsistent.analyze_window(first)
            with self.assertRaisesRegex(RuntimeError, "adjacent analysis frame modes differ"):
                inconsistent.analyze_window(wrong_second)

    def test_reset_starts_a_new_frame_lifecycle(self):
        stream = _stream()
        first = _window(0, 0, 0, 1, 256)
        with patch.object(stream, "_analyze_short", return_value=object()):
            stream.analyze_window(first)
            stream.reset()
            stream.analyze_window(first)

if __name__ == "__main__":
    unittest.main()
