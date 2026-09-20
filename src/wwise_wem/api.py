"""Single public encoding entry point."""

from __future__ import annotations

from os import PathLike
from pathlib import Path

from .application.models import EncodeResult
from .model import PcmBuffer, RawPcm


def encode(
    source: str | PathLike[str] | PcmBuffer | RawPcm,
    *,
    profile: str | None = None,
    quality: float | None = None,
) -> EncodeResult:
    """Encode a WAV path, an in-memory PCM buffer, or typed raw PCM.

    WAV format is detected from the RIFF header. Raw bytes require a
    :class:`RawPcm` wrapper because their geometry cannot be inferred.
    Implementation imports stay local so importing the package remains cheap.
    """
    from .application.encoder import Encoder
    from .adapters.raw import _normalize_pcm16_bytes
    from .adapters.wav import _read_wav_pcm16_bytes
    from .profiles.registry import (
        load_wem_profile,
        resolve_wem_profile,
    )

    pcm: PcmBuffer | None = None
    if isinstance(source, PcmBuffer):
        pcm = source
        sample_rate = pcm.sample_rate
        channels = pcm.channel_count
        payload = None
    elif isinstance(source, RawPcm):
        sample_rate = source.sample_rate
        channels = source.channels
        payload = _normalize_pcm16_bytes(
            source.data,
            sample_rate=sample_rate,
            channels=channels,
            bits_per_sample={"s16le": 16, "s24le": 24, "f32le": 32}[
                source.sample_format
            ],
        )
    elif isinstance(source, (str, PathLike)):
        sample_rate, channels, payload = _read_wav_pcm16_bytes(Path(source))
    else:
        raise TypeError("source must be a path, PcmBuffer, or RawPcm")
    selected = (
        load_wem_profile(profile, quality=quality)
        if profile is not None
        else resolve_wem_profile(channels, sample_rate, quality=quality)
    )
    encoder = Encoder(selected)
    if pcm is not None:
        return encoder.encode_pcm(pcm)
    if payload is None:
        raise RuntimeError("input normalization produced no PCM payload")
    return encoder.encode_pcm16_interleaved(
        payload,
        sample_rate=sample_rate,
        channels=channels,
    )


__all__ = ["encode"]
