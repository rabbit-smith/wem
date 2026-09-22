#!/usr/bin/env python3
"""Render the concurrency curves from the recorded samples.

Draws ``docs/figures/concurrency-curves.png`` from
``docs/figures/concurrency-samples.json`` — the raw samples
``scripts/measure_concurrency.py`` recorded. The figure is a generated asset
and a committed one, so this script is **byte-stable**: running it twice leaves
an empty diff.

One record feeds the figure. The tree also commits a second, quieter record
(``concurrency-samples-quiet-window.json``) that the finding compares against
this one; it is deliberately not drawn, because that comparison is between two
runs rather than an extra series. ``--samples`` renders it if that is what you
want to look at.

Tool versions needed to reproduce it:
  * Python 3.14 (any 3.11+ should do; the code uses no version-specific
    syntax beyond PEP 604 unions)
  * Matplotlib 3.11.2

Matplotlib is a **documentation tool here, not a project dependency**: it is
not in ``pyproject.toml``, not in any crate, and not in the shared ``.venv``.
Install it in a venv of your own inside the tree (``.venv/`` is ignored) and
run this script with that interpreter::

    python3 -m venv .venv && ./.venv/bin/python -m pip install matplotlib
    ./.venv/bin/python scripts/plot_concurrency_curves.py

Style: **the default ``rcParams``, unmodified.** No style sheet, no
``rcParams.update``, no ``plt.xkcd``. The figure is meant to look like an
ordinary Matplotlib figure, because a reader who wants to change it should
only have to read the plot calls. The two settings that *are* chosen — figure
size and DPI — are chosen for legibility and for byte-stability, not for
appearance, and are named in ``FIGURE_SIZE_INCHES`` and ``FIGURE_DPI`` below.
The default colour cycle (``tab10``) and the default font (DejaVu Sans) are
left alone.

Byte-stability, and what makes it hold: the same input, the same code and the
same library give the same pixels, but the *container* is what usually moves.
What this script does about it:

  * the figure size and DPI are pinned, so the raster geometry cannot drift
    with a display or with a default;
  * every series is **sorted by N** before it is drawn, so the drawn path is a
    function of the data and not of the JSON's key order;
  * nothing is jittered and no random state is seeded, so there is no draw-time
    noise to reproduce;
  * ``savefig`` is called with ``metadata={}``.

Measured on this machine, Matplotlib 3.11.2, two renders of one dataset:
**PNG is byte-identical, and PDF is too. SVG is not.** PNG's only text chunk is
``Software``, carrying the Matplotlib version — no creation date is written at
all, with or without ``metadata={}`` — so PNG is stable for a pinned Matplotlib
and changes when the version does. SVG carries a wall-clock ``<dc:date>`` and
derives element ids and ``clip-path``/marker references from a per-process
value, so two runs differ in the ids even with ``metadata={"Date": None}``;
the documented remedy is the ``svg.hashsalt`` rcParam, and tuning an rcParam is
what the default-style rule above rules out. PNG is therefore the committed
format, and a regeneration check on a different Matplotlib version will report
a difference that is the version and not the data. The exact digests are in the
finding.

Usage:
  ./.venv/bin/python scripts/plot_concurrency_curves.py
  ./.venv/bin/python scripts/plot_concurrency_curves.py --check
  ./.venv/bin/python scripts/plot_concurrency_curves.py --tables
  ./.venv/bin/python scripts/plot_concurrency_curves.py --out "$TMPDIR/curve.png"

Exit codes: 0 rendered, 1 a required input is missing or unreadable, 2 usage.
"""
from __future__ import annotations

import argparse
import json
import sys
from pathlib import Path

REPO = Path(__file__).resolve().parents[1]
SAMPLES = REPO / "docs" / "figures" / "concurrency-samples.json"
OUTPUT = REPO / "docs" / "figures" / "concurrency-curves.png"

# Chosen, not default, and the only two rcParams this script touches:
#   * the size is set for legibility of a six-panel grid;
#   * the DPI is pinned so the raster geometry is a property of this script
#     rather than of the machine's display.
# `savefig` then uses ``figure.dpi`` because the default ``savefig.dpi`` is
# ``"figure"``, which is left as it is.
FIGURE_SIZE_INCHES = (11.0, 13.0)
FIGURE_DPI = 110

# Two series, one per feature configuration, from the default colour cycle.
CONFIG_STYLE = {
    "parallel": {"label": "internal parallel on", "color": "#1f77b4", "marker": "o"},
    "scalar": {"label": "internal parallel off", "color": "#ff7f0e", "marker": "s"},
}


def display(path: Path) -> str:
    """A path as a message should name it: repo-relative where it is in the tree."""
    try:
        return str(path.relative_to(REPO))
    except ValueError:
        return str(path)


def load_samples(path: Path) -> dict:
    if not path.is_file():
        raise SystemExit(
            f"missing {display(path)}\n"
            "record it first:\n"
            "  python3 scripts/measure_concurrency.py"
        )
    try:
        return json.loads(path.read_text(encoding="utf-8"))
    except json.JSONDecodeError as error:
        raise SystemExit(f"{display(path)} is not valid JSON: {error}") from error


def series_for(cells: list[dict], geometry: str, config: str, field: str) -> tuple[list[int], list[float]]:
    """One series, sorted by N.

    Sorting is what makes the drawn path a function of the data: the JSON's
    key order is an implementation detail of the writer, and a line drawn in
    that order would be a different picture for the same numbers.
    """
    pairs = sorted(
        (cell["n"], cell[field])
        for cell in cells
        if cell["geometry"] == geometry and cell["config"] == config and cell[field] is not None
    )
    return [n for n, _ in pairs], [value for _, value in pairs]


def draw(document: dict, output: Path) -> None:
    import matplotlib.pyplot as plt

    cells = document["cells"]
    geometries = [entry["name"] for entry in document["matrix"]["geometries"]]
    n_values = sorted(document["matrix"]["n_values"])

    figure, axes = plt.subplots(
        len(geometries), 3, figsize=FIGURE_SIZE_INCHES, dpi=FIGURE_DPI, squeeze=False
    )

    for row, geometry in enumerate(geometries):
        entry = next(e for e in document["matrix"]["geometries"] if e["name"] == geometry)
        audio_s = entry["audio_s"]

        # Column 1: throughput, in the unit a server plans with.
        axis = axes[row][0]
        for config, style in CONFIG_STYLE.items():
            n_axis, throughput = series_for(cells, geometry, config, "throughput_per_s")
            if not n_axis:
                continue
            axis.plot(n_axis, throughput, color=style["color"], marker=style["marker"],
                      label=style["label"])
            # The ideal-linear reference, anchored at that series' own N=1:
            # one free core per concurrent encode would hold this line.
            anchor = throughput[0]
            axis.plot(n_axis, [n * anchor for n in n_axis], color=style["color"],
                      linestyle=":", linewidth=1.0, alpha=0.55,
                      label=f"{style['label']}, ideal linear from N=1")
        axis.set_title(f"{geometry} — throughput")
        axis.set_xlabel("N concurrent encodes")
        axis.set_ylabel("encodes / s")
        axis.set_xticks(n_values)

        # Column 2: what one caller sees. Median solid, p95 dashed.
        axis = axes[row][1]
        for config, style in CONFIG_STYLE.items():
            n_axis, median = series_for(cells, geometry, config, "latency_ms_median")
            if not n_axis:
                continue
            axis.plot(n_axis, median, color=style["color"], marker=style["marker"],
                      label=f"{style['label']}, median")
            _, p95 = series_for(cells, geometry, config, "latency_ms_p95")
            axis.plot(n_axis, p95, color=style["color"], linestyle="--", linewidth=1.0,
                      marker=style["marker"], markersize=3,
                      label=f"{style['label']}, p95")
            # Perfect scaling keeps the caller's wait where it started.
            axis.plot(n_axis, [median[0]] * len(n_axis), color=style["color"],
                      linestyle=":", linewidth=1.0, alpha=0.55,
                      label=f"{style['label']}, N=1 level")
        axis.set_title(f"{geometry} — per-encode latency")
        axis.set_xlabel("N concurrent encodes")
        axis.set_ylabel("milliseconds")
        axis.set_xticks(n_values)

        # Column 3: the efficiency number. CPU per encode is the quantity that
        # survives a loaded machine, so it is drawn rather than tabulated only.
        axis = axes[row][2]
        for config, style in CONFIG_STYLE.items():
            n_axis, cpu = series_for(cells, geometry, config, "cpu_per_encode_ms_median")
            if not n_axis:
                continue
            axis.plot(n_axis, cpu, color=style["color"], marker=style["marker"],
                      label=style["label"])
            axis.plot(n_axis, [cpu[0]] * len(n_axis), color=style["color"],
                      linestyle=":", linewidth=1.0, alpha=0.55,
                      label=f"{style['label']}, N=1 level")
        axis.set_title(f"{geometry} — CPU per encode (user + sys)")
        axis.set_xlabel("N concurrent encodes")
        axis.set_ylabel("milliseconds")
        axis.set_xticks(n_values)

        axes[row][0].annotate(
            f"input {entry['input']}  ({audio_s:.3f} s, "
            f"{entry['channels']} ch @ {entry['sample_rate']} Hz)",
            xy=(0.0, -0.22), xycoords="axes fraction", fontsize=8, color="0.35",
        )

    for row in range(len(geometries)):
        for column in range(3):
            axes[row][column].grid(True, alpha=0.3)
            axes[row][column].legend(fontsize=7)

    figure.suptitle("Encoding throughput, per-encode latency and CPU per encode", fontsize=13)
    figure.tight_layout(rect=(0.0, 0.01, 1.0, 0.975))

    output.parent.mkdir(parents=True, exist_ok=True)
    figure.savefig(output, metadata={})
    plt.close(figure)


def markdown_tables(document: dict) -> str:
    """The finding's tables, rendered from the same JSON the figure uses."""
    cells = document["cells"]
    geometries = [entry["name"] for entry in document["matrix"]["geometries"]]
    lines: list[str] = []
    for geometry in geometries:
        entry = next(e for e in document["matrix"]["geometries"] if e["name"] == geometry)
        for config in ("parallel", "scalar"):
            rows = [c for c in cells if c["geometry"] == geometry and c["config"] == config]
            rows.sort(key=lambda c: c["n"])
            if not rows:
                continue
            lines.append(f"\n### {geometry} — {config} ({entry['input']})\n")
            lines.append(
                "| N | encodes/s | audio-s/s | speedup vs N=1 | ideal encodes/s | "
                "latency med ms | latency p95 ms | encode-stage med ms | "
                "batch CPU ms | CPU/encode ms | cores busy | "
                "RSS/child MB | resident for N MB | load 1m (min/med/max) |"
            )
            lines.append(
                "| ---: | ---: | ---: | ---: | ---: | ---: | ---: | ---: | ---: | "
                "---: | ---: | ---: | ---: | ---: |"
            )
            for cell in rows:
                lines.append(
                    f"| {cell['n']} | {cell['throughput_per_s']:.2f} | "
                    f"{cell['audio_s_per_s']:.2f} | {cell['speedup_vs_n1']:.2f}× | "
                    f"{cell['ideal_throughput_per_s']:.1f} | "
                    f"{cell['latency_ms_median']:.1f} | {cell['latency_ms_p95']:.1f} | "
                    f"{cell['encode_ms_median']:.1f} | "
                    f"{cell['cpu_total_ms_median']:.1f} | "
                    f"{cell['cpu_per_encode_ms_median']:.1f} | "
                    f"{cell['cores_busy_median']:.2f} | "
                    f"{cell['peak_rss_mb_median']:.1f} | "
                    f"{cell['projected_resident_mb']:.0f} | "
                    f"{cell['load_1min_min']:.0f}/{cell['load_1min_median']:.0f}/"
                    f"{cell['load_1min_max']:.0f} |"
                )
    lines.append("\n### paired parallel / scalar, median over repetitions\n")
    lines.append("| geometry | N | pairs | throughput ratio | latency ratio | CPU-per-encode ratio |")
    lines.append("| --- | ---: | ---: | ---: | ---: | ---: |")
    for row in document["paired"]:
        lines.append(
            f"| {row['geometry']} | {row['n']} | {row['pairs']} | "
            f"{row['throughput_ratio_median']:.2f}× | {row['latency_ratio_median']:.2f}× | "
            f"{row['cpu_per_encode_ratio_median']:.2f}× |"
        )
    return "\n".join(lines)


def main(argv: list[str] | None = None) -> int:
    parser = argparse.ArgumentParser(
        description="Render the concurrency curves from the recorded samples."
    )
    parser.add_argument("--samples", type=Path, default=SAMPLES,
                        help="recorded samples (default: %(default)s)")
    parser.add_argument("--out", type=Path, default=OUTPUT,
                        help="where the figure is written (default: %(default)s)")
    parser.add_argument("--tables", action="store_true",
                        help="print the finding's Markdown tables and render nothing")
    parser.add_argument("--check", action="store_true",
                        help="render to a temporary file and compare with --out, writing nothing")
    args = parser.parse_args(argv)

    document = load_samples(args.samples)
    if args.tables:
        print(markdown_tables(document))
        return 0

    if args.check:
        import hashlib
        import tempfile

        if not args.out.is_file():
            raise SystemExit(f"nothing to compare against: {args.out} does not exist")
        with tempfile.TemporaryDirectory(prefix="wem-curves-") as tmp:
            fresh = Path(tmp) / args.out.name
            draw(document, fresh)
            before = hashlib.sha256(args.out.read_bytes()).hexdigest()
            after = hashlib.sha256(fresh.read_bytes()).hexdigest()
        if before != after:
            print(f"differs: committed {before[:16]}… re-rendered {after[:16]}…")
            return 1
        print(f"byte-identical: {display(args.out)} sha256 {before}")
        return 0

    draw(document, args.out)
    print(f"wrote {display(args.out)}")
    return 0


if __name__ == "__main__":
    sys.exit(main())
