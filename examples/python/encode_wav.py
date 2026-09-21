#!/usr/bin/env python3
"""Encode one WAV file through the package's single public entry point."""

from __future__ import annotations

import argparse
from pathlib import Path

from wwise_wem import WwiseProfile, WwiseVersion, encode


def main() -> None:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("input", type=Path, help="input RIFF/WAVE file")
    parser.add_argument("output", type=Path, help="output WEM file")
    parser.add_argument(
        "--wwise-version",
        default=None,
        help="optional Wwise generation (2013 or 2013.2); default: the installed one",
    )
    parser.add_argument("--quality", type=float, help="optional profile quality")
    args = parser.parse_args()

    # Without --wwise-version the installed generation is selected for the
    # geometry the input declares; with one, the two together are the selection.
    profile = None
    if args.wwise_version is not None:
        from wwise_wem.adapters.wav import read_pcm_wav

        pcm = read_pcm_wav(args.input)
        profile = WwiseProfile(
            WwiseVersion.parse(args.wwise_version),
            pcm.channel_count,
            pcm.sample_rate,
        )

    result = encode(args.input, profile=profile, quality=args.quality)
    args.output.write_bytes(result.data)
    print(
        f"wrote {len(result.data)} bytes to {args.output} "
        f"({result.stats.audio_packets} audio packets, sha256={result.sha256})"
    )


if __name__ == "__main__":
    main()
