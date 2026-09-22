"""Build the LPC-padded PCM timeline consumed by the transient detector."""

from __future__ import annotations

from typing import Iterator, Sequence

from ...scheduling.model import DEFAULT_BLOCKSIZES


def detector_pcm_streams(
    pcm: Sequence[Sequence[float]],
    *,
    prefix_samples: int | None = None,
    terminal_samples: int = 8192,
    tail_training: int | None = None,
    blocksizes: Sequence[int] = DEFAULT_BLOCKSIZES,
) -> tuple[tuple[float, ...], ...]:
    """Build the absolute PCM streams consumed by the transient detector.

    The detector uses a 128-sample window with a 64-sample hop.  Unlike the first short
    MDCT view, its initial LPC history spans ``blocksize1/2`` (1024 samples
    for this setup).  EOS uses the same forward 32-tap predictor as the main
    feeder.  The result is independent of selector decisions and is therefore
    suitable for the runtime selector as well as offline regression.
    """
    if not pcm:
        raise ValueError("transient detector needs at least one PCM channel")
    source_len = len(pcm[0])
    if source_len < 4096 or any(len(channel) != source_len for channel in pcm):
        raise ValueError("transient detector PCM must be equal-length and at least 4096 samples")
    if terminal_samples < 0:
        raise ValueError("transient-detector terminal prediction count must be non-negative")
    if prefix_samples is None:
        prefix_samples = int(blocksizes[1]) // 2
    if prefix_samples < 1:
        raise ValueError("transient-detector prefix length must be positive")

    from ..dsp.lpc import (
        wwise_first_frame_lpc_prime,
        wwise_lpc_from_data,
        wwise_lpc_predict,
    )
    from ..._f32 import _f32

    if tail_training is None:
        tail_training = max(int(size) for size in blocksizes)
    if tail_training <= 32 or tail_training > source_len:
        raise ValueError("transient-detector tail training length is invalid")
    result = []
    for source in pcm:
        channel = [_f32(value) for value in source]
        prefix = wwise_first_frame_lpc_prime(channel, prefill=prefix_samples)
        tail = wwise_lpc_predict(
            wwise_lpc_from_data(channel[-tail_training:], order=32),
            channel[-32:],
            terminal_samples,
        )
        result.append(tuple(prefix + channel + tail))
    return tuple(result)


def iter_detector_quanta(
    pcm: Sequence[Sequence[float]],
    *,
    hop: int = 64,
    window: int = 128,
    count: int | None = None,
    terminal_samples: int = 8192,
    tail_training: int | None = None,
    blocksizes: Sequence[int] = DEFAULT_BLOCKSIZES,
) -> Iterator[tuple[tuple[float, ...], ...]]:
    """Yield channel-major transient-detector PCM windows on the absolute detector timeline."""
    if hop <= 0 or window <= 0:
        raise ValueError("transient-detector hop/window must be positive")
    streams = detector_pcm_streams(
        pcm,
        terminal_samples=terminal_samples,
        tail_training=tail_training,
        blocksizes=blocksizes,
    )
    available = (len(streams[0]) - window) // hop + 1
    if count is None:
        count = available
    if count < 0 or count > available:
        raise ValueError(f"requested {count} transient quanta; only {available} available")
    for quantum in range(count):
        start = quantum * hop
        yield tuple(
            tuple(channel[start : start + window]) for channel in streams
        )


__all__ = ["detector_pcm_streams", "iter_detector_quanta"]
