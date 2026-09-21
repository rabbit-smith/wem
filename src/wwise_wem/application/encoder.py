"""Deep PCM-to-WEM encoder orchestration.

The :class:`Encoder` is a thin facade over the Rust kernel: every
byte-producing call runs on the in-package native extension
``wwise_wem._core`` (the abi3 binding built from ``crates/wem-python``,
shipped inside the wheel by the maturin backend).  There is no engine
selection, no fallback, and no second path: the extension is a required
runtime asset, and a missing one surfaces as the ordinary
:class:`ImportError` that the Python import machinery raises for
``wwise_wem._core``.

Two equivalent constructions mirror the kernel: :meth:`Encoder.__init__`
owns one installed profile (resolved from its installed bundle), and
:meth:`Encoder.for_selection` owns a structured ``WwiseProfile`` selection
(resolved by the kernel against the configurations it carries).
"""

from __future__ import annotations

from typing import TYPE_CHECKING, Any

import wwise_wem._core as _core
from ..profiles.bundle import load_profile_bundle
from ..profiles.model import EncoderProfile
from ..model import PcmBuffer
from .models import EncodeResult, EncodeStats

if TYPE_CHECKING:
    from .._core import WwiseProfile


class Encoder:
    """Own immutable codec inputs and run one complete encode per PCM buffer."""

    def __init__(self, profile: EncoderProfile) -> None:
        if not isinstance(profile, EncoderProfile):
            raise TypeError("profile must be EncoderProfile")
        if tuple(profile.block_sizes) != (256, 2048):
            raise ValueError(
                "selected profile block geometry is unsupported; "
                "the installed runtime supports 256/2048 blocks"
            )

        bundle = load_profile_bundle(profile=profile.name, verify_all=False)
        if bundle.key != profile.key:
            raise ValueError("selected profile differs from installed profile bundle")
        if profile.setup_sha256 != bundle.setup.sha256:
            raise ValueError(
                f"selected profile {profile.name} differs from installed profile setup"
            )

        self.profile: EncoderProfile | None = profile
        self.selection: WwiseProfile | None = None
        self._quality: float | None = profile.quality
        self._core_backend: Any = None

    @classmethod
    def for_selection(
        cls,
        selection: WwiseProfile,
        *,
        quality: float | None = None,
    ) -> Encoder:
        """Own a structured profile selection resolved by the kernel.

        ``selection`` names a Wwise generation plus the PCM geometry; the
        kernel resolves it against the profiles compiled into the extension,
        so an unsatisfiable selection is rejected there and never replaced by
        a default.  ``quality`` binds the quality factor exactly as
        ``load_wem_profile(name, quality=...)`` does.
        """
        if not isinstance(selection, _core.WwiseProfile):
            raise TypeError("selection must be WwiseProfile")
        encoder = cls.__new__(cls)
        encoder.profile = None
        encoder.selection = selection
        encoder._quality = quality
        encoder._core_backend = None
        return encoder

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
        if self.selection is not None:
            return (self.selection.channels, self.selection.sample_rate)
        if self.profile is None:
            raise RuntimeError(
                "encoder carries neither an installed profile nor a selection"
            )
        return (self.profile.channels, self.profile.sample_rate)

    def _backend(self) -> Any:
        if self._core_backend is None:
            selection = self.selection
            profile = self.profile
            if selection is not None:
                self._core_backend = _core.Encoder(selection, self._quality)
            elif profile is not None:
                self._core_backend = _core.Encoder(profile.name, self._quality)
            else:
                raise RuntimeError(
                    "encoder carries neither an installed profile nor a selection"
                )
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
            raise ValueError(str(error)) from error
        return self._result_from_core(result)

    def _encode_pcm_core(self, pcm: PcmBuffer, rows: list[list[int]]) -> EncodeResult:
        """Run one encode on the native kernel and fill the Python DTOs.

        The quality factor owned by the encoder is forwarded to the kernel:
        with a quality value the kernel interpolates the selected profile's
        quality curves during assembly; without one the historical bytes are
        reproduced exactly.
        """
        try:
            result = self._backend().encode_pcm(pcm.sample_rate, rows)
        except _core.WemEncoderError as error:
            # The public contract surfaces input/configuration errors as
            # ValueError; kernel rejections reach the user only after all
            # Python-side validation passed, so the mapping preserves the
            # contract's error surface (message text is not contractual).
            # (Backend construction can raise the same kernel error, e.g.
            # a quality request on a profile without quality-curves, or a
            # selection no installed profile satisfies.)
            raise ValueError(str(error)) from error
        return self._result_from_core(result)

    def _result_from_core(self, result: Any) -> EncodeResult:
        profile = self.profile
        stats = EncodeStats(
            pcm_frames=int(result.pcm_frames),
            channels=int(result.channels),
            audio_packets=int(result.audio_packets),
            short_packets=int(result.short_packets),
            long_packets=int(result.long_packets),
            bytes=int(result.bytes_out),
            # An installed profile is named here; a structured selection is
            # named by the kernel, which resolved it against its bundle.
            metadata_source=(
                f"profile:{profile.name}"
                if profile is not None
                else str(result.metadata_source)
            ),
        )
        return EncodeResult(bytes(result.data), stats)


__all__ = ["Encoder"]
