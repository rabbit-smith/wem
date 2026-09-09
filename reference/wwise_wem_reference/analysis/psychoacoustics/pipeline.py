#!/usr/bin/env python3
"""Long and short frame psychoacoustic orchestration."""
from __future__ import annotations

from dataclasses import dataclass
from typing import Sequence

from ..config import AnalysisProfileResources, NEGATIVE_INFINITY_DB
from ..dsp.spectrum import wwise_log_curve, wwise_mdct_log_curve
from .envelope import (
    FloorEnvelopeScratch,
    make_channel_floor_envelope_scratch,
    shape_first_long_floor_envelope,
)
from .remap import build_long_psy_remap_variant, build_psy_remap
from .seed import (
    SpectrumPeakState,
    build_long_floor_seed,
    compute_spectrum_peak,
    update_frame_spectrum_peak,
    wwise_seed_floor_from_look,
)
from .short import PsyFrameControls, ShortPsyFrameResult, ShortPsyAnalyzer
from ..dsp.transform import _f32, mdct_forward

@dataclass
class FirstPsyFrame:
    """Diagnostic first-frame surface before the floor1 fit."""

    raw_mdct: list[list[float]]
    fft: list[list[float]]
    remap: list[list[float]]
    seed: list[list[float]]
    post: list[list[float]]
    side: list[list[float]]
    scratch: list[FloorEnvelopeScratch]


@dataclass
class LongPsyFrame:
    """Pure local surface for one fresh long-mode analysis group.

    ``windowed_frames`` is intentionally outside this result: the caller
    owns overlap/window scheduling.  Every field here is generated from the
    supplied 2048-sample frames plus immutable long psycho tables and the
    carried packet-global FFT maximum.
    """

    coefficients: list[list[float]]
    raw_mdct: list[list[float]]
    fft: list[list[float]]
    channel_specmax: tuple[float, ...]
    global_specmax: float
    remap: list[list[float]]
    seed: list[list[float]]
    post: list[list[float]]
    side: list[list[float]]
    scratch: list[FloorEnvelopeScratch]
    state_info: PsyFrameControls | None = None


@dataclass
class ShortPsyStreamFrame:
    """One local short analysis group connected to the shared floor-envelope stage owner.

    This is the streaming counterpart to :class:`LongPsyFrame`.
    The caller supplies the stateful short owner and scheduler controls; no
    external records or first-frame-only scratch are consulted here.
    """

    coefficients: list[list[float]]
    raw_mdct: list[list[float]]
    fft: list[list[float]]
    channel_specmax: tuple[float, ...]
    global_specmax: float
    remap: list[list[float]]
    seed: list[list[float]]
    post: list[list[float]]
    side: list[list[float]]
    state_result: ShortPsyFrameResult

def analyze_long_frame(
    windowed_frames: Sequence[Sequence[float]],
    *,
    carried_global_specmax: float = NEGATIVE_INFINITY_DB,
    resources: AnalysisProfileResources,
    scratch: Sequence[FloorEnvelopeScratch] | None = None,
    stream: ShortPsyAnalyzer | None = None,
    long_variant: int = 0,
    following_mode: int = 1,
    specmax_state: SpectrumPeakState | None = None,
) -> LongPsyFrame:
    """Execute the local fresh long-block floor-analysis chain.

    This encoder-facing long path depends only on PCM and packaged profile
    data:

    ``windowed PCM → spectrum transform MDCT/log + packed FFT/log → psychoacoustic remap → floor-seed stage``
    ``→ inactive floor-envelope stage``.

    ``carried_global_specmax`` is the caller's stream value *after* any
    scheduler decay and before this channel group.  For scheduler-owned code,
    pass ``specmax_state`` instead: it applies the per-frame decay and folds
    all channel maxima itself.  The function then runs floor-seed stage in group order.
    ``scratch`` optionally supplies the six shared
    1024-bin state/history pairs retained across block-size transitions; the
    inactive long branch writes only ``state=raw`` and leaves history intact.
    Pass ``stream`` instead to use the same six allocations as
    :func:`analyze_short_frame`; its long-state bridge then performs
    the actual long→short 8:1 reduction when ``following_mode`` is zero.
    """
    if not windowed_frames:
        raise ValueError("long analysis needs at least one channel frame")
    if int(long_variant) not in (0, 1):
        raise ValueError("long analysis variant must be 0 or 1")
    table = resources.long_base
    if table.n != 1024:
        raise ValueError("long analysis table must have 1024 output bins")
    if any(len(frame) != table.n * 2 for frame in windowed_frames):
        raise ValueError("long analysis expects 2048-sample windowed frames")
    if scratch is not None and len(scratch) != len(windowed_frames):
        raise ValueError("long analysis scratch count must equal channel count")
    if stream is not None:
        if scratch is not None:
            raise ValueError("long analysis accepts either scratch or shared stream, not both")
        if len(stream.channels) != len(windowed_frames):
            raise ValueError("long analysis stream channel count differs from frames")

    # The two scheduler variants are distinct psycho looks even though both
    # use a 1024-bin block: transition blocks select psychoacoustic remap mode 2, while an
    # ordinary long->long block selects mode 3.  floor-seed stage is shared.  Keep the
    # variant table separate so callers that inject the base seed table do
    # not accidentally force every long packet through mode 2.

    analysis_mode = 2 + int(long_variant)
    analysis_table = resources.long_variants[analysis_mode]
    mdct_look = resources.mdct_looks[table.n * 2]
    if mdct_look.n != table.n * 2:
        raise ValueError("long MDCT look differs from analysis geometry")
    coefficients = [
        mdct_forward(mdct_look, [_f32(value) for value in frame])
        for frame in windowed_frames
    ]
    raw_mdct = [wwise_mdct_log_curve(values) for values in coefficients]
    fft = [
        wwise_log_curve([_f32(value) for value in frame], twiddles=resources._frozen_twiddles())
        for frame in windowed_frames
    ]
    if specmax_state is None:
        channel_specmax, global_specmax = compute_spectrum_peak(
            fft, initial_global=carried_global_specmax
        )
    else:
        channel_specmax, global_specmax = update_frame_spectrum_peak(
            fft,
            block_bins=table.n,
            sample_rate=table.sample_rate,
            state=specmax_state,
        )

    shared_scratch = (
        [FloorEnvelopeScratch(channel.state, channel.history) for channel in stream.channels]
        if stream is not None
        else scratch
    )
    remap: list[list[float]] = []
    seed: list[list[float]] = []
    post: list[list[float]] = []
    side: list[list[float]] = []
    scratches: list[FloorEnvelopeScratch] = []
    long_floor_envelope = resources.long_floor_looks[analysis_mode]
    for channel, (raw, logfft, specmax, coeff) in enumerate(zip(
        raw_mdct, fft, channel_specmax, coefficients
    )):
        local_remap = build_long_psy_remap_variant(
            raw, analysis_mode, analysis_table
        ).remap
        local_seed = build_long_floor_seed(
            logfft,
            channel_specmax=specmax,
            global_specmax=global_specmax,
            table=table,
        )
        local_scratch = (
            shared_scratch[channel]
            if shared_scratch is not None
            else make_channel_floor_envelope_scratch()
        )
        local_post, local_side = shape_first_long_floor_envelope(
            local_seed,
            local_remap,
            raw,
            coeff,
            local_scratch,
            look=long_floor_envelope,
        )
        remap.append(local_remap)
        seed.append(local_seed)
        post.append(local_post)
        side.append(local_side)
        scratches.append(local_scratch)
    state_info = (
        stream.commit_long_state(
            raw_mdct,
            long_variant=long_variant,
            following_mode=following_mode,
        )
        if stream is not None
        else None
    )
    return LongPsyFrame(
        coefficients=coefficients,
        raw_mdct=raw_mdct,
        fft=fft,
        channel_specmax=channel_specmax,
        global_specmax=global_specmax,
        remap=remap,
        seed=seed,
        post=post,
        side=side,
        scratch=scratches,
        state_info=state_info,
    )


def analyze_short_frame(
    windowed_frames: Sequence[Sequence[float]],
    stream: ShortPsyAnalyzer,
    resources: AnalysisProfileResources,
    *,
    short_variant: int,
    following_mode: int,
    q: float = -1.0,
    update_gate: int = 0,
    carried_global_specmax: float = NEGATIVE_INFINITY_DB,
    specmax_state: SpectrumPeakState | None = None,
    groups: Sequence[Sequence[float]] | None = None,
) -> ShortPsyStreamFrame:
    """Run one local short frame through the persistent active floor-envelope stage path.

    Inputs are the six scheduler-windowed 256-sample PCM views.  The caller
    supplies the selector's short variant and following audio mode; the
    :class:`~wwise_wem.analysis.psychoacoustics.short.ShortPsyAnalyzer` derives and advances the
    previous-transition/run/tail recurrence once for the whole channel group.
    The transform and psychoacoustic stages use the same typed frame data as the long path.
    """
    if not windowed_frames:
        raise ValueError("short analysis needs at least one channel frame")
    if len(windowed_frames) != len(stream.channels):
        raise ValueError("short analysis channel count differs from state owner")
    if any(len(frame) != 256 for frame in windowed_frames):
        raise ValueError("short analysis expects 256-sample windowed frames")
    if groups is not None and len(groups) != len(windowed_frames):
        raise ValueError("short analysis group-work count differs from channels")

    mdct_look = resources.mdct_looks[256]
    if mdct_look.n != 256:
        raise ValueError("short MDCT look differs from analysis geometry")
    coefficients = [
        mdct_forward(mdct_look, [_f32(value) for value in frame])
        for frame in windowed_frames
    ]
    raw_mdct = [wwise_mdct_log_curve(values) for values in coefficients]
    fft = [
        wwise_log_curve([_f32(value) for value in frame], twiddles=resources._frozen_twiddles())
        for frame in windowed_frames
    ]
    if specmax_state is None:
        channel_specmax, global_specmax = compute_spectrum_peak(
            fft, initial_global=carried_global_specmax
        )
    else:
        channel_specmax, global_specmax = update_frame_spectrum_peak(
            fft,
            block_bins=128,
            sample_rate=resources.short_look.sample_rate,
            state=specmax_state,
        )
    look = resources.short_look
    remap = [
        build_psy_remap(
            raw, q, look,
            curve_offsets=resources.short_surface.remap_curve_offsets,
        )[3]
        for raw in raw_mdct
    ]
    seed = [
        wwise_seed_floor_from_look(
            look,
            logfft,
            channel_specmax=channel_max,
            global_specmax=global_specmax,
        )
        for logfft, channel_max in zip(fft, channel_specmax)
    ]
    state_result = stream.process_frame(
        remap,
        seed,
        coefficients,
        raw_mdct,
        short_variant=short_variant,
        following_mode=following_mode,
        q=q,
        update_gate=update_gate,
        groups=groups,
    )
    return ShortPsyStreamFrame(
        coefficients=coefficients,
        raw_mdct=raw_mdct,
        fft=fft,
        channel_specmax=channel_specmax,
        global_specmax=global_specmax,
        remap=remap,
        seed=seed,
        post=[list(row.post) for row in state_result.channels],
        side=[list(row.side) for row in state_result.channels],
        state_result=state_result,
    )
