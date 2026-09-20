"""Reference (oracle) encode pipeline, a test-time asset.

This module owns the reference-tree imports of the analysis session, Vorbis
packet encoder, and container builder; tests and capture tooling import it
directly (e.g. ``wwise_wem_reference.python_engine.encode_pcm_python``) and
patch its module-level names to observe the real pipeline without modifying
it on disk.  It is not a runtime engine: the distributed facade runs on the
native kernel (``wwise_wem._core``) only, and nothing in ``src/`` imports
this package.  Byte-identical parity between this oracle and the kernel is
pinned by the test suites, not by a runtime switch.
"""

from __future__ import annotations

from wwise_wem.application.models import EncodeResult, EncodeStats
from wwise_wem.model import PcmBuffer
from wwise_wem.profiles.bundle import load_profile_bundle
from wwise_wem.profiles.model import EncoderProfile

from .analysis.session import AnalysisSession
from .container.model import ContainerPlan
from .container.wem import build_vorbis_wem
from .profiles.assembly import assemble_encoder_profile_resources
from .vorbis.packet_encoder import pack_analysis_frame


def encode_pcm_python(
    *,
    profile: EncoderProfile,
    container: ContainerPlan,
    pcm: PcmBuffer,
) -> EncodeResult:
    """Run one complete pure-Python reference encode.

    ``container`` is the per-output container plan (:class:`ContainerPlan`);
    it carries container metadata only, never codec state.
    """
    bundle = load_profile_bundle(profile=profile.name, verify_all=False)
    setup_packet = profile.setup_packet()
    resources = assemble_encoder_profile_resources(
        bundle,
        setup_packet=setup_packet,
        quality=profile.quality,
    )
    session = AnalysisSession(
        profile.channels,
        sample_rate=profile.sample_rate,
        blocksizes=profile.block_sizes,
        resources=resources.analysis,
    )
    conditioned_pcm = session.condition_pcm(pcm.channels)
    modes, windows = session.selected_windows(conditioned_pcm)
    audio_packets: list[bytes] = []
    for window in windows:
        analysis = session.analyze_window(window)
        audio_packets.append(
            pack_analysis_frame(
                resources.setup,
                resources.codebooks,
                analysis,
                channels=profile.channels,
            ).packet
        )
    if len(audio_packets) != len(modes):
        raise AssertionError("analysis window and mode counts diverged")

    fmt = dict(container.fmt)
    fmt["dwTotalPCMFrames"] = pcm.frame_count
    encoded = build_vorbis_wem(
        fmt,
        [setup_packet, *audio_packets],
        seek_table=container.seek_table,
        endian=container.endian,
        extra_chunks=list(container.extra_chunks),
        recompute_sizes=True,
    )
    stats = EncodeStats(
        pcm_frames=pcm.frame_count,
        channels=pcm.channel_count,
        audio_packets=len(audio_packets),
        short_packets=modes.count(0),
        long_packets=modes.count(1),
        bytes=len(encoded),
        metadata_source=container.metadata_source,
    )
    return EncodeResult(encoded, stats)


__all__ = ["ContainerPlan", "encode_pcm_python"]
