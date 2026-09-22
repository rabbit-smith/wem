"""One WEM in, interleaved f32 PCM out, over the native extension.

The decode counterpart of :mod:`wwise_wem.application.encoder`, and the same
kind of shell: every step runs on the in-package native extension
``wwise_wem._core`` (the abi3 binding built from ``crates/wem-python``), there
is no engine selection and no fallback, and a missing extension surfaces as the
ordinary :class:`ImportError` the import machinery raises for
``wwise_wem._core``.

:meth:`DecodeResult.open` is a plain classmethod, not a generator function: it
opens the native session and consumes the container's header region at call
time, so the geometry is readable before the first block and a refusal of the
container framing or the setup packet is raised by the call itself — the
error timing ``encode`` already documents. A generator function would defer
all of that to the first ``next()``.
"""

from __future__ import annotations

from collections import deque
from collections.abc import Iterator
from typing import NoReturn

import wwise_wem._core as _core
from ..model import WwiseWemError

# WEM bytes handed to the native session per step. This is the result object's
# pacing and not a parameter: the kernel's samples do not depend on how the
# stream is chunked (include/wem.h section 5), and a caller that needs a
# specific block size can regroup what it is handed.
_PUSH_BYTES = 8192

# The code the kernel reports for a fault in the library itself. It is the
# class a decode result reports when the session resolved no geometry and no
# refusal either — an outcome the kernel's own contract makes unreachable, and
# one that must fail loudly rather than be papered over with a guess.
_INTERNAL = "INTERNAL"


def _raise_from_core(error: _core.WemEncoderError) -> NoReturn:
    """Re-raise one kernel rejection as the facade's error.

    The kernel's stable code becomes ``code``, its diagnostic text becomes the
    message (and ``str(error)``), and the kernel error itself stays reachable
    as ``__cause__`` — exactly how the encoder path maps the same boundary.
    """
    raise WwiseWemError(error.code, str(error)) from error


class DecodeResult(Iterator[list[float]]):
    """The decoded PCM of one WEM, as a one-shot iterator of f32 blocks.

    Instances come from :func:`wwise_wem.decode`. ``channels``,
    ``sample_rate`` and ``total_frames`` are the container's own declaration
    and are readable before the first block.

    Each block is a list of interleaved f32 samples at ±1.0 full scale, in
    bounded blocks (1024 frames each today — the delivery size of the C ABI's
    PCM callback). A decoded sample outside ±1.0 is passed through rather than
    clipped: the library owns no clipping policy.

    The object owns one native decode session. It is released when the object
    is collected (ownership by value and Rust ``Drop``, the lifecycle the
    extension's ``StreamSession`` already has) or deterministically by
    ``close()`` — the method generators already have, so
    ``contextlib.closing(result)`` needs no new protocol.

    Iterating consumes the session, like a generator. A rejection the
    audio-packet stream produces is raised from the iteration step that
    reaches it, *after* the blocks the earlier packets completed have been
    handed over: the kernel delivers the prefix a refusal completed and then
    reports its code, and dropping that prefix would be the silent truncation
    its contract forbids. Once a rejection has been raised the result is
    exhausted — the kernel repeats the same rejection for every later step, so
    there is nothing after it to iterate.
    """

    def __init__(
        self,
        session: _core.Decoder,
        source: bytes,
        cursor: int,
        blocks: deque[list[float]],
        pending: _core.WemEncoderError | None,
        *,
        channels: int,
        sample_rate: int,
        total_frames: int,
    ) -> None:
        self._session: _core.Decoder | None = session
        self._source = source
        self._cursor = cursor
        self._blocks = blocks
        self._pending: _core.WemEncoderError | None = pending
        self._finished = False
        self._channels = channels
        self._sample_rate = sample_rate
        self._total_frames = total_frames

    @classmethod
    def open(cls, source: bytes) -> DecodeResult:
        """Open a decode session over ``source`` and resolve its geometry.

        This is the call-time work: the container's header region is pushed
        until the native session announces the geometry (channels, sample
        rate) and the declared frame count is readable. A refusal of the
        container framing, of the setup packet, or of the geometry it names is
        raised here; a refusal only the audio packets can produce is raised
        while iterating, after the frames the earlier packets completed have
        been handed over.
        """
        session = _core.Decoder()
        blocks: deque[list[float]] = deque()
        cursor = 0
        announced: _core.DecodedHeader | None = None
        pending: _core.WemEncoderError | None = None
        while announced is None and cursor < len(source):
            chunk = source[cursor : cursor + _PUSH_BYTES]
            cursor += len(chunk)
            step = session.push(chunk)
            blocks.extend(step.pcm)
            announced = step.header
            if step.error is not None:
                pending = step.error
                break

        if announced is None:
            # No geometry: nothing was decoded, so the refusal belongs to the
            # call itself. When the source simply ran out, `finish` is what
            # reports it (a container region that never resolved, or a data
            # payload that carried no setup packet).
            if pending is None:
                pending = session.finish().error
            if pending is None:
                raise WwiseWemError(
                    _INTERNAL,
                    "the native decode session resolved no geometry and "
                    "reported no refusal",
                )
            _raise_from_core(pending)

        total_frames = session.total_frames
        if total_frames is None:
            # The geometry was announced, so the container's header region is
            # the session's to read: a declared frame count it cannot read is a
            # defect, never a value to guess.
            raise WwiseWemError(
                _INTERNAL,
                "the container's declared frame count is not readable after "
                "the header announcement",
            )
        return cls(
            session,
            source,
            cursor,
            blocks,
            pending,
            channels=int(announced.channels),
            sample_rate=int(announced.sample_rate),
            total_frames=int(total_frames),
        )

    @property
    def channels(self) -> int:
        """PCM channel count the container declares."""
        return self._channels

    @property
    def sample_rate(self) -> int:
        """PCM sample rate the container declares."""
        return self._sample_rate

    @property
    def total_frames(self) -> int:
        """Frames the decode delivers: the container's ``dw_total_pcm_frames``."""
        return self._total_frames

    def __iter__(self) -> DecodeResult:
        """The result is its own iterator: it owns one live decode session."""
        return self

    def __next__(self) -> list[float]:
        while not self._blocks:
            session = self._session
            if session is None:
                # Released (`close()`) or exhausted: the result is a one-shot
                # iterator, and a released one has nothing left to hand over.
                raise StopIteration
            if self._pending is not None:
                pending, self._pending = self._pending, None
                # The rejection ends the result: the kernel keeps the bytes it
                # could not read pending and would report the same refusal for
                # every later step.
                self._release()
                _raise_from_core(pending)
            if self._finished and self._cursor >= len(self._source):
                self._release()
                raise StopIteration
            self._advance(session)
        return self._blocks.popleft()

    def close(self) -> None:
        """Release the native session and stop iteration.

        Idempotent, and the same release a generator's ``close()`` performs, so
        ``contextlib.closing(result)`` works on a decode result. The geometry
        the result already announced stays readable.
        """
        self._release()

    def _advance(self, session: _core.Decoder) -> None:
        """Push the next chunk, or finish the stream, and queue what it made."""
        if self._cursor >= len(self._source):
            step = session.finish()
            self._finished = True
        else:
            chunk = self._source[self._cursor : self._cursor + _PUSH_BYTES]
            self._cursor += len(chunk)
            step = session.push(chunk)
        self._blocks.extend(step.pcm)
        if step.error is not None:
            self._pending = step.error

    def _release(self) -> None:
        """Drop the native session and everything the result still held."""
        self._session = None
        self._source = b""
        self._cursor = 0
        self._blocks.clear()
        self._pending = None


__all__ = ["DecodeResult"]
