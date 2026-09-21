"""Command-line adapter for the typed WAV encoder API."""

from __future__ import annotations

import argparse
from pathlib import Path

from . import WwiseProfile, WwiseVersion
from .adapters.wav import read_pcm_wav
from .api import encode
from .profiles.registry import load_wem_profile, profile_names


def _parse_wwise_version(text: str) -> WwiseVersion:
    """Decode one ``--wwise-version`` spelling ("2013" or "2013.2")."""
    try:
        return WwiseVersion.parse(text)
    except ValueError as error:
        supported = ", ".join(version.label for version in WwiseVersion.ALL)
        raise ValueError(
            f"unsupported --wwise-version {text!r}; supported: {supported}"
        ) from error


def _installed_wwise_version(profile_name: str) -> WwiseVersion:
    """The Wwise generation of one installed profile."""
    generation = load_wem_profile(profile_name).key.generation
    try:
        return WwiseVersion.from_generation(generation)
    except ValueError as error:
        raise ValueError(
            f"installed profile {profile_name!r} has an unsupported Wwise "
            f"generation {generation!r}"
        ) from error


def main() -> None:
    parser = argparse.ArgumentParser(
        description="Encode a supported PCM WAV to a Wwise 2013.2 Vorbis WEM."
    )
    parser.add_argument("wav", type=Path)
    parser.add_argument("--profile", choices=profile_names())
    parser.add_argument("--quality", type=float, help="psy quality factor")
    parser.add_argument(
        "--wwise-version",
        default="2013",
        metavar="GENERATION",
        help="Wwise generation of the selected profile (2013 or 2013.2)",
    )
    parser.add_argument("--channels", type=int, help="assert input channel count")
    parser.add_argument("--sample-rate", type=int, help="assert input sample rate")
    parser.add_argument("--output", required=True, type=Path)
    parser.add_argument("--expect-sha256")
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

    selection: str | WwiseProfile
    if args.profile is not None:
        # The named profile is the selection; the requested generation must
        # agree with the generation that profile is installed under.
        installed = _installed_wwise_version(args.profile)
        if installed != version:
            parser.error(
                f"--wwise-version {version.label} differs from profile "
                f"{args.profile!r} ({installed.generation})"
            )
        selection = args.profile
    else:
        # No named profile: the requested generation plus the input geometry
        # is the whole selection, handed to the kernel as one WwiseProfile.
        selection = WwiseProfile(version, pcm.channel_count, pcm.sample_rate)

    result = encode(pcm, profile=selection, quality=args.quality)
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
