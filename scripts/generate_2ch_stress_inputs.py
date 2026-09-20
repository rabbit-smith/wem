#!/usr/bin/env python3
"""Generate the deterministic 2ch/48 kHz stress WAV inputs."""

from __future__ import annotations

import argparse
import sys
from pathlib import Path

ROOT = Path(__file__).resolve().parents[1]
sys.path.insert(0, str(ROOT))

from tests.two_channel_corpus_support import STRESS_CASES, verify_or_write_inputs  # noqa: E402


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--check", action="store_true", help="verify committed WAVs")
    args = parser.parse_args()
    destination = ROOT / "tests" / "data" / "2ch-stress"
    try:
        verify_or_write_inputs(
            STRESS_CASES, destination, destination / "manifest.json", check=args.check
        )
    except (OSError, ValueError) as error:
        print(f"ERROR: {error}", file=sys.stderr)
        return 1
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
