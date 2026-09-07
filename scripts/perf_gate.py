#!/usr/bin/env python3
"""Fixture-encode performance gate for the release build.

Measures the `wwise-wem` CLI's encode stage on the golden fixture
(``--time`` stage timings, release build) over N runs and gates on:

  * the median must stay under the normative cap (``cap_ms``, 150);
  * the median must not degrade more than ``regression_factor``
    (25%) against the recorded baseline median.

The baseline is a versioned asset (``tests/data/perf-baseline.json``);
re-record it only with an explicit decision (``--record``).

Usage:
  cargo build --release -p wem-core
  python3 scripts/perf_gate.py
  python3 scripts/perf_gate.py --bin /path/to/wwise-wem --runs 9

Exit codes: 0 green, 1 red (threshold breach, the gate reason is printed),
2 usage/environment error.
"""
from __future__ import annotations

import argparse
import json
import re
import statistics
import subprocess
import sys
from pathlib import Path

REPO = Path(__file__).resolve().parents[1]
FIXTURE = REPO / "tests" / "fixtures" / "input.wav"
BASELINE_FILE = REPO / "tests" / "data" / "perf-baseline.json"
DEFAULT_BIN = REPO / "crates" / "target" / "release" / "wwise-wem"
DEFAULT_RUNS = 7
ENCODE_PATTERN = re.compile(r"encode ([\d.]+)ms")


def _measure(bin_path: Path, runs: int) -> list[float]:
    """Encode-stage milliseconds for ``runs`` warm releases of the binary."""
    samples: list[float] = []
    for _ in range(runs):
        result = subprocess.run(
            [str(bin_path), str(FIXTURE), "--output", "/dev/null", "--time"],
            capture_output=True,
            text=True,
        )
        if result.returncode != 0:
            raise RuntimeError(
                f"wwise-wem exited {result.returncode}:\n{result.stderr}"
            )
        match = ENCODE_PATTERN.search(result.stderr)
        if match is None:
            raise RuntimeError(f"--time output missing encode stage:\n{result.stderr}")
        samples.append(float(match.group(1)))
    samples.sort()
    return samples


def _record_baseline(bin_path: Path, runs: int) -> float:
    samples = _measure(bin_path, runs)
    median = statistics.median(samples)
    baseline = json.loads(BASELINE_FILE.read_text(encoding="utf-8"))
    baseline["median_encode_ms"] = round(median, 2)
    baseline["runs"] = runs
    BASELINE_FILE.write_text(
        json.dumps(baseline, indent=2, sort_keys=False) + "\n",
        encoding="utf-8",
    )
    print(f"perf baseline re-recorded: median={median:.2f}ms")
    return median


def _parse_args(argv: list[str]) -> argparse.Namespace:
    parser = argparse.ArgumentParser(
        description=__doc__,
        formatter_class=argparse.RawDescriptionHelpFormatter,
    )
    parser.add_argument(
        "--bin",
        default=str(DEFAULT_BIN),
        help=f"path to the release wwise-wem binary (default: {DEFAULT_BIN})",
    )
    parser.add_argument(
        "--runs",
        type=int,
        default=DEFAULT_RUNS,
        help=f"number of warm runs (default: {DEFAULT_RUNS})",
    )
    parser.add_argument(
        "--record",
        action="store_true",
        help="re-record the baseline median from this measurement "
        "(use only with an explicit decision)",
    )
    return parser.parse_args(argv)


def main() -> int:
    args = _parse_args(sys.argv[1:])
    bin_path = Path(args.bin).expanduser()
    if not bin_path.is_file():
        print(
            f"perf gate: binary not found: {bin_path}; build it first with: "
            "cargo build --release -p wem-core",
            file=sys.stderr,
        )
        return 2

    if args.record:
        _record_baseline(bin_path, args.runs)
        return 0

    baseline = json.loads(BASELINE_FILE.read_text(encoding="utf-8"))
    cap_ms = float(baseline["cap_ms"])
    factor = float(baseline["regression_factor"])
    baseline_median = float(baseline["median_encode_ms"])

    samples = _measure(bin_path, args.runs)
    median = statistics.median(samples)
    regression_limit = baseline_median * factor

    print(
        f"encode median over {len(samples)} runs: {median:.2f}ms "
        f"(samples: {[round(sample, 2) for sample in samples]})"
    )
    print(
        f"limits: cap={cap_ms:.2f}ms, baseline={baseline_median:.2f}ms, "
        f"regression limit={regression_limit:.2f}ms"
    )

    if median > cap_ms:
        print(
            f"perf gate RED: median {median:.2f}ms exceeds the cap "
            f"{cap_ms:.2f}ms",
            file=sys.stderr,
        )
        return 1
    if median > regression_limit:
        print(
            f"perf gate RED: median {median:.2f}ms degraded more than "
            f"{(factor - 1) * 100:.0f}% against baseline "
            f"{baseline_median:.2f}ms (limit {regression_limit:.2f}ms)",
            file=sys.stderr,
        )
        return 1
    print("perf gate OK")
    return 0


if __name__ == "__main__":
    sys.exit(main())
