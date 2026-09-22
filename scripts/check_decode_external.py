#!/usr/bin/env python3
"""Compare native streaming decode with an explicit external vgmstream executable.

The input stays local. PCM is compared in bounded blocks and is never recorded as
a baseline. Requires the dev extra and a built native extension. Example:

    python scripts/check_decode_external.py tests/fixtures/reference.wem
"""
from __future__ import annotations

import argparse
from contextlib import closing
import json
import struct
import subprocess
import sys
import tempfile
from pathlib import Path
from typing import BinaryIO

import numpy as np

ROOT = Path(__file__).resolve().parents[1]


def read_exact(stream: BinaryIO, size: int) -> bytes:
    parts = bytearray()
    while len(parts) < size:
        chunk = stream.read(size - len(parts))
        if not chunk:
            raise ValueError(f"external PCM ended with {size - len(parts)} bytes missing")
        parts.extend(chunk)
    return bytes(parts)


def float_wav_header(stream: BinaryIO) -> tuple[int, int, int]:
    """Read the ordinary float32 RIFF emitted by vgmstream's -W 4 option."""
    riff, _, wave = struct.unpack("<4sI4s", read_exact(stream, 12))
    if (riff, wave) != (b"RIFF", b"WAVE"):
        raise ValueError("external decoder did not emit a RIFF/WAVE stream")
    geometry = None
    while True:
        chunk, size = struct.unpack("<4sI", read_exact(stream, 8))
        if chunk == b"data":
            if geometry is None:
                raise ValueError("external WAV has no format before its PCM")
            channels, rate = geometry
            if size % (4 * channels):
                raise ValueError("external WAV ends with a partial PCM frame")
            return channels, rate, size // (4 * channels)
        if chunk == b"fmt ":
            if size < 16:
                raise ValueError("external WAV format is truncated")
            tag, channels, rate, _, align, bits = struct.unpack(
                "<HHIIHH", read_exact(stream, 16)
            )
            if tag != 3 or bits != 32 or not channels or align != 4 * channels:
                raise ValueError("external WAV must contain interleaved float32 PCM")
            geometry = channels, rate
            size -= 16
        # Discard metadata in bounded reads, including a possible pad byte.
        remaining = size + (size & 1)
        while remaining:
            amount = min(remaining, 65536)
            read_exact(stream, amount)
            remaining -= amount


def compare_pcm(native, external: BinaryIO, *, atol: float, rtol: float) -> dict:
    channels, rate, frames = float_wav_header(external)
    if (native.channels, native.sample_rate, native.total_frames) != (channels, rate, frames):
        raise ValueError("native and external declared PCM geometry differs")
    samples = 0
    squared_error = 0.0
    peak_error = 0.0
    for block in native:
        actual = np.asarray(block, dtype=np.float64)
        expected = np.frombuffer(read_exact(external, actual.size * 4), dtype="<f4")
        if not np.isfinite(actual).all() or not np.isfinite(expected).all():
            raise ValueError("non-finite decoded PCM")
        difference = np.abs(actual - expected)
        bad = np.flatnonzero(difference > atol + rtol * np.abs(expected))
        if bad.size:
            position = samples + int(bad[0])
            raise ValueError(
                f"PCM mismatch at frame {position // channels}, channel {position % channels}: "
                f"native={actual[bad[0]]}, external={expected[bad[0]]}"
            )
        samples += actual.size
        squared_error += float(np.dot(difference, difference))
        peak_error = max(peak_error, float(np.max(difference, initial=0.0)))
    if samples != frames * channels:
        raise ValueError(f"native decode delivered {samples} samples, expected {frames * channels}")
    if external.read(1):
        raise ValueError("external decoder emitted bytes after its declared PCM")
    return {
        "channels": channels, "sample_rate": rate, "frames": frames,
        "maximum_error": peak_error,
        "rms_error": (squared_error / samples) ** 0.5 if samples else 0.0,
        "absolute_tolerance": atol, "relative_tolerance": rtol,
    }


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("wem", type=Path)
    parser.add_argument("--vgmstream", default="vgmstream-cli")
    parser.add_argument("--atol", type=float, default=1e-6)
    parser.add_argument("--rtol", type=float, default=1e-6)
    args = parser.parse_args()
    if not all(np.isfinite(value) and value >= 0 for value in (args.atol, args.rtol)):
        parser.error("tolerances must be finite and nonnegative")
    sys.path.insert(0, str(ROOT / "src"))
    from wwise_wem import decode

    command = [args.vgmstream, "-i", "-W", "4", "-p", str(args.wem.resolve())]
    try:
        with closing(decode(args.wem)) as native, tempfile.TemporaryFile() as diagnostics:
            with subprocess.Popen(command, stdout=subprocess.PIPE, stderr=diagnostics) as process:
                try:
                    if process.stdout is None:
                        raise RuntimeError("external decoder stdout pipe was not opened")
                    metrics = compare_pcm(native, process.stdout, atol=args.atol, rtol=args.rtol)
                    status = process.wait()
                    if status:
                        diagnostics.seek(0)
                        raise ValueError(
                            f"external decoder exited {status}: "
                            + diagnostics.read(4096).decode(errors="replace")
                        )
                finally:
                    if process.poll() is None:
                        process.kill()
        print(json.dumps({"input": str(args.wem), "external": args.vgmstream, **metrics}))
        return 0
    except (OSError, ValueError) as error:
        print(str(error), file=sys.stderr)
        return 1


if __name__ == "__main__":
    raise SystemExit(main())
