"""Command-line adapter for the typed WAV encoder API."""

from __future__ import annotations

import argparse
from pathlib import Path

from .api import encode_wav
from .adapters.wav import read_wav_geometry
from .profiles.registry import PROFILES


def main() -> None:
    parser = argparse.ArgumentParser(
        description="Encode signed-16 PCM WAV to a Wwise 2013.2 Vorbis WEM."
    )
    parser.add_argument("wav", type=Path)
    source = parser.add_mutually_exclusive_group()
    source.add_argument("--template", type=Path)
    source.add_argument("--profile", choices=sorted(PROFILES))
    parser.add_argument("--wwise-version", choices=("2013",), default="2013")
    parser.add_argument("--channels", type=int, help="assert input channel count")
    parser.add_argument("--sample-rate", type=int, help="assert input sample rate")
    parser.add_argument("--output", required=True, type=Path)
    parser.add_argument("--expect-sha256")
    args = parser.parse_args()

    wav_channels, wav_rate = read_wav_geometry(args.wav)
    if args.channels is not None and args.channels != wav_channels:
        parser.error(f"--channels={args.channels} differs from WAV ({wav_channels})")
    if args.sample_rate is not None and args.sample_rate != wav_rate:
        parser.error(f"--sample-rate={args.sample_rate} differs from WAV ({wav_rate})")

    result = encode_wav(args.wav, args.template, profile=args.profile)
    digest = result.sha256
    if args.expect_sha256 and digest.lower() != args.expect_sha256.lower():
        raise AssertionError(f"WEM SHA-256 differs: {digest} != {args.expect_sha256}")
    args.output.parent.mkdir(parents=True, exist_ok=True)
    args.output.write_bytes(result.data)
    print(
        "WAV to WEM OK",
        {
            **result.stats.to_legacy_dict(),
            "sha256": digest,
            "output": str(args.output),
        },
    )


if __name__ == "__main__":
    main()
