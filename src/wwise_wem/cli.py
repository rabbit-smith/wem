"""Command-line adapter for the typed WAV encoder API and the WEM decoder.

One command, two directions:

```text
wwise-wem INPUT.wav --output OUTPUT.wem          # encode (default)
wwise-wem INPUT.wem --decode --output OUTPUT.wav # decode
```

``--decode`` is a mode flag rather than a subcommand, for the reason the
positional argument exists: the command line is positional, so a ``decode``
subcommand would take the argv slot a file named ``decode`` already occupies.
A flag leaves every encode invocation — including one naming such a file —
exactly as it was, and reaches both front ends identically (the Rust binary
``crates/wem-core/src/bin/wwise-wem.rs`` takes the same flag with the same
meaning).

Decoded output is a signed-16 PCM WAV. Encode's command line writes a container
and decode's writes one too, rather than handing raw PCM to a caller who would
have to build a header; and signed-16 is the form the encode direction reads,
so a decoded file goes straight back into the same command line. The sample
mapping is :func:`wwise_wem.adapters.wav_writer.pcm16_wav_bytes` — the
package's own float-to-signed-16 rule — so this module holds no numerics of its
own.
"""

from __future__ import annotations

import argparse
import sys
from collections.abc import Iterable, Iterator
from contextlib import closing
from pathlib import Path

from . import WwiseProfile, WwiseVersion
from .adapters.wav import read_pcm_wav
from .adapters.wav_writer import pcm16_wav_bytes
from .api import decode, encode

# The encoder's options. Their defaults are ``argparse.SUPPRESS`` so that the
# parsed namespace carries exactly the options the caller wrote: that is what
# makes "was this given?" answerable when the mode is decode.
_ENCODER_OPTIONS = ("quality", "wwise_version", "channels", "sample_rate")


def _parse_wwise_version(text: str) -> WwiseVersion:
    """Decode one ``--wwise-version`` spelling ("2013" or "2013.2")."""
    try:
        return WwiseVersion.parse(text)
    except ValueError as error:
        supported = ", ".join(version.label for version in WwiseVersion.ALL)
        raise ValueError(
            f"unsupported --wwise-version {text!r}; supported: {supported}"
        ) from error


def _refuse_encoder_options(
    parser: argparse.ArgumentParser, args: argparse.Namespace
) -> None:
    """Refuse the encoder's options in decode mode.

    A WEM is self-describing: its setup packet fixes the geometry and there is
    no quality to choose, so none of these has anything to select or assert.
    Ignoring one would hide the caller's mistake; rejecting it reports that the
    command that was written is not the command that would run.
    """
    given = [name for name in _ENCODER_OPTIONS if hasattr(args, name)]
    if given:
        spellings = ", ".join("--" + name.replace("_", "-") for name in given)
        parser.error(
            "--decode reads a WEM, which is self-describing; "
            f"encoder options given: {spellings}"
        )


def _decode_to_wav(args: argparse.Namespace) -> None:
    """Decode ``args.wav`` (a WEM) into the signed-16 PCM WAV at ``args.output``.

    Nothing is written until the session has finished. A refusal only the audio
    packets can produce arrives while iterating, *after* the frames the earlier
    packets completed have been handed over — the kernel delivers that prefix
    and then reports its code — and a WAV built from those frames would present
    a prefix of the declared stream as a whole file. The refusal is reported
    instead, naming how much of the stream it left behind, and the output is
    never created.

    The frame count in the report is the container's declared one: a successful
    decode delivers exactly that many, and the writer counts the payload it
    actually holds.
    """
    delivered = 0

    def counted(blocks: Iterable[list[float]]) -> Iterator[list[float]]:
        """Yield decoded blocks unchanged, counting the frames they carry."""
        nonlocal delivered
        for block in blocks:
            delivered += len(block) // channels
            yield block

    try:
        with closing(decode(args.wav)) as result:
            channels = result.channels
            sample_rate = result.sample_rate
            declared = result.total_frames
            wav = pcm16_wav_bytes(channels, sample_rate, counted(result))
    except ValueError as error:
        # The kernel's refusal — ``WwiseWemError`` is a ``ValueError`` — and
        # the sample mapping's own rejection arrive the same way: one
        # diagnostic line and a non-zero status, without a traceback, which is
        # the shape the Rust command line reports.
        if delivered:
            note = (
                f"the session was refused after {delivered} of {declared} "
                "declared frames"
            )
        else:
            note = "the refusal came before any frame"
        print(f"wwise-wem: {error}; nothing written: {note}", file=sys.stderr)
        raise SystemExit(1)

    args.output.parent.mkdir(parents=True, exist_ok=True)
    args.output.write_bytes(wav)
    print(
        "WEM to WAV OK",
        {
            "channels": channels,
            "sample_rate": sample_rate,
            "frames": declared,
            "bytes": len(wav),
            "output": str(args.output),
        },
    )


def main() -> None:
    parser = argparse.ArgumentParser(
        description=(
            "Encode a supported PCM WAV to a Wwise 2013.2 Vorbis WEM, or "
            "decode a WEM back to a signed-16 PCM WAV with --decode."
        )
    )
    parser.add_argument(
        "wav",
        metavar="INPUT",
        type=Path,
        help="WAV to encode, or the WEM to decode with --decode",
    )
    parser.add_argument(
        "--quality", type=float, default=argparse.SUPPRESS, help="psy quality factor"
    )
    parser.add_argument(
        "--wwise-version",
        default=argparse.SUPPRESS,
        metavar="GENERATION",
        help="Wwise generation to encode for (2013 or 2013.2; default 2013)",
    )
    parser.add_argument(
        "--channels",
        type=int,
        default=argparse.SUPPRESS,
        help="assert input channel count",
    )
    parser.add_argument(
        "--sample-rate",
        type=int,
        default=argparse.SUPPRESS,
        help="assert input sample rate",
    )
    parser.add_argument("--output", required=True, type=Path)
    parser.add_argument(
        "--decode",
        action="store_true",
        help=(
            "decode INPUT, a WEM, to a signed-16 PCM WAV instead of encoding"
        ),
    )
    args = parser.parse_args()

    if args.decode:
        _refuse_encoder_options(parser, args)
        _decode_to_wav(args)
        return

    try:
        version = _parse_wwise_version(getattr(args, "wwise_version", "2013"))
    except ValueError as error:
        # A bad spelling is an argument error, reported the way argparse
        # reports every other one on this command line.
        parser.error(str(error))

    quality = getattr(args, "quality", None)
    channel_assertion = getattr(args, "channels", None)
    sample_rate_assertion = getattr(args, "sample_rate", None)

    pcm = read_pcm_wav(args.wav)
    if channel_assertion is not None and channel_assertion != pcm.channel_count:
        parser.error(
            f"--channels={channel_assertion} differs from WAV ({pcm.channel_count})"
        )
    if sample_rate_assertion is not None and sample_rate_assertion != pcm.sample_rate:
        parser.error(
            f"--sample-rate={sample_rate_assertion} differs from WAV ({pcm.sample_rate})"
        )

    selection = WwiseProfile(version, pcm.channel_count, pcm.sample_rate)
    result = encode(pcm, profile=selection, quality=quality)
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
