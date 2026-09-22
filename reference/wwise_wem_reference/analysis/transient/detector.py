"""Per-channel transient detection and persistent band history."""

from __future__ import annotations

from dataclasses import dataclass, field
from typing import Sequence

from ..config import MdctLook, TransientDetectorTables
from ..dsp.spectrum import wwise_float_log
from ..._f32 import _f32
from ..dsp.transform import wwise_psy_mdct


@dataclass
class WwisePsyHistory:
    """Energy and band-history state for one detector channel."""

    energy_ring: list[float] = field(default_factory=lambda: [0.0] * 15)
    energy_sum: float = 0.0
    last_energy: float = 0.0
    cursor: int = 0
    band_rings: list[list[float]] = field(
        default_factory=lambda: [[0.0] * 17 for _ in range(12)]
    )
    band_cursors: list[int] = field(default_factory=lambda: [0] * 12)
    band_flags: int = 0



def _wwise_psy_band_update(
    mask: Sequence[float],
    history: WwisePsyHistory,
    *,
    history_window: int,
    config_row: Sequence[float],
    tables: TransientDetectorTables,
) -> int:
    """Update twelve band histories and return their combined flags.

    Configuration fields 1..12 hold upper thresholds and fields 13..24 hold
    lower thresholds.  Each band keeps a 17-slot ring.  Descriptor dot
    products round after every addition, making the stored float32 words part
    of the observable detector state.
    """
    if len(mask) < max(band.offset + len(band.weights) for band in tables.bands):
        raise ValueError("psycho mask is shorter than the band table")
    if len(history.band_rings) != 12 or len(history.band_cursors) != 12:
        raise ValueError("psycho band state must contain twelve bands")
    if len(config_row) < 26:
        raise ValueError("psycho config row must contain fields 0..25")
    # The shared history window advances before every channel in a quantum.
    # Its half count is clamped only for the history scan; the carry expression
    # uses the unclamped value.
    half_window = int(history_window) // 2
    window = max(2, half_window)
    carry = float(config_row[25]) - float(half_window - 2)
    carry = max(0.0, min(float(config_row[25]), carry))

    flags = 0
    for band, descriptor in enumerate(tables.bands):
        accumulator = 0.0
        for index, weight in enumerate(descriptor.weights):
            accumulator = _f32(
                accumulator + float(mask[descriptor.offset + index]) * weight
            )
        current = _f32(accumulator * descriptor.scale)
        ring = history.band_rings[band]
        cursor = history.band_cursors[band]
        previous = ring[(cursor - 1) % 17]
        current_max = max(current, previous)
        current_min = min(current, previous)
        history_max = -99999.0
        history_min = 99999.0
        for step in range(window):
            # The previous slot is folded into current_{min,max}; the
            # history scan starts one additional slot earlier.
            value = ring[(cursor - 2 - step) % 17]
            history_max = max(history_max, value)
            history_min = min(history_min, value)
        max_delta = current_max - history_max
        min_delta = current_min - history_min
        # The preceding cursor slot is read as ``previous``; the current
        # cursor slot is overwritten, then advanced.  This is visible in a
        # fresh history: all twelve values land at ring[0], not ring[16].
        ring[cursor] = current
        history.band_cursors[band] = (cursor + 1) % 17

        if float(config_row[1 + band]) + carry < max_delta:
            flags |= 5
        if float(config_row[13 + band]) - carry > min_delta:
            flags |= 2

    history.band_flags = flags
    return flags


def wwise_psy_mask(
    samples: Sequence[float],
    tables: TransientDetectorTables,
    mdct_look: MdctLook,
    history: WwisePsyHistory | None = None,
    *,
    bias: float | None = None,
    history_window: int = 1,
    config_row: Sequence[float] | None = None,
) -> list[float]:
    """Compute an MDCT mask and update twelve-band transient history.

    The returned values are the compressed pair mask consumed by the twelve
    psychoacoustic bands.  ``history.band_flags`` receives the block-decision
    bits for the supplied history window and configuration row.
    """
    if history is None:
        history = WwisePsyHistory()
    if len(history.energy_ring) != 15:
        raise ValueError("psycho energy ring must contain 15 slots")

    if len(samples) != tables.n:
        raise ValueError(f"transient mask requires {tables.n} samples")
    if bias is None:
        bias = tables.bias
    if config_row is None:
        config_row = tables.config
    spectrum = wwise_psy_mdct(samples, look=mdct_look, tables=tables)
    if len(spectrum) < 4 or len(spectrum) & 1:
        raise ValueError("psycho spectrum must contain an even number of bins")

    energy = _f32(
        spectrum[0] * spectrum[0]
        + spectrum[1] * 0.7 * spectrum[1]
        + spectrum[2] * 0.2 * spectrum[2]
    )
    slot = history.cursor
    if slot:
        average_source = _f32(history.last_energy + energy)
        history.last_energy = average_source
        history.energy_sum = _f32(energy + history.energy_sum)
    else:
        average_source = _f32(energy + history.energy_sum)
        history.last_energy = average_source
        history.energy_sum = energy
    history.last_energy = _f32(history.last_energy - history.energy_ring[slot])
    history.energy_ring[slot] = energy
    history.cursor += 1
    if history.cursor >= 15:
        history.cursor = 0

    avg = _f32(average_source * 0.0625)
    descending_floor = _f32(0.5 * wwise_float_log(abs(avg)) - 15.0)
    out: list[float] = []
    for i in range(len(spectrum) // 2):
        power = _f32(
            spectrum[2 * i] * spectrum[2 * i]
            + spectrum[2 * i + 1] * spectrum[2 * i + 1]
        )
        curve = _f32(0.5 * wwise_float_log(abs(power)))
        value = max(descending_floor, curve, bias)
        out.append(_f32(value))
        descending_floor = _f32(descending_floor - 8.0)
    _wwise_psy_band_update(
        out,
        history,
        history_window=history_window,
        config_row=config_row,
        tables=tables,
    )
    return out



@dataclass
class TransientDetector:
    """Own channel histories and combine their transient flags."""

    channels: int
    tables: TransientDetectorTables
    mdct_look: MdctLook
    bins: int = 128
    histories: list[WwisePsyHistory] = field(init=False)
    quanta: int = field(init=False, default=0)

    def __post_init__(self) -> None:
        if self.channels <= 0:
            raise ValueError("transient detector needs at least one channel")
        if self.bins != 128:
            raise ValueError("transient detector requires exactly 128 samples")
        if self.tables.n != self.bins or self.mdct_look.n != self.bins:
            raise ValueError("transient resources differ from detector geometry")
        self.reset()

    def reset(self) -> None:
        """Clear all channel histories and the quantum counter."""
        self.histories = [WwisePsyHistory() for _ in range(self.channels)]
        self.quanta = 0

    def analyze_quantum(
        self,
        pcm_by_channel: Sequence[Sequence[float]],
        *,
        history_window: int,
    ) -> int:
        """Analyze one channel-aligned quantum and combine its flags."""
        if len(pcm_by_channel) != self.channels:
            raise ValueError("transient quantum channel count differs from detector")
        if any(len(row) != self.bins for row in pcm_by_channel):
            raise ValueError(
                f"transient quantum must contain {self.bins} samples per channel"
            )
        flags = 0
        for samples, history in zip(pcm_by_channel, self.histories):
            wwise_psy_mask(
                samples,
                self.tables,
                self.mdct_look,
                history,
                history_window=history_window,
            )
            flags |= int(history.band_flags)
        self.quanta += 1
        return flags


__all__ = ["TransientDetector"]
