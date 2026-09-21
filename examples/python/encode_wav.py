#!/usr/bin/env python3
"""Encode one WAV file through the package's single public entry point."""

from __future__ import annotations

import argparse
from pathlib import Path

from wwise_wem import encode


def main() -> None:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("input", type=Path, help="input RIFF/WAVE file")
    parser.add_argument("output", type=Path, help="output WEM file")
    parser.add_argument("--profile", help="optional installed profile name")
    parser.add_argument("--quality", type=float, help="optional profile quality")
    args = parser.parse_args()

    result = encode(args.input, profile=args.profile, quality=args.quality)
    args.output.write_bytes(result.data)
    print(
        f"wrote {len(result.data)} bytes to {args.output} "
        f"({result.stats.audio_packets} audio packets, sha256={result.sha256})"
    )


if __name__ == "__main__":
    main()
