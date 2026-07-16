"""Deep PCM-to-WEM encoder orchestration."""

from __future__ import annotations

from dataclasses import dataclass
from types import MappingProxyType
from typing import Any, Mapping

from ..vorbis.packet_encoder import pack_analysis_frame
from ..profiles.assembly import assemble_encoder_profile_resources
from ..profiles.bundle import load_profile_bundle
from ..container.wem import build_vorbis_wem
from .models import EncodeResult, EncodeStats
from ..model import PcmBuffer
from ..analysis.session import AnalysisSession
from ..profiles.model import EncoderProfile


@dataclass(frozen=True)
class _ContainerPlan:
    """Per-output container metadata, kept separate from codec identity."""

    fmt: Mapping[str, Any]
    endian: str
    seek_table: bytes
    extra_chunks: tuple[tuple[bytes, bytes], ...]
    metadata_source: str

    def __post_init__(self) -> None:
        if not isinstance(self.fmt, Mapping):
            raise TypeError("container fmt must be a mapping")
        if self.endian not in ("le", "be"):
            raise ValueError("container endian must be 'le' or 'be'")
        if not isinstance(self.metadata_source, str) or not self.metadata_source:
            raise ValueError("metadata_source must be a non-empty string")
        frozen_fmt = MappingProxyType(dict(self.fmt))
        extras: list[tuple[bytes, bytes]] = []
        for chunk_id, payload in self.extra_chunks:
            chunk_id = bytes(chunk_id)
            if len(chunk_id) != 4:
                raise ValueError("container extra chunk id must be exactly 4 bytes")
            extras.append((chunk_id, bytes(payload)))
        object.__setattr__(self, "fmt", frozen_fmt)
        object.__setattr__(self, "seek_table", bytes(self.seek_table))
        object.__setattr__(self, "extra_chunks", tuple(extras))

    @classmethod
    def from_profile(cls, profile: EncoderProfile) -> "_ContainerPlan":
        return cls(
            fmt=profile.container_metadata.to_fmt_dict(
                frame_count=profile.container_metadata.dwTotalPCMFrames
            ),
            endian=profile.endian,
            seek_table=profile.seek_table,
            extra_chunks=profile.extra_chunks,
            metadata_source=f"profile:{profile.name}",
        )


class Encoder:
    """Own immutable codec inputs and run one complete encode per PCM buffer."""

    def __init__(
        self,
        profile: EncoderProfile,
        *,
        _container: _ContainerPlan | None = None,
    ) -> None:
        if not isinstance(profile, EncoderProfile):
            raise TypeError("profile must be EncoderProfile")
        if tuple(profile.block_sizes) != (256, 2048):
            raise ValueError(
                "selected profile block geometry is unsupported; "
                "the installed runtime supports 256/2048 blocks"
            )
        plan = _ContainerPlan.from_profile(profile) if _container is None else _container
        if not isinstance(plan, _ContainerPlan):
            raise TypeError("_container must be _ContainerPlan")
        if (
            int(plan.fmt["nChannels"]),
            int(plan.fmt["nSamplesPerSec"]),
        ) != (profile.channels, profile.sample_rate):
            raise ValueError("container metadata geometry differs from encoder profile")

        bundle = load_profile_bundle(profile=profile.name, verify_all=False)
        if bundle.key != profile.key:
            raise ValueError("selected profile differs from installed profile bundle")
        if profile.setup_sha256 != bundle.setup.sha256:
            raise ValueError(
                f"selected profile {profile.name} differs from installed profile setup"
            )
        setup_packet = profile.setup_packet()
        resources = assemble_encoder_profile_resources(
            bundle,
            setup_packet=setup_packet,
        )

        self.profile = profile
        self._container = plan
        self._resources = resources
        self._setup_packet = resources.setup_packet
        self._analysis_resources = resources.analysis
        self._setup = resources.setup
        self._books = resources.codebooks

    def encode_pcm(self, pcm: PcmBuffer) -> EncodeResult:
        """Encode one independent PCM buffer into a complete Wwise WEM."""
        if not isinstance(pcm, PcmBuffer):
            raise TypeError("pcm must be PcmBuffer")
        if (pcm.channel_count, pcm.sample_rate) != (
            self.profile.channels,
            self.profile.sample_rate,
        ):
            raise ValueError(
                "PCM channel count/sample rate differs from encoder profile"
            )
        if pcm.frame_count < 4096:
            raise ValueError("PCM input must contain at least 4096 frames")

        session = AnalysisSession(
            self.profile.channels,
            sample_rate=self.profile.sample_rate,
            blocksizes=self.profile.block_sizes,
            resources=self._analysis_resources,
        )
        modes, windows = session.selected_windows(pcm.channels)
        audio_packets: list[bytes] = []
        for window in windows:
            analysis = session.analyze_window(window)
            audio_packets.append(
                pack_analysis_frame(
                    self._setup,
                    self._books,
                    analysis,
                    channels=self.profile.channels,
                ).packet
            )
        if len(audio_packets) != len(modes):
            raise AssertionError("analysis window and mode counts diverged")

        fmt = dict(self._container.fmt)
        fmt["dwTotalPCMFrames"] = pcm.frame_count
        encoded = build_vorbis_wem(
            fmt,
            [self._setup_packet, *audio_packets],
            seek_table=self._container.seek_table,
            endian=self._container.endian,
            extra_chunks=list(self._container.extra_chunks),
            recompute_sizes=True,
        )
        stats = EncodeStats(
            pcm_frames=pcm.frame_count,
            channels=pcm.channel_count,
            audio_packets=len(audio_packets),
            short_packets=modes.count(0),
            long_packets=modes.count(1),
            bytes=len(encoded),
            metadata_source=self._container.metadata_source,
        )
        return EncodeResult(encoded, stats)


__all__ = ["Encoder"]
