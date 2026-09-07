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


def encode_pcm_wav(
    wav: Path,
    *,
    profile: str | None = None,
) -> EncodeResult:
    """Encode a PCM WAV (16-bit, 24-bit, or 32-bit IEEE float) to a WEM.

    16-bit input behaves exactly like :func:`encode_wav`.  24-bit and
    float32 input is converted into the signed-16 encoder domain at the
    adapter boundary using deterministic, documented rules (see
    ``wwise_wem.adapters.sample_conversion``); the encoder then sees only
    in-domain samples.  The WEM bytes for converted inputs are
    deterministic, but are not promised to be bit-exact against Wwise's
    own import path.
    """
    from .application.encoder import Encoder
    from .adapters.wav import read_pcm_wav
    from .profiles.registry import (
        load_wem_profile,
        resolve_wem_profile,
    )

    pcm = read_pcm_wav(wav)
    selected = (
        load_wem_profile(profile)
        if profile is not None
        else resolve_wem_profile(pcm.channel_count, pcm.sample_rate)
    )

    return Encoder(selected).encode_pcm(pcm)


def encode_raw_pcm(
    data: bytes,
    *,
    sample_rate: int,
    channels: int,
    bits_per_sample: int,
    profile: str | None = None,
) -> EncodeResult:
    """Encode raw PCM bytes (16/24-bit signed or 32-bit float32) to a WEM.

    The caller supplies the full geometry explicitly; validation follows
    the :class:`PcmBuffer` rejection semantics (see
    ``wwise_wem.adapters.raw``).  24-bit and float32 payloads are
    converted into the signed-16 domain at this boundary; see
    :func:`encode_pcm_wav` for the bit-exactness caveat.
    """
    from .application.encoder import Encoder
    from .adapters.raw import read_raw_pcm
    from .profiles.registry import (
        load_wem_profile,
        resolve_wem_profile,
    )

    pcm = read_raw_pcm(
        data,
        sample_rate=sample_rate,
        channels=channels,
        bits_per_sample=bits_per_sample,
    )
    selected = (
        load_wem_profile(profile)
        if profile is not None
        else resolve_wem_profile(pcm.channel_count, pcm.sample_rate)
    )

    return Encoder(selected).encode_pcm(pcm)
