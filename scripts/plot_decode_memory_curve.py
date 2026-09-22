#!/usr/bin/env python3
"""Render the decode memory curve from the recorded samples.

``scripts/measure_decode_perf.py --rss --rss-json <path>`` records one child
process per stream length, each reporting the peak RSS it observed through its
own ``wait4`` rusage. This script turns that record into
``docs/figures/decode-memory-curve.png``.

What the figure is for: the decode surface is streaming-only so that a live
session's memory does not grow with the stream it is decoding. That is a claim
about the product, and this is the picture of it. The left panel is the
measurement; the right panel divides every point by the first, so the claim
reads as the horizontal line at 1.0 rather than as a judgement about whether a
slope "looks flat".

Two deliberate choices:

* **The band is the run spread, not an error bar.** ``ru_maxrss`` is resident
  pages and macOS reclaims a write-once buffer, so a point's spread across
  repetitions is part of the reading. Drawing the median alone would hide it.
* **The x axis is logarithmic** because the points are geometric multiples of
  one fixture (1×, 2×, 4×, …). On a linear axis the last point would own most
  of the width and the small-stream end, where a leak would show first, would
  be compressed into the margin.

Matplotlib is a documentation tool and is deliberately absent from
``pyproject.toml`` and from every crate. Run this from a venv of your own:

    ./.venv/bin/python scripts/plot_decode_memory_curve.py
    ./.venv/bin/python scripts/plot_decode_memory_curve.py --check

``--check`` renders to a temporary file and compares it with the committed one,
writing nothing.

Determinism, the same rules the concurrency figure follows: the figure size and
DPI are pinned, every series is sorted before it is drawn (a JSON object's key
order must not reach the image), and ``savefig`` is called with ``metadata={}``
so no timestamp is embedded. The rendered bytes are still a property of this
script *and the Matplotlib version* -- ``--check`` compares against a pinned
Matplotlib and changes when the version does.
"""
from __future__ import annotations

import argparse
import hashlib
import json
import sys
import tempfile
from pathlib import Path
from statistics import median
from typing import Any

REPO = Path(__file__).resolve().parent.parent
SAMPLES = REPO / "docs" / "figures" / "decode-memory-samples.json"
OUTPUT = REPO / "docs" / "figures" / "decode-memory-curve.png"

# Chosen, not default: the size is set for two panels a reader can read at a
# glance, and the DPI is pinned so the raster geometry is a property of this
# script rather than of the machine's display. `savefig` then uses
# ``figure.dpi``, the default being ``"figure"``.
FIGURE_SIZE_INCHES = (13.0, 5.2)
FIGURE_DPI = 110

MB = 1e6


def load_samples(path: Path) -> dict[str, Any]:
    """The recorded curve, validated enough that a wrong file names itself."""
    if not path.is_file():
        raise SystemExit(
            f"{path} does not exist: record it with\n"
            f"  python3 scripts/measure_decode_perf.py --rss "
            f"--rss-json {path}"
        )
    document = json.loads(path.read_text(encoding="utf-8"))
    expected = "wwise-wem.decode-memory-curve.v1"
    if document.get("schema") != expected:
        raise SystemExit(f"{path}: schema is {document.get('schema')!r}, not {expected!r}")
    if not document.get("points"):
        raise SystemExit(f"{path}: no points recorded")
    return document


def series(document: dict[str, Any]) -> list[dict[str, float]]:
    """One sorted row per stream length: seconds, median RSS and the spread."""
    rows = [
        {
            "seconds": float(point["audio_seconds"]),
            "frames": float(point["frames"]),
            "low": min(point["rss_bytes"]) / MB,
            "high": max(point["rss_bytes"]) / MB,
            "median": median(point["rss_bytes"]) / MB,
        }
        for point in document["points"]
    ]
    rows.sort(key=lambda row: row["seconds"])
    return rows


def draw(document: dict[str, Any], output: Path) -> None:
    import matplotlib

    matplotlib.use("Agg")
    import matplotlib.pyplot as plt

    rows = series(document)
    seconds = [row["seconds"] for row in rows]
    medians = [row["median"] for row in rows]
    lows = [row["low"] for row in rows]
    highs = [row["high"] for row in rows]
    first = medians[0]
    ratios = [value / first for value in medians]

    figure, (left, right) = plt.subplots(
        1, 2, figsize=FIGURE_SIZE_INCHES, dpi=FIGURE_DPI
    )
    geometry = f"{document['channels']} ch @ {document['sample_rate']} Hz"

    # Left: the measurement, with the run spread as a band.
    left.fill_between(seconds, lows, highs, alpha=0.25, linewidth=0, label="run spread")
    left.plot(seconds, medians, marker="o", label="median peak RSS")
    left.axhline(
        first,
        linestyle=":",
        linewidth=1.0,
        label=f"flat from the first point ({first:.1f} MB)",
    )
    left.set_xscale("log")
    left.set_xlabel("stream length (seconds of audio, log scale)")
    left.set_ylabel("peak RSS per decode (MB)")
    left.set_title(f"Peak RSS against stream length — {geometry}")
    left.grid(True, which="both", alpha=0.2)
    left.legend(loc="upper left", fontsize="small")

    # Right: the same numbers divided by their own first point, so the claim
    # reads as a line rather than as a judgement about a slope.
    right.plot(seconds, ratios, marker="o", label="median, relative to 1×")
    right.axhline(1.0, linestyle=":", linewidth=1.0, label="no growth")
    right.set_xscale("log")
    right.set_xlabel("stream length (seconds of audio, log scale)")
    right.set_ylabel("peak RSS / peak RSS at 1×")
    right.set_title("The same curve, relative to its own first point")
    right.grid(True, which="both", alpha=0.2)
    right.legend(loc="upper left", fontsize="small")

    figure.tight_layout()
    output.parent.mkdir(parents=True, exist_ok=True)
    figure.savefig(output, metadata={})
    plt.close(figure)


def main(argv: list[str] | None = None) -> int:
    parser = argparse.ArgumentParser(
        description="Render the decode memory curve from the recorded samples."
    )
    parser.add_argument(
        "--samples",
        type=Path,
        default=SAMPLES,
        help="recorded samples (default: %(default)s)",
    )
    parser.add_argument(
        "--out",
        type=Path,
        default=OUTPUT,
        help="where the figure is written (default: %(default)s)",
    )
    parser.add_argument(
        "--check",
        action="store_true",
        help="render to a temporary file and compare with --out, writing nothing",
    )
    args = parser.parse_args(argv)

    document = load_samples(args.samples)
    if args.check:
        if not args.out.is_file():
            raise SystemExit(f"nothing to compare against: {args.out} does not exist")
        with tempfile.TemporaryDirectory(prefix="wem-decode-memory-") as tmp:
            fresh = Path(tmp) / args.out.name
            draw(document, fresh)
            before = hashlib.sha256(args.out.read_bytes()).hexdigest()
            after = hashlib.sha256(fresh.read_bytes()).hexdigest()
        if before != after:
            print(f"differs: committed {before[:16]}… re-rendered {after[:16]}…")
            return 1
        print(f"byte-identical: {args.out} sha256 {before}")
        return 0

    draw(document, args.out)
    rows = series(document)
    print(
        f"wrote {args.out}: {len(rows)} points, "
        f"{rows[0]['seconds']:.1f} s -> {rows[-1]['seconds']:.1f} s, "
        f"median peak RSS {rows[0]['median']:.2f} -> {rows[-1]['median']:.2f} MB "
        f"({rows[-1]['median'] / rows[0]['median']:.2f}x)"
    )
    return 0


if __name__ == "__main__":
    sys.exit(main())
