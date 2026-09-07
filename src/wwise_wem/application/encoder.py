"""Deep PCM-to-WEM encoder orchestration.

The :class:`Encoder` is a native-first facade: byte-producing calls run on
the Rust kernel (``_wwise_wem_native``) when the resolved engine is native
(see ``wwise_wem._engine`` and ``WWISE_WEM_ENGINE``), and otherwise on the
pure-Python reference implementation in the development-tree
``wwise_wem_reference`` package, which stays the reference oracle.  That
package is not part of the wheel, so an installed facade without the native
kernel reports a clear ``ImportError`` instead of running a half-built path.
"""

from __future__ import annotations

import importlib.util
from pathlib import Path
from typing import Any

from .. import _engine, _reference
from ..profiles.bundle import load_profile_bundle
from ..profiles.model import EncoderProfile
from ..model import PcmBuffer
from .models import EncodeResult, EncodeStats


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

        self.profile = profile
        self._native_backend: Any = None

    def encode_pcm(self, pcm: PcmBuffer) -> EncodeResult:
        """Encode one independent PCM buffer into a complete Wwise WEM.

        Engine dispatch (see ``wwise_wem._engine``): the native kernel is
        used when the resolved engine is native and the PCM values sit in
        the signed-16 sample domain, raising ``ValueError`` otherwise under
        an explicit ``native`` pin while ``auto`` routes such buffers to the
        pure-Python reference implementation.
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
        reference path rejects input errors (``ValueError``).
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
            metadata_source=f"profile:{self.profile.name}",
            engine="native",
        )
        return EncodeResult(bytes(result.data), stats)

    def _encode_pcm_python(self, pcm: PcmBuffer) -> EncodeResult:
        """Pure-Python reference encode path (oracle behavior)."""
        python_engine = _reference.reference_module("python_engine")
        container = python_engine.ContainerPlan.from_profile(self.profile)
        return python_engine.encode_pcm_python(
            profile=self.profile,
            container=container,
            pcm=pcm,
        )


__all__ = ["Encoder"]
