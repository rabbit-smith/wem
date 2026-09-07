#!/usr/bin/env python3
"""Regenerate the stage-golden baseline assets from the checked fixture.

One-shot, idempotent, byte-stable:

    python3 scripts/emit_stage_golden.py

By default this re-derives every asset under tests/data/stage-golden/stages/
from tests/fixtures/input.wav through the real production encoder path and
rewrites index.json and the raw frame dumps.  Stale dumps are removed.
"""
from __future__ import annotations

import argparse
import json
import sys
from pathlib import Path

ROOT = Path(__file__).resolve().parents[1]
for entry in (ROOT, ROOT / "src"):
    if str(entry) not in sys.path:
        sys.path.insert(0, str(entry))

from tests.contract.stage_golden_support import (  # noqa: E402
    build_stage_golden,
    write_stage_golden,
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
        default=ROOT / "tests" / "data" / "stage-golden" / "stages",
        help="asset directory (default: tests/data/stage-golden/stages)",
    )
    parser.add_argument("--profile", default=None, help="encoder profile name")
    args = parser.parse_args()

    index_doc, dumps = build_stage_golden(args.wav, profile=args.profile)
    summary = write_stage_golden(index_doc, dumps, args.out)
    summary.update(
        {
            "profile": index_doc["profile"],
            "input_sha256": index_doc["input_sha256"],
            "wem_sha256": index_doc["container"]["wem_sha256"],
            "wem_size": index_doc["container"]["wem_size"],
        }
    )
    print(json.dumps(summary, indent=2, sort_keys=True))


if __name__ == "__main__":
    main()
