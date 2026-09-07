"""Small typed façade over the deep PCM encoder implementation."""

from __future__ import annotations

from pathlib import Path

from .application.models import EncodeResult


def encode_wav(
    wav: Path,
    *,
    profile: str | None = None,
) -> EncodeResult:
    """Encode signed-16 PCM WAV input and return an immutable typed result.

    Heavy implementation imports are intentionally local so importing the
    package does not initialize transform, floor, residue, or analysis state.
    """
    from .application.encoder import Encoder
    from .adapters.wav import read_pcm16
    from .profiles.registry import (
        load_wem_profile,
        resolve_wem_profile,
    )

    pcm = read_pcm16(wav)
    selected = (
        load_wem_profile(profile)
        if profile is not None
        else resolve_wem_profile(pcm.channel_count, pcm.sample_rate)
    )

    return Encoder(selected).encode_pcm(pcm)
