"""Deep PCM-to-WEM encoder orchestration.

The :class:`Encoder` is a thin facade over the Rust kernel: every
byte-producing call runs on the in-package native extension
``wwise_wem._core`` (the abi3 binding built from ``crates/wem-python``,
shipped inside the wheel by the maturin backend).  There is no engine
selection, no fallback, and no second path: the extension is a required
runtime asset, and a missing one surfaces as the ordinary
:class:`ImportError` that the Python import machinery raises for
``wwise_wem._core``.

One construction path mirrors the kernel: an :class:`Encoder` owns one
structured ``WwiseProfile`` selection (a Wwise generation plus the PCM
geometry), which the kernel resolves against the configurations it carries.
"""

from __future__ import annotations

from typing import Any

import wwise_wem._core as _core
from ..model import PcmBuffer, WwiseWemError
from .models import EncodeResult, EncodeStats


class Encoder:
    """Own immutable codec inputs and run one complete encode per PCM buffer."""

    def __init__(
        self,
        selection: _core.WwiseProfile,
        *,
        quality: float | None = None,
    ) -> None:
        """Own a structured profile selection resolved by the kernel.

        ``selection`` names a Wwise generation plus the PCM geometry; the
        kernel resolves it against the profiles compiled into the extension,
        so an unsatisfiable selection is rejected there and never replaced by
        a default.  ``quality`` binds the quality factor before assembly:
        with a quality value the kernel interpolates the selected profile's
        quality curves, and without one the historical bytes are reproduced
        exactly.
        """
        if not isinstance(selection, _core.WwiseProfile):
            raise TypeError("selection must be WwiseProfile")
        self.selection: _core.WwiseProfile = selection
        self._quality = quality
        self._core_backend: Any = None

    def encode_pcm(self, pcm: PcmBuffer) -> EncodeResult:
        """Encode one independent PCM buffer into a complete WEM.

        The call runs on the native kernel (``wwise_wem._core``).  The
        kernel consumes integer signed-16 samples; the public float domain
        is ``value / 32768.0`` as produced by the adapters, so every sample
        must be exactly that of an integer in the signed-16 range.  Out-of-
        domain input is rejected with a plain :class:`ValueError` here,
        before the kernel is reached.
        """
        if not isinstance(pcm, PcmBuffer):
            raise TypeError("pcm must be PcmBuffer")
        if (pcm.channel_count, pcm.sample_rate) != self._geometry:
            raise ValueError(
                "PCM channel count/sample rate differs from encoder profile"
            )
        if pcm.frame_count < 4096:
            raise ValueError("PCM input must contain at least 4096 frames")

        rows = self._int16_rows(pcm)
        return self._encode_pcm_core(pcm, rows)

    def _int16_rows(self, pcm: PcmBuffer) -> list[list[int]]:
        """Convert in-domain float PCM rows into kernel signed-16 rows.

        The kernel consumes integer signed-16 samples; the public float
        domain is ``value / 32768.0`` as produced by the input adapters.
        Every sample must be exactly that of an integer in the signed-16
        range; anything else is rejected with a plain :class:`ValueError`.
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
                        "(expected the value/32768.0 signed-16 domain)"
                    )
                values.append(int(scaled))
            rows.append(values)
        return rows

    @property
    def _geometry(self) -> tuple[int, int]:
        """The selected PCM geometry as ``(channels, sample_rate)``."""
        return (self.selection.channels, self.selection.sample_rate)

    def _backend(self) -> Any:
        if self._core_backend is None:
            self._core_backend = _core.Encoder(self.selection, self._quality)
        return self._core_backend

    def encode_pcm16_interleaved(
        self,
        data: bytes,
        *,
        sample_rate: int,
        channels: int,
    ) -> EncodeResult:
        """Encode packed signed-16 input without expanding Python sample objects."""
        if (channels, sample_rate) != self._geometry:
            raise ValueError(
                "PCM channel count/sample rate differs from encoder profile"
            )
        bytes_per_frame = channels * 2
        if len(data) % bytes_per_frame:
            raise ValueError("PCM data does not align to whole frames")
        if len(data) // bytes_per_frame < 4096:
            raise ValueError("PCM input must contain at least 4096 frames")
        try:
            result = self._backend().encode_pcm16_interleaved(
                sample_rate,
                channels,
                data,
            )
        except _core.WemEncoderError as error:
            # The kernel's stable code class travels with the rejection; the
            # message stays the kernel's own diagnostic text.
            raise WwiseWemError(error.code, str(error)) from error
        return self._result_from_core(result)

    def _encode_pcm_core(self, pcm: PcmBuffer, rows: list[list[int]]) -> EncodeResult:
        """Run one encode on the native kernel and fill the Python DTOs.

        The quality factor owned by the encoder is forwarded to the kernel.
        Only what the kernel observed travels back: the container bytes and
        the statistics the caller cannot recompose from them.
        """
        try:
            result = self._backend().encode_pcm(pcm.sample_rate, rows)
        except _core.WemEncoderError as error:
            # A kernel rejection reaches the caller with its error class: the
            # code is the stable part of the kernel's surface (the same codes
            # every cross-language shell maps), the message is the kernel's
            # diagnostic text, and the original error stays reachable as
            # __cause__.  Nothing here reinterprets either of them.
            # (Backend construction can raise the same kernel error, e.g.
            # a quality request on a profile without quality-curves, or a
            # selection no installed profile satisfies.)
            raise WwiseWemError(error.code, str(error)) from error
        return self._result_from_core(result)

    def _result_from_core(self, result: Any) -> EncodeResult:
        stats = EncodeStats(
            pcm_frames=int(result.pcm_frames),
            channels=int(result.channels),
            audio_packets=int(result.audio_packets),
            short_packets=int(result.short_packets),
            long_packets=int(result.long_packets),
        )
        return EncodeResult(bytes(result.data), stats)


__all__ = ["Encoder"]
