"""Immutable values passed between scheduling, analysis and packet layers."""

from __future__ import annotations

from dataclasses import dataclass

from .preprocessing.windowing import WindowedFrame


FloatRows = tuple[tuple[float, ...], ...]


@dataclass(frozen=True)
class SpectrumFrame:
    """Transform-domain values and peak state for one windowed frame."""

    window: WindowedFrame
    coefficients: FloatRows
    raw_mdct: FloatRows
    fft: FloatRows
    channel_specmax: tuple[float, ...]
    global_specmax: float


@dataclass(frozen=True)
class PsyFrame:
    """Psychoacoustic surfaces paired with their source spectrum."""

    spectrum: SpectrumFrame
    remap: FloatRows
    seed: FloatRows
    post: FloatRows
    side: FloatRows
    coupling_peak: FloatRows

    @property
    def window(self) -> WindowedFrame:
        return self.spectrum.window

    @property
    def coefficients(self) -> FloatRows:
        return self.spectrum.coefficients

    @property
    def raw_mdct(self) -> FloatRows:
        return self.spectrum.raw_mdct

    @property
    def fft(self) -> FloatRows:
        return self.spectrum.fft

    @property
    def channel_specmax(self) -> tuple[float, ...]:
        return self.spectrum.channel_specmax

    @property
    def global_specmax(self) -> float:
        return self.spectrum.global_specmax


__all__ = ["PsyFrame", "SpectrumFrame"]
