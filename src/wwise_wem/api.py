"""Single public encoding entry point."""

from __future__ import annotations

from os import PathLike
from pathlib import Path
from typing import TYPE_CHECKING

from .application.models import EncodeResult
from .model import PcmBuffer, RawPcm

if TYPE_CHECKING:
    from ._core import WwiseProfile


def encode(
    source: str | PathLike[str] | PcmBuffer | RawPcm,
    *,
    profile: WwiseProfile | None = None,
    quality: float | None = None,
) -> EncodeResult:
    """Encode a WAV path, an in-memory PCM buffer, or typed raw PCM.

    WAV format is detected from the RIFF header. Raw bytes require a
    :class:`RawPcm` wrapper because their geometry cannot be inferred.

    ``profile`` is a :class:`WwiseProfile` selection (Wwise generation plus
    PCM geometry); it is handed straight to the kernel, which resolves it
    against the configurations it carries, so an unsatisfiable selection is
    rejected there instead of being guessed. With no ``profile`` the
    installed generation's configuration for the input geometry is selected.
    Implementation imports stay local so importing the package remains cheap.
    """
    from ._core import WwiseProfile, WwiseVersion
    from .adapters.raw import _normalize_pcm16_bytes
    from .adapters.wav import _read_wav_pcm16_bytes
    from .application.encoder import Encoder

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

    selection = (
        WwiseProfile(WwiseVersion.DEFAULT, channels, sample_rate)
        if profile is None
        else profile
    )
    encoder = Encoder(selection, quality=quality)
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
