"""Regression: the mode-selection tail rule must match the paired build.

The paired build emits frames while the *previous* frame's center still lies
inside the PCM, so the final frame runs one hop past the source length -- its
own hop, not a fixed prefix.  A constant ``center < source_len + prefix`` bound
overshoots by a fixed amount instead and emits trailing frames the build does
not (the 2ch/48k reference stream was seven short frames shorter than ours
before this rule was adopted).
"""

from __future__ import annotations

import unittest
from pathlib import Path

from tests.analysis_resource_support import installed_profile_bundle
from wwise_wem import WwiseProfile, WwiseVersion
from wwise_wem.profiles.registry import resolve_selection
from wwise_wem.adapters.wav import read_pcm_wav
from wwise_wem_reference.analysis.session import AnalysisSession
from wwise_wem_reference.profiles.assembly import assemble_encoder_profile_resources

ROOT = Path(__file__).resolve().parents[3]
SIX_CHANNEL_FIXTURE = ROOT / "tests" / "fixtures" / "input.wav"
SIX_CHANNEL_SELECTION = WwiseProfile(WwiseVersion.WWISE2013, 6, 44100)
TWO_CHANNEL_SELECTION = WwiseProfile(WwiseVersion.WWISE2013, 2, 48000)
#: Synthetic length whose center grid places a frame start past the PCM end:
#: the old bound emitted 11 frames here, the paired-build rule emits 10.
DISCRIMINATING_FRAMES = 7425


def _session(selection: WwiseProfile) -> tuple[int, AnalysisSession]:
    profile = resolve_selection(selection)
    bundle = installed_profile_bundle(
        profile.channels, profile.sample_rate, profile.key.generation
    )
    resources = assemble_encoder_profile_resources(
        bundle, setup_packet=profile.setup_packet(), quality=profile.quality
    )
    session = AnalysisSession(
        profile.channels,
        sample_rate=profile.sample_rate,
        blocksizes=profile.block_sizes,
        resources=resources.analysis,
    )
    return profile.channels, session


def _centres(modes: tuple[int, ...], blocksizes: tuple[int, ...]) -> list[int]:
    """Frame start positions, using the same hop advance as the selector."""
    centre = 0
    centres: list[int] = []
    for index, mode in enumerate(modes):
        following = modes[index + 1] if index + 1 < len(modes) else 0
        centres.append(centre)
        centre += blocksizes[mode] // 4 + blocksizes[following] // 4
    return centres


def _synthetic(frames: int, channels: int) -> list[list[float]]:
    return [
        [((frame * 7 + channel * 11 + 3) % 64536 - 32768) / 32768.0 for frame in range(frames)]
        for channel in range(channels)
    ]


class ModeSelectionTailTests(unittest.TestCase):
    def test_six_channel_fixture_plan_is_stable(self) -> None:
        """The byte-exact 6ch reference's plan must not move."""
        _, session = _session(SIX_CHANNEL_SELECTION)
        pcm = read_pcm_wav(SIX_CHANNEL_FIXTURE)
        modes = session.select_modes(pcm.channels)
        self.assertEqual(len(modes), 205)

    def test_tail_stops_one_frame_past_the_source_length(self) -> None:
        """Every frame but the last starts inside the PCM; the last reaches it."""
        channels, session = _session(TWO_CHANNEL_SELECTION)
        modes = session.select_modes(_synthetic(DISCRIMINATING_FRAMES, channels))
        centres = _centres(modes, session.blocksizes)
        self.assertGreaterEqual(centres[-1], DISCRIMINATING_FRAMES)
        for centre in centres[:-1]:
            self.assertLess(centre, DISCRIMINATING_FRAMES)
        self.assertEqual(len(centres), 10)


if __name__ == "__main__":
    unittest.main()
