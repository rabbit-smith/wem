#!/usr/bin/env python3
"""Decode one WEM file through the package's single public decode entry point.

The output is the decoder's own samples, written exactly as they arrive:
interleaved, little-endian f32 at +-1.0 full scale, with no header. A WAV (or
any other container) would mean choosing a sample format and converting the
kernel's output on the way out — a surface this example does not own, and one
that could be subtly wrong while still looking plausible. The numbers printed
below are what makes the file interpretable, and `shasum -a 256` or `cmp`
against another example's output is the check.
"""

from __future__ import annotations

import argparse
import struct
from contextlib import closing
from pathlib import Path

from wwise_wem import decode


def main() -> None:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("input", type=Path, help="input WEM file")
    parser.add_argument(
        "output", type=Path, help="output raw interleaved little-endian f32 file"
    )
    args = parser.parse_args()

    # `decode` consumes the container's header region at call time, so the
    # geometry is readable before the first block and a rejection of the
    # container framing or of its setup packet is raised by the call itself.
    # The result owns one native decode session; `closing` is the documented
    # deterministic release.
    with closing(decode(args.input)) as result:
        frames = 0
        with args.output.open("wb") as out:
            for block in result:
                # `<`: the file format is little-endian f32 whatever the host
                # is; `block` is a flat list of one block's interleaved samples.
                out.write(struct.pack(f"<{len(block)}f", *block))
                frames += len(block) // result.channels

        print(
            f"wrote {args.output} "
            f"({frames} frames, {result.channels} channels, {result.sample_rate} Hz, "
            f"container declares {result.total_frames} frames)"
        )


if __name__ == "__main__":
    main()
