#!/usr/bin/env python3
"""Regenerate the stage-records baseline assets from the checked fixture.

One-shot, idempotent, byte-stable:

    python3 scripts/emit_stage_records.py

By default this re-derives every asset under tests/data/stage-records/stages/
from tests/fixtures/input.wav through the real production encoder path and
rewrites index.json and the raw frame dumps.  Stale dumps are removed.
"""
from __future__ import annotations

import argparse
import json
import sys
from pathlib import Path

ROOT = Path(__file__).resolve().parents[1]
# `reference` carries the pure-Python oracle the stage support drives, so the
# script resolves both trees from this file and needs no PYTHONPATH.
for entry in (ROOT, ROOT / "src", ROOT / "reference"):
    if str(entry) not in sys.path:
        sys.path.insert(0, str(entry))

from tests.parity.stage_records_support import (  # noqa: E402
    build_stage_records,
    write_stage_records,
)


def main() -> None:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument(
        "--wav",
        type=Path,
        default=ROOT / "tests" / "fixtures" / "input.wav",
        help="PCM WAV fixture to encode (default: tests/fixtures/input.wav)",
    )
    parser.add_argument(
        "--out",
        type=Path,
        default=ROOT / "tests" / "data" / "stage-records" / "stages",
        help="asset directory (default: tests/data/stage-records/stages)",
    )
    parser.add_argument(
        "--wwise-version",
        default=None,
        help="Wwise generation (2013 or 2013.2); default: the installed one",
    )
    args = parser.parse_args()

    selection = None
    if args.wwise_version is not None:
        from wwise_wem import WwiseProfile, WwiseVersion  # noqa: PLC0415
        from wwise_wem.adapters.wav import read_pcm_wav  # noqa: PLC0415

        pcm = read_pcm_wav(args.wav)
        selection = WwiseProfile(
            WwiseVersion.parse(args.wwise_version),
            pcm.channel_count,
            pcm.sample_rate,
        )

    index_doc, dumps = build_stage_records(args.wav, selection=selection)
    summary = write_stage_records(index_doc, dumps, args.out)
    summary.update(
        {
            "profile": index_doc["profile"],
            "wem_sha256": index_doc["container"]["wem_sha256"],
            "wem_size": index_doc["container"]["wem_size"],
        }
    )
    print(json.dumps(summary, indent=2, sort_keys=True))


if __name__ == "__main__":
    main()
