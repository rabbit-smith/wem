"""Command-line adapter for the typed WAV encoder API."""

from __future__ import annotations

import argparse
from pathlib import Path

from .api import encode
from .adapters.wav import read_pcm_wav
from .profiles.registry import profile_names


def main() -> None:
    parser = argparse.ArgumentParser(
        description="Encode a supported PCM WAV to a Wwise 2013.2 Vorbis WEM."
    )
    parser.add_argument("wav", type=Path)
    parser.add_argument("--profile", choices=profile_names())
    parser.add_argument("--quality", type=float, help="psy quality factor")
    parser.add_argument("--wwise-version", choices=("2013",), default="2013")
    parser.add_argument("--channels", type=int, help="assert input channel count")
    parser.add_argument("--sample-rate", type=int, help="assert input sample rate")
    parser.add_argument("--output", required=True, type=Path)
    parser.add_argument("--expect-sha256")
    args = parser.parse_args()

    pcm = read_pcm_wav(args.wav)
    if args.channels is not None and args.channels != pcm.channel_count:
        parser.error(
            f"--channels={args.channels} differs from WAV ({pcm.channel_count})"
        )
    if args.sample_rate is not None and args.sample_rate != pcm.sample_rate:
        parser.error(
            f"--sample-rate={args.sample_rate} differs from WAV ({pcm.sample_rate})"
        )

    result = encode(pcm, profile=args.profile, quality=args.quality)
    digest = result.sha256
    if args.expect_sha256 and digest.lower() != args.expect_sha256.lower():
        raise AssertionError(f"WEM SHA-256 differs: {digest} != {args.expect_sha256}")
    args.output.parent.mkdir(parents=True, exist_ok=True)
    args.output.write_bytes(result.data)
    print(
        "WAV to WEM OK",
        {
            **result.stats.to_dict(),
            "sha256": digest,
            "output": str(args.output),
        },
    )


if __name__ == "__main__":
    main()
