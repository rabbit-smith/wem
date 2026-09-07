"""Small typed façade over the deep PCM encoder implementation."""

from __future__ import annotations

from pathlib import Path

from .application.models import EncodeResult


def encode_wav(
    wav: Path,
    template: Path | None = None,
    *,
    profile: str | None = None,
) -> EncodeResult:
    """Encode signed-16 PCM WAV input and return an immutable typed result.

    Heavy implementation imports are intentionally local so importing the
    package does not initialize transform, floor, residue, or analysis state.
    """
    import hashlib

    from ._reference import reference_module
    from .application.encoder import Encoder, _ContainerPlan
    from .adapters.wav import read_pcm16
    from .profiles.registry import (
        PROFILE_REGISTRY,
        load_wem_profile,
        resolve_wem_profile,
    )

    pcm = read_pcm16(wav)
    if template is not None and profile is not None:
        raise ValueError("select either an encoder template or profile")

    container = None
    if template is not None:
        load_wem_parts_bytes = reference_module("container.wem").load_wem_parts_bytes
        template_path = Path(template)
        parts = load_wem_parts_bytes(
            template_path.read_bytes(),
            file=template_path.name,
            path=str(template_path),
        )
        if parts.get("kind") != "wwise_vorbis" or not parts.get("packets"):
            raise ValueError(
                "template must be a Wwise Vorbis WEM with a setup packet"
            )
        fmt = dict(parts["fmt"])
        setup_packet = parts["packets"][0]
        template_channels = int(fmt["nChannels"])
        template_sample_rate = int(fmt["nSamplesPerSec"])
        if (pcm.channel_count, pcm.sample_rate) != (
            template_channels,
            template_sample_rate,
        ):
            raise ValueError(
                "WAV channel count/sample rate differs from encoder metadata"
            )
        setup_sha256 = hashlib.sha256(setup_packet).hexdigest()
        selected = PROFILE_REGISTRY.resolve_setup(
            template_channels,
            template_sample_rate,
            setup_sha256,
        )
        installed_setup = selected.setup_packet()
        if setup_packet != installed_setup:
            raise ValueError(
                f"template setup SHA-256 {setup_sha256} differs from "
                f"installed profile {selected.name} setup SHA-256 "
                f"{selected.setup_sha256} for "
                f"{template_channels}ch/{template_sample_rate}Hz"
            )
        container = _ContainerPlan(
            fmt=fmt,
            endian=str(parts.get("endian", "le")),
            seek_table=parts.get("seek_table", b""),
            extra_chunks=tuple(parts.get("extras_before_data", ())),
            metadata_source=f"template:{template}",
        )
    else:
        selected = (
            load_wem_profile(str(profile))
            if profile is not None
            else resolve_wem_profile(pcm.channel_count, pcm.sample_rate)
        )

    return Encoder(selected, _container=container).encode_pcm(pcm)
