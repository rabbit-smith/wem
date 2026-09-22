"""Command-line adapter for the typed WAV encoder API."""

from __future__ import annotations

import argparse
from pathlib import Path

from . import WwiseProfile, WwiseVersion
from .adapters.wav import read_pcm_wav
from .api import encode


def _parse_wwise_version(text: str) -> WwiseVersion:
    """Decode one ``--wwise-version`` spelling ("2013" or "2013.2")."""
    try:
        return WwiseVersion.parse(text)
    except ValueError as error:
        supported = ", ".join(version.label for version in WwiseVersion.ALL)
        raise ValueError(
            f"unsupported --wwise-version {text!r}; supported: {supported}"
        ) from error


def main() -> None:
    parser = argparse.ArgumentParser(
        description="Encode a supported PCM WAV to a Wwise 2013.2 Vorbis WEM."
    )
    parser.add_argument("wav", type=Path)
    parser.add_argument("--quality", type=float, help="psy quality factor")
    parser.add_argument(
        "--wwise-version",
        default="2013",
        metavar="GENERATION",
        help="Wwise generation to encode for (2013 or 2013.2)",
    )
    parser.add_argument("--channels", type=int, help="assert input channel count")
    parser.add_argument("--sample-rate", type=int, help="assert input sample rate")
    parser.add_argument("--output", required=True, type=Path)
    args = parser.parse_args()

    try:
        version = _parse_wwise_version(args.wwise_version)
    except ValueError as error:
        # A bad spelling is an argument error, reported the way argparse
        # reports every other one on this command line.
        parser.error(str(error))

    pcm = read_pcm_wav(args.wav)
    if args.channels is not None and args.channels != pcm.channel_count:
        parser.error(
            f"--channels={args.channels} differs from WAV ({pcm.channel_count})"
        )
    if args.sample_rate is not None and args.sample_rate != pcm.sample_rate:
        parser.error(
            f"--sample-rate={args.sample_rate} differs from WAV ({pcm.sample_rate})"
        )

    selection = WwiseProfile(version, pcm.channel_count, pcm.sample_rate)
    result = encode(pcm, profile=selection, quality=args.quality)
    args.output.parent.mkdir(parents=True, exist_ok=True)
    args.output.write_bytes(result.data)
    print(
        "WAV to WEM OK",
        {
            **result.stats.to_dict(),
            "bytes": len(result),
            "output": str(args.output),
        },
    )


if __name__ == "__main__":
    main()
