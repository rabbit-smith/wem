"""Deep PCM-to-WEM encoder orchestration.

The :class:`Encoder` is a native-first facade: byte-producing calls run on
the Rust kernel (``_wwise_wem_native``) when the resolved engine is native
(see ``wwise_wem._engine`` and ``WWISE_WEM_ENGINE``), and otherwise on the
pure-Python implementation below, which stays the reference oracle.
"""

from __future__ import annotations

import importlib.util
from dataclasses import dataclass
from pathlib import Path
from types import MappingProxyType
from typing import Any, Mapping

from .. import _engine
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
        self._native_backend: Any = None

    def encode_pcm(self, pcm: PcmBuffer) -> EncodeResult:
        """Encode one independent PCM buffer into a complete Wwise WEM.

        Engine dispatch (see ``wwise_wem._engine``): template containers
        always use the pure-Python implementation (the native one-shot API
        is profile-container only); profile containers use the native
        kernel when the resolved engine is native and the PCM values sit in
        the signed-16 sample domain, raising ``ValueError`` otherwise under
        an explicit ``native`` pin while ``auto`` routes such buffers to the
        pure-Python implementation.
        """
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

        if self._container.metadata_source.startswith("profile:"):
            engine = _engine.active_engine()
            if engine == "native":
                try:
                    rows = self._int16_rows(pcm)
                except ValueError:
                    if _engine.requested_engine() != "auto":
                        # An explicit native pin never downgrades silently.
                        raise
                    return self._encode_pcm_python(pcm)
                return self._encode_pcm_native(pcm, rows)
        return self._encode_pcm_python(pcm)

    def _int16_rows(self, pcm: PcmBuffer) -> list[list[int]]:
        """Convert in-domain float PCM rows into kernel signed-16 rows.

        The kernel consumes integer signed-16 samples; the public float
        domain is ``value / 32768.0`` as produced by ``read_pcm16_wav`` /
        ``read_pcm16``. Every sample must be exactly that of an integer in
        the signed-16 range; anything else is rejected the same way the
        pure-Python path rejects input errors (``ValueError``).
        """
        rows: list[list[int]] = []
        for channel, row in enumerate(pcm.channels):
            values: list[int] = []
            for frame, sample in enumerate(row):
                scaled = sample * 32768.0
                if not scaled.is_integer() or not -32768.0 <= scaled <= 32767.0:
                    raise ValueError(
                        f"PCM channel {channel} frame {frame} value {sample!r} "
                        "is not an integer signed-16 sample "
                        "(expected the value/32768.0 domain of read_pcm16_wav)"
                    )
                values.append(int(scaled))
            rows.append(values)
        return rows

    def _profile_data_dir(self) -> str | None:
        """Locate the installed profile tree for the native kernel.

        The kernel resolves ``WEM_DATA_DIR`` or its build-time repository
        layout; passing the explicit directory keeps facade behavior
        identical from a source tree, an installed site-packages, or any
        current directory.
        """
        spec = importlib.util.find_spec("wwise_wem")
        origin = getattr(spec, "origin", None) if spec is not None else None
        if not isinstance(origin, str) or not origin:
            return None
        candidate = Path(origin).resolve().parent / "data" / "profiles"
        return str(candidate) if candidate.is_dir() else None

    def _encode_pcm_native(self, pcm: PcmBuffer, rows: list[list[int]]) -> EncodeResult:
        """Run one encode on the native kernel and fill the Python DTOs."""
        module = _engine.native_module()
        if module is None:
            raise RuntimeError("native engine resolved without an importable module")
        if self._native_backend is None:
            self._native_backend = module.Encoder(
                self.profile.name,
                self._profile_data_dir(),
            )
        try:
            result = self._native_backend.encode_pcm(pcm.sample_rate, rows)
        except module.WemEncoderError as error:
            # The public contract surfaces input/configuration errors as
            # ValueError; kernel rejections reach the user only after all
            # Python-side validation passed, so the mapping preserves the
            # contract's error surface (message text is not contractual).
            raise ValueError(str(error)) from error
        stats = EncodeStats(
            pcm_frames=int(result.pcm_frames),
            channels=int(result.channels),
            audio_packets=int(result.audio_packets),
            short_packets=int(result.short_packets),
            long_packets=int(result.long_packets),
            bytes=int(result.bytes_out),
            metadata_source=self._container.metadata_source,
        )
        return EncodeResult(bytes(result.data), stats)

    def _encode_pcm_python(self, pcm: PcmBuffer) -> EncodeResult:
        """Pure-Python reference encode path (oracle behavior)."""
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
