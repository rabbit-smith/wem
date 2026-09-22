#!/usr/bin/env python3
"""Measure the duration curves: peak RSS, CPU, wall time, and the stage split.

Answers the owner's question — *"we realistically face music of five minutes or
less: what do the memory and CPU peak curves look like, and is anything still
worth optimizing?"* — by running the two encode paths at several durations in
both installed geometries and reporting the shape.  It has **no thresholds**:
nothing here decides whether a number is acceptable, and there is no recorded
baseline to compare against.  A loaded machine changes the numbers, never the
exit status.

What it drives
--------------

``crates/wem-core/tests/duration_curves.rs`` (the instrument, ``#[ignore]``d):
one worker per path — ``noop`` (process baseline), ``load`` (caller PCM only),
``batch`` (``Encoder::encode_pcm``), ``stream`` (``StreamSession`` fed from a
streaming reader) — and a parent reporter that forks one worker and prints its
peak RSS, user CPU, system CPU and wall time from ``wait4().rusage``.  The
Python driver owns the matrix, the ordering, and the statistics; the harness
prints raw samples.

``crates/wem-core/tests/stage_timings.rs`` (taken from
``codex/measure-and-investigate`` plus an additive duration driver) supplies the
per-stage split at the requested durations.

Inputs come from ``scripts/generate_duration_curve_inputs.py`` and land in a
gitignored scratch tree (``crates/target/curves/inputs``).  Nothing is
downloaded.

Honesty rules this script implements
------------------------------------

* **Paired, order-alternated rounds.**  ``batch`` and ``stream`` are measured
  next to each other inside one round, and which of the two goes first flips
  every round, so a drift in machine load cannot be mistaken for a difference
  between the two paths.
* **Raw samples, min/median/max/spread.**  Single samples are never reported as
  a curve.
* **Contaminated points are re-run, not averaged in.**  A point whose slowest
  wall sample exceeds ``--contamination-factor`` x its median gets extra rounds
  and is flagged in the report.
* **Machine load is recorded with every point** (1-minute load average before
  and after, plus a whole-machine busy-CPU sample per round), and the whole run
  is labelled provisional when load is high.

Usage::

    # the whole matrix on the opt-in (parallel) release build
    PATH="$HOME/.rustup/toolchains/stable-aarch64-apple-darwin/bin:$PATH" \\
        PYTHONDONTWRITEBYTECODE=1 PYTHONPATH=src:reference \\
        .venv/bin/python scripts/measure_duration_curves.py

    # the scalar split at the longest duration: the default build is scalar
    ... scripts/measure_duration_curves.py --tag scalar \\
        --durations 300 --paths batch,stream --reps 3

    # re-render this report from an existing JSON result
    ... scripts/measure_duration_curves.py --report-only --json <result.json>

Exit codes: 0 measured, 1 measurement failed, 2 usage/environment error.
"""
from __future__ import annotations

import argparse
import json
import math
import os
import platform
import re
import statistics
import subprocess
import sys
import time
from dataclasses import dataclass, field
from pathlib import Path

REPO = Path(__file__).resolve().parents[1]
CRATES = REPO / "crates"
DEFAULT_INPUTS = CRATES / "target" / "curves" / "inputs"
DEFAULT_SCRATCH = CRATES / "target" / "curves"
RUSTUP_BIN = Path.home() / ".rustup" / "toolchains" / "stable-aarch64-apple-darwin" / "bin"

DEFAULT_DURATIONS = (10, 30, 60, 120, 300, 600)
#: geometry label -> the profile selection the harness builds.
DEFAULT_GEOMETRIES = (("6ch44100", "fixture"), ("2ch48000", "two_channel"))
DEFAULT_PATHS = ("batch", "stream")
#: Paths measured once per (geometry, duration) as references, not per round.
#: `load` is what the caller's PCM costs; `rows`/`condition`/`detector`/
#: `source`/`windows` replay `encode_pcm`'s own statements up to a point and so
#: decompose the `batch` peak into named terms instead of one number.
REFERENCE_PATHS = ("noop", "load", "rows", "condition", "detector", "source", "windows")
#: The order the two encode paths are run in inside a round; it flips per round.
PAIRED_PATHS = ("batch", "stream")
#: Durations at or above this get `--reps-long` rounds instead of `--reps`.
LONG_SECONDS = 300
DEFAULT_STAGE_DURATIONS = (10, 60, 300)
#: Durations at which the output side (`finish`) is decomposed, when they were
#: measured. Requires `batch,stream,stream_push` in `--paths`.
DEFAULT_OUTPUT_DURATIONS = (300, 600)

POINT_TEST = "duration_curves_point"
STAGE_TEST = "stage_timings_duration_report"
RESULT_PATTERN = re.compile(r"^wem_curves (\S.*)$", re.MULTILINE)
PERF_PATTERN = re.compile(r"^wem_perf(?:_input|_attribution)? corpus=\S+ .*$", re.MULTILINE)


# ---------------------------------------------------------------------------
# Environment and process plumbing
# ---------------------------------------------------------------------------


def cargo_env() -> dict[str, str]:
    """The environment every cargo invocation gets.

    The lane rule is a prefixed ``PATH`` with the pinned stable toolchain
    first; `--all-targets` applies to the workspace-wide suite, not to a
    single `--test` build, which is what this script builds.
    """
    env = dict(os.environ)
    env["PATH"] = f"{RUSTUP_BIN}:{env.get('PATH', '')}"
    env["PYTHONDONTWRITEBYTECODE"] = "1"
    env.setdefault("PYTHONPATH", "src:reference")
    return env


def build_test_binary(cargo: str, test_name: str, parallel: bool) -> Path:
    """Build one release test target and return its executable path.

    ``parallel`` opts into the kernel's internal parallelism; the default build
    is scalar, so leaving it false measures the library's own configuration.
    """
    command = [
        cargo,
        "test",
        "--release",
        "-p",
        "wem-core",
        "--test",
        test_name,
        "--no-run",
        "--message-format=json",
    ]
    if parallel:
        command += ["--features", "parallel"]
    result = subprocess.run(
        command, cwd=CRATES, env=cargo_env(), capture_output=True, text=True
    )
    if result.returncode != 0:
        raise RuntimeError(f"building {test_name} failed:\n{result.stderr[-4000:]}")
    executable = None
    for line in result.stdout.splitlines():
        line = line.strip()
        if not line.startswith("{"):
            continue
        try:
            message = json.loads(line)
        except json.JSONDecodeError:
            continue
        if message.get("reason") != "compiler-artifact":
            continue
        if message.get("target", {}).get("name") == test_name and message.get("executable"):
            executable = message["executable"]
    if executable is None:
        raise RuntimeError(f"no executable reported for {test_name}")
    return Path(executable)


def machine_identity() -> dict[str, str]:
    """What the numbers came from; reported, never interpreted."""
    identity = {
        "platform": platform.platform(),
        "machine": platform.machine(),
        "python": f"{platform.python_version()} ({sys.executable})",
        "logical_cpus": str(os.cpu_count()),
    }
    for key, label in (
        ("machdep.cpu.brand_string", "cpu"),
        ("hw.ncpu", "hw_ncpu"),
        ("hw.memsize", "hw_memsize"),
    ):
        value = sysctl(key)
        if value:
            identity[label] = value
    rustc = RUSTUP_BIN / "rustc"
    if not rustc.exists():
        rustc = Path("rustc")
    result = subprocess.run([str(rustc), "-vV"], capture_output=True, text=True)
    if result.returncode == 0:
        for line in result.stdout.splitlines():
            if line.startswith("rustc "):
                identity["rustc"] = line.split(" ", 1)[1]
            elif line.startswith(("host:", "LLVM")):
                key, _, value = line.partition(":")
                identity[key.strip().lower()] = value.strip()
    return identity


def sysctl(key: str) -> str | None:
    try:
        result = subprocess.run(["sysctl", "-n", key], capture_output=True, text=True)
    except FileNotFoundError:
        return None
    return result.stdout.strip() if result.returncode == 0 else None


def busy_cpu_percent() -> float:
    """Whole-machine busy CPU, summed over processes (`ps`), in percent."""
    result = subprocess.run(["ps", "-Ao", "%cpu="], capture_output=True, text=True)
    if result.returncode != 0:
        return float("nan")
    total = 0.0
    for token in result.stdout.split():
        try:
            total += float(token)
        except ValueError:
            continue
    return total


def load_snapshot() -> str:
    one, five, fifteen = os.getloadavg()
    return f"{one:.2f}/{five:.2f}/{fifteen:.2f}"


# ---------------------------------------------------------------------------
# Running one point
# ---------------------------------------------------------------------------


@dataclass
class Point:
    """One raw measurement of one (geometry, path, duration, rep)."""

    geometry: str
    path: str
    seconds: int
    rep: int
    fields: dict[str, object]
    load_before: str
    load_after: str
    started: float

    @property
    def key(self) -> tuple[str, str, int]:
        return (self.geometry, self.path, self.seconds)

    def value(self, name: str, default: float = float("nan")) -> float:
        raw = self.fields.get(name, default)
        return float(raw) if isinstance(raw, (int, float)) else default


def run_point(
    binary: Path,
    inputs: Path,
    geometry: str,
    selection: str,
    path: str,
    seconds: int,
    rep: int,
    extra_env: dict[str, str] | None = None,
) -> Point:
    wav = inputs / f"{geometry}-{seconds}s.wav"
    if not wav.is_file():
        raise RuntimeError(f"missing measurement input {wav}")
    env = cargo_env()
    env.update(
        {
            "WEM_CURVES_WAV": str(wav),
            "WEM_CURVES_SELECTION": selection,
            "WEM_CURVES_GEOMETRY": geometry,
            "WEM_CURVES_PATH": path,
            "WEM_CURVES_SECONDS": str(seconds),
            "WEM_CURVES_REP": str(rep),
        }
    )
    if extra_env:
        env.update(extra_env)
    command = [str(binary), "--ignored", "--exact", POINT_TEST, "--nocapture"]
    load_before = load_snapshot()
    started = time.time()
    result = subprocess.run(command, env=env, capture_output=True, text=True)
    load_after = load_snapshot()
    if result.returncode != 0:
        raise RuntimeError(
            f"point {geometry}/{path}/{seconds}s rep={rep} failed "
            f"(exit {result.returncode}):\n{result.stderr[-2000:]}"
        )
    match = RESULT_PATTERN.search(result.stdout)
    if match is None:
        raise RuntimeError(
            f"point {geometry}/{path}/{seconds}s rep={rep} printed no report line:\n"
            f"{result.stdout[-2000:]}"
        )
    return Point(
        geometry=geometry,
        path=path,
        seconds=seconds,
        rep=rep,
        fields=parse_fields(match.group(1)),
        load_before=load_before,
        load_after=load_after,
        started=started,
    )


def parse_fields(line: str) -> dict[str, object]:
    """`key=value …` into a typed dict; unknown shapes stay strings."""
    fields: dict[str, object] = {}
    for token in line.split():
        key, _, value = token.partition("=")
        if not _:
            continue
        if value in ("true", "false"):
            fields[key] = value == "true"
            continue
        try:
            fields[key] = int(value)
            continue
        except ValueError:
            pass
        try:
            fields[key] = float(value)
        except ValueError:
            fields[key] = value
    return fields


# ---------------------------------------------------------------------------
# Statistics
# ---------------------------------------------------------------------------


def percentile(samples: list[float], fraction: float) -> float:
    """Percentile by linear interpolation between order statistics."""
    ordered = sorted(samples)
    if len(ordered) == 1:
        return ordered[0]
    position = fraction * (len(ordered) - 1)
    lower = math.floor(position)
    upper = math.ceil(position)
    if lower == upper:
        return ordered[lower]
    weight = position - lower
    return ordered[lower] * (1.0 - weight) + ordered[upper] * weight


@dataclass
class Series:
    """The samples of one field over the repetitions of one point."""

    samples: list[float] = field(default_factory=list)

    def add(self, value: float) -> None:
        self.samples.append(value)

    @property
    def n(self) -> int:
        return len(self.samples)

    @property
    def minimum(self) -> float:
        return min(self.samples)

    @property
    def median(self) -> float:
        return statistics.median(self.samples)

    @property
    def maximum(self) -> float:
        return max(self.samples)

    @property
    def spread(self) -> float:
        return self.maximum - self.minimum

    def summary(self) -> dict[str, float]:
        median = self.median
        return {
            "n": float(self.n),
            "min": self.minimum,
            "median": median,
            "max": self.maximum,
            "spread": self.spread,
            "spread_pct_of_median": (self.spread / median * 100.0) if median else float("nan"),
        }


def fmt(value: float, unit: str = "", width: int = 9) -> str:
    if isinstance(value, float) and math.isnan(value):
        return f"{'--':>{width}}"
    if unit == "mb":
        return f"{value / 1e6:>{width}.1f}"
    if unit == "s":
        return f"{value / 1e3:>{width}.3f}"
    if unit == "x":
        return f"{value:>{width}.2f}"
    return f"{value:>{width}.3f}"


# ---------------------------------------------------------------------------
# The matrix
# ---------------------------------------------------------------------------


def reps_for(seconds: int, reps: int, reps_long: int) -> int:
    return reps_long if seconds >= LONG_SECONDS else reps


def run_matrix(
    binary: Path,
    inputs: Path,
    durations: list[int],
    geometries: list[tuple[str, str]],
    paths: list[str],
    reps: int,
    reps_long: int,
    contamination_factor: float,
    max_extra_rounds: int,
    reference_durations: list[int],
) -> tuple[list[Point], list[dict[str, object]]]:
    """Paired, order-alternated rounds, then a targeted re-run of outliers."""
    points: list[Point] = []
    rounds: list[dict[str, object]] = []
    paired = [path for path in PAIRED_PATHS if path in paths]
    extra = [path for path in paths if path not in PAIRED_PATHS]
    highest = max(reps, reps_long)

    for rep in range(highest):
        order = list(paired) if rep % 2 == 0 else list(reversed(paired))
        order += extra
        busy = busy_cpu_percent()
        round_started = time.time()
        ran = 0
        for geometry, selection in geometries:
            if not paired:
                continue
            for seconds in durations:
                if rep >= reps_for(seconds, reps, reps_long):
                    continue
                for path in order:
                    points.append(
                        run_point(binary, inputs, geometry, selection, path, seconds, rep)
                    )
                    ran += 1
        rounds.append(
            {
                "rep": rep,
                "order": order,
                "busy_cpu_percent": busy,
                "wall_s": time.time() - round_started,
                "points": ran,
                "load": load_snapshot(),
            }
        )
        print(
            f"round {rep}: order={' '.join(order)} busy_cpu={busy:.0f}% "
            f"points={ran} wall={time.time() - round_started:.1f}s load={load_snapshot()}",
            file=sys.stderr,
            flush=True,
        )

    # Reference paths: one sample per (geometry, duration); they are baselines,
    # not a series to compare across rounds. The phase workers replay the whole
    # analysis pipeline, so they are run at the reference durations only.
    for geometry, selection in geometries:
        for seconds in durations:
            if seconds not in reference_durations:
                continue
            for path in REFERENCE_PATHS:
                if path in paths:
                    points.append(
                        run_point(binary, inputs, geometry, selection, path, seconds, 0)
                    )
                    points.append(
                        run_point(binary, inputs, geometry, selection, path, seconds, 1)
                    )

    # Contamination re-runs: a point whose slowest wall sample is many times its
    # median is re-measured rather than averaged in.
    contaminated = contaminated_keys(points, contamination_factor)
    for rep in range(reps, reps + max_extra_rounds):
        if not contaminated:
            break
        print(
            f"re-run round {rep} for {len(contaminated)} contaminated point(s): "
            f"{sorted(contaminated)}",
            file=sys.stderr,
            flush=True,
        )
        rerun_order = list(paired) if rep % 2 == 0 else list(reversed(paired))
        rerun_order += extra
        for geometry, selection in geometries:
            for seconds in durations:
                for path in rerun_order:
                    key = (geometry, path, seconds)
                    if key not in contaminated:
                        continue
                    points.append(
                        run_point(binary, inputs, geometry, selection, path, seconds, rep)
                    )
        contaminated = contaminated_keys(points, contamination_factor)

    return points, rounds


def contaminated_keys(
    points: list[Point], factor: float
) -> set[tuple[str, str, int]]:
    grouped: dict[tuple[str, str, int], list[float]] = {}
    for point in points:
        grouped.setdefault(point.key, []).append(point.value("wall_ms"))
    keys = set()
    for key, walls in grouped.items():
        if len(walls) < 3:
            continue
        median = statistics.median(walls)
        if median > 0 and max(walls) > factor * median:
            keys.add(key)
    return keys


# ---------------------------------------------------------------------------
# The stage split
# ---------------------------------------------------------------------------


def run_stage_split(
    stage_binary: Path,
    inputs: Path,
    geometry: str,
    selection: str,
    seconds: int,
    runs: int,
) -> dict[str, object]:
    wav = inputs / f"{geometry}-{seconds}s.wav"
    env = cargo_env()
    env.update(
        {
            "WEM_CURVES_WAV": str(wav),
            "WEM_CURVES_SELECTION": selection,
            "WEM_CURVES_LABEL": f"{geometry}-{seconds}s",
            "WEM_CURVES_RUNS": str(runs),
            "WEM_CURVES_ATTRIBUTION": "1",
        }
    )
    command = [str(stage_binary), "--ignored", "--exact", STAGE_TEST, "--nocapture"]
    result = subprocess.run(command, env=env, capture_output=True, text=True)
    if result.returncode != 0:
        raise RuntimeError(
            f"stage split {geometry}/{seconds}s failed (exit {result.returncode}):\n"
            f"{result.stderr[-2000:]}"
        )
    lines = [line for line in PERF_PATTERN.findall(result.stdout)]
    staged = [
        parse_fields(line.split(" ", 2)[2])
        for line in lines
        if line.startswith("wem_perf corpus")
    ]
    attribution = [
        parse_fields(line.split(" ", 2)[2])
        for line in lines
        if line.startswith("wem_perf_attribution")
    ]
    identity = [
        parse_fields(line.split(" ", 2)[2]) for line in lines if line.startswith("wem_perf_input")
    ]
    if not staged:
        raise RuntimeError(
            f"stage split {geometry}/{seconds}s printed no staged run:\n{result.stdout[-2000:]}"
        )
    if not all(sample.get("staged_matches_encode_pcm") is True for sample in staged):
        raise RuntimeError(
            f"stage split {geometry}/{seconds}s: the staged replay diverged from encode_pcm"
        )
    return {
        "geometry": geometry,
        "seconds": seconds,
        "runs": runs,
        "identity": identity[0] if identity else {},
        "staged": staged,
        "attribution": attribution[0] if attribution else {},
        "load": load_snapshot(),
    }


# ---------------------------------------------------------------------------
# Reporting
# ---------------------------------------------------------------------------


def series_by_key(points: list[Point], name: str) -> dict[tuple[str, str, int], Series]:
    grouped: dict[tuple[str, str, int], Series] = {}
    for point in points:
        grouped.setdefault(point.key, Series()).add(point.value(name))
    return grouped


def report_memory(points: list[Point], durations: list[int], geometries: list[tuple[str, str]]) -> list[str]:
    lines = ["", "== (b) peak RSS by duration (min / median / max, MB) ==", ""]
    header = f"{'geometry':<10} {'path':<10} " + " ".join(f"{d:>7}s" for d in durations)
    lines.append(header)
    lines.append("-" * len(header))
    rss = series_by_key(points, "ru_maxrss_bytes")
    for geometry, _ in geometries:
        for path in (
            "noop",
            "load",
            "rows",
            "condition",
            "detector",
            "source",
            "windows",
            "batch",
            "stream",
        ):
            cells = []
            for seconds in durations:
                series = rss.get((geometry, path, seconds))
                if series is None:
                    cells.append(f"{'--':>8}")
                    continue
                cells.append(f"{series.minimum / 1e6:>5.1f}/{series.median / 1e6:>5.1f}")
            lines.append(f"{geometry:<10} {path:<10} " + " ".join(f"{cell:>8}" for cell in cells))
    lines.append("")
    lines.append("cell = min/median MB. `noop` is the process baseline, `load` the caller's")
    lines.append("PCM, `rows`/`condition`/`windows` the one-shot pipeline's own statements,")
    lines.append("`batch` = encode_pcm, `stream` = StreamSession from a streaming reader.")
    return lines


def report_shape(
    points: list[Point], durations: list[int], geometries: list[tuple[str, str]]
) -> list[str]:
    """Name the shape of each curve and locate a knee if there is one.

    The shape is derived, not eyeballed. Every point carries a least-squares
    line; a curve whose largest residual to its own line is a small fraction of
    its largest value *is* that line, so it is called `linear`. A knee is a
    systematic departure from the line (residual above `KNEE_RESIDUAL_SHARE`),
    not the +/-25% jitter a loaded machine puts on individual interval slopes --
    the interval slopes are printed next to the verdict so that distinction is
    visible rather than asserted.
    """
    lines = ["", "== (b) curve shape: slopes between durations and a linear fit ==", ""]
    knee_residual_share = 0.15
    flat_share = 0.05
    rss = series_by_key(points, "ru_maxrss_bytes")
    total = series_by_key(points, "total_ms")
    for label, series_map, scale, unit in (
        ("peak RSS", rss, 1e6, "MB"),
        ("encode-side wall", total, 1e3, "s"),
    ):
        lines.append(f"-- {label} ({unit}) --")
        lines.append(
            f"{'geometry':<10} {'path':<7} {'shape':<8} {'fit slope':>12} "
            f"{'intercept':>10} {'resid%':>7} {'slope drift':>11}  slopes between durations"
        )
        for geometry, _ in geometries:
            for path in ("batch", "stream"):
                values = []
                for seconds in durations:
                    series = series_map.get((geometry, path, seconds))
                    if series is None:
                        continue
                    values.append((seconds, series.minimum / scale))
                if len(values) < 2:
                    continue
                slopes = [
                    (values[index + 1][1] - values[index][1])
                    / (values[index + 1][0] - values[index][0])
                    for index in range(len(values) - 1)
                ]
                slope, intercept, residual = linear_fit(
                    [entry[0] for entry in values], [entry[1] for entry in values]
                )
                largest = max(entry[1] for entry in values)
                span = abs(slope) * values[-1][0]
                if span < flat_share * largest:
                    shape = "flat"
                elif residual <= knee_residual_share * largest:
                    shape = "linear"
                else:
                    shape = "knee"
                magnitudes = [abs(value) for value in slopes if value]
                drift = (max(magnitudes) / min(magnitudes)) if magnitudes else float("nan")
                lines.append(
                    f"{geometry:<10} {path:<7} {shape:<8} {slope:>12.4f} "
                    f"{intercept:>10.3f} {100.0 * residual / largest:>6.1f}% {drift:>10.2f}x  "
                    + " ".join(f"{value:+.4f}" for value in slopes)
                )
        lines.append("")
    lines.append("fit slope = least squares over every duration, in unit per audio-second;")
    lines.append("intercept = the fitted duration-0 value; resid% = the largest residual to")
    lines.append("that line as a share of the curve's largest value (so `linear` here means")
    lines.append("the line is the curve, not that the numbers were rounded); slope drift =")
    lines.append("largest interval slope / smallest, which shows the machine's jitter.")
    return lines


def linear_fit(xs: list[float], ys: list[float]) -> tuple[float, float, float]:
    """Least-squares slope/intercept and the largest absolute residual."""
    mean_x = statistics.fmean(xs)
    mean_y = statistics.fmean(ys)
    denominator = sum((x - mean_x) ** 2 for x in xs)
    slope = (
        sum((x - mean_x) * (y - mean_y) for x, y in zip(xs, ys)) / denominator
        if denominator
        else 0.0
    )
    intercept = mean_y - slope * mean_x
    residual = max(abs(y - (intercept + slope * x)) for x, y in zip(xs, ys))
    return slope, intercept, residual


def report_decomposition(
    points: list[Point], durations: list[int], geometries: list[tuple[str, str]]
) -> list[str]:
    """Where the one-shot peak comes from, per duration, in MB."""
    lines = ["", "== (b) decomposition of the one-shot peak (min RSS, MB) ==", ""]
    rss = series_by_key(points, "ru_maxrss_bytes")
    lines.append(
        f"{'geometry':<10} {'dur':>5} {'baseline':>9} {'+PCM':>8} {'+rows':>8} "
        f"{'+condition':>11} {'+detector':>10} {'+source':>8} {'+windows':>9} "
        f"{'=batch':>8} {'stream':>8} {'batch-D':>8}"
    )

    def value(key: tuple[str, str, int]) -> float | None:
        series = rss.get(key)
        return series.minimum / 1e6 if series else None

    cells_paths = (
        "noop",
        "load",
        "rows",
        "condition",
        "detector",
        "source",
        "windows",
        "batch",
    )
    for geometry, _ in geometries:
        for seconds in durations:
            cells = [value((geometry, path, seconds)) for path in cells_paths]
            stream = value((geometry, "stream", seconds))
            if all(cell is None for cell in cells):
                continue
            base = cells[0] or 0.0
            batch = cells[7]
            lines.append(
                f"{geometry:<10} {seconds:>4}s "
                + " ".join(f"{cell:>8.1f}" if cell is not None else f"{'--':>8}" for cell in cells)
                + (f" {stream:>8.1f}" if stream is not None else f" {'--':>8}")
                + (f" {batch - base:>8.1f}" if batch is not None else f" {'--':>8}")
            )
    lines.append("")
    lines.append("Each column is a phase worker holding what it built; `+detector` and")
    lines.append("`+source` are the two whole-input terms `selected_window_source` added to")
    lines.append("the one-shot path, and `+windows` is `+source` plus the materialized frame")
    lines.append("sequence. The columns are statements of `Encoder::encode_pcm`, not additive")
    lines.append("deltas. `batch-D` = batch minus the process baseline: what one one-shot")
    lines.append("encode actually costs resident.")
    return lines


def run_output_split(
    binary: Path,
    inputs: Path,
    points: list[Point],
    geometries: list[tuple[str, str]],
    output_durations: list[int],
    samples: int,
) -> list[dict[str, object]]:
    """Decompose `finish` into the session's retention and the builder's copies.

    `finish` is a single call and `ru_maxrss` is a high-water mark that a later
    `drop` cannot lower, so the two halves cannot be sequenced apart inside one
    child. The split therefore comes from a third child: `stream` and
    `stream_push` (already measured) bound everything `finish` adds, and an
    `assembly` child runs *only* `build_vorbis_wem` over a synthetic packet set
    carrying the measured packet count and the measured output size. The two
    routes to the same quantity are reported next to each other as the check
    that the synthetic set stands in for the real one.
    """
    grouped: dict[tuple[str, str, int], list[Point]] = {}
    for point in points:
        grouped.setdefault(point.key, []).append(point)
    splits: list[dict[str, object]] = []
    for geometry, selection in geometries:
        for seconds in output_durations:
            stream_points = grouped.get((geometry, "stream", seconds), [])
            push_points = grouped.get((geometry, "stream_push", seconds), [])
            if not stream_points or not push_points:
                print(
                    f"output split: {geometry} {seconds}s skipped "
                    f"(needs stream and stream_push in --paths)",
                    file=sys.stderr,
                    flush=True,
                )
                continue
            output = int(stream_points[0].value("wem_bytes"))
            packets = int(stream_points[0].value("audio_packets"))
            if output <= 0 or packets <= 0:
                continue
            extra = {
                "WEM_CURVES_PACKETS": str(packets),
                "WEM_CURVES_PACKET_BYTES": str(max(1, output // packets)),
            }
            print(f"output split: {geometry} {seconds}s", file=sys.stderr, flush=True)
            runs = [
                run_point(binary, inputs, geometry, selection, "noop", seconds, rep)
                for rep in range(samples)
            ]
            runs += [
                run_point(
                    binary, inputs, geometry, selection, "assembly", seconds, rep, extra
                )
                for rep in range(samples)
            ]
            splits.append(
                {
                    "geometry": geometry,
                    "selection": selection,
                    "seconds": seconds,
                    "output_bytes": output,
                    "output_bytes_seen": sorted(
                        {int(point.value("wem_bytes")) for point in stream_points}
                    ),
                    "audio_packets": packets,
                    "packet_bytes": int(extra["WEM_CURVES_PACKET_BYTES"]),
                    "stream_rss_bytes": min(
                        point.value("ru_maxrss_bytes") for point in stream_points
                    ),
                    "stream_push_rss_bytes": min(
                        point.value("ru_maxrss_bytes") for point in push_points
                    ),
                    "runs": [point_to_json(point) for point in runs],
                }
            )
    return splits


def report_output(splits: list[dict[str, object]]) -> list[str]:
    """The output side: what `finish` adds to the streaming peak, and where."""
    lines = ["", "== (e) the output side: what `finish` adds, and where it goes ==", ""]
    if not splits:
        lines.append("(not measured: `--paths` needs stream and stream_push, and the")
        lines.append("durations need to be in `--output-durations`)")
        return lines
    lines.append(
        f"{'geometry':<10} {'dur':>5} {'output':>8} {'push':>7} {'stream':>7} "
        f"{'finish':>7} {'fin/out':>8} {'assembly':>8} {'set':>7} {'builder':>8} "
        f"{'retain':>7} {'model':>7} {'n':>3}"
    )
    for split in splits:
        runs = [point_from_json(entry) for entry in split["runs"]]
        baseline = min(
            (point.value("ru_maxrss_bytes") for point in runs if point.path == "noop"),
            default=float("nan"),
        )
        assembly = [point for point in runs if point.path == "assembly"]
        if not assembly or math.isnan(baseline):
            continue
        base = baseline / 1e6
        output = float(split["output_bytes"]) / 1e6
        push = float(split["stream_push_rss_bytes"]) / 1e6 - base
        stream = float(split["stream_rss_bytes"]) / 1e6 - base
        finish = stream - push
        set_bytes = max(point.value("set_bytes") for point in assembly) / 1e6
        built = min(point.value("ru_maxrss_bytes") for point in assembly) / 1e6 - base
        builder = built - set_bytes
        retain = finish - builder
        lines.append(
            f"{split['geometry']:<10} {split['seconds']:>4}s {output:>8.1f} "
            f"{push:>7.1f} {stream:>7.1f} {finish:>7.1f} {finish / output:>7.2f}x "
            f"{built:>8.1f} {set_bytes:>7.1f} {builder:>8.1f} {retain:>7.1f} "
            f"{built / finish if finish else float('nan'):>6.2f}x {len(assembly):>3}"
        )
    lines.append("")
    lines.append("Every cell is MB of peak RSS above the process baseline (`noop`), read")
    lines.append("from a reaped child's `ru_maxrss`, one fresh child per column. `output` is")
    lines.append("the WEM the encode returns; `push` the session after every chunk and before")
    lines.append("`finish`; `finish` what `finish` adds on top of it; `assembly` a child that")
    lines.append("runs only `build_vorbis_wem` over a synthetic packet set carrying the")
    lines.append("measured packet count and output size; `set` that set; `builder` =")
    lines.append("`assembly - set`, the copies the container builder makes on top of its own")
    lines.append("input; `retain` = `finish - builder`, the copies the session makes. `model`")
    lines.append("= `assembly / finish`, the same quantity reached by the other route, and the")
    lines.append("check that the synthetic set stands in for the real one.")
    return lines


def report_time(
    points: list[Point], durations: list[int], geometries: list[tuple[str, str]], paths: list[str]
) -> list[str]:
    lines = ["", "== (c) wall time, CPU and the multiple of real time ==", ""]
    wall = series_by_key(points, "wall_ms")
    user = series_by_key(points, "user_ms")
    sysc = series_by_key(points, "sys_ms")
    total = series_by_key(points, "total_ms")
    encode_paths = [path for path in paths if path in PAIRED_PATHS]
    for geometry, _ in geometries:
        lines.append(f"-- {geometry} --")
        lines.append(
            f"{'path':<7} {'dur':>6} {'wall min':>9} {'wall med':>9} {'wall max':>9} "
            f"{'user med':>9} {'sys med':>9} {'xRT(min)':>9} {'xRT(med)':>9} "
            f"{'par med':>8} {'n':>3}"
        )
        for path in encode_paths:
            for seconds in durations:
                key = (geometry, path, seconds)
                if key not in total:
                    continue
                ratios = [
                    seconds / (value / 1e3) for value in total[key].samples if value > 0
                ]
                cpu = [u + s for u, s in zip(user[key].samples, sysc[key].samples)]
                parallelism = [c / w for c, w in zip(cpu, wall[key].samples) if w > 0]
                lines.append(
                    f"{path:<7} {seconds:>5}s {fmt(wall[key].minimum, 's'):>9} "
                    f"{fmt(wall[key].median, 's'):>9} {fmt(wall[key].maximum, 's'):>9} "
                    f"{fmt(user[key].median, 's'):>9} {fmt(sysc[key].median, 's'):>9} "
                    f"{fmt(max(ratios), 'x'):>9} {fmt(statistics.median(ratios), 'x'):>9} "
                    f"{fmt(statistics.median(parallelism), 'x'):>8} {total[key].n:>3}"
                )
        lines.append("")
    lines.append("wall = whole child process (includes ~3-4 ms start-up); user/sys from")
    lines.append("wait4 rusage over all threads; xRT = audio seconds / encode-side wall")
    lines.append("(`total_ms`, measured inside the process); par = (user+sys)/wall, i.e.")
    lines.append("how many cores the process occupied. xRT(min) is the least-contended")
    lines.append("sample, xRT(med) the median one.")
    return lines


def report_spread(
    points: list[Point], geometries: list[tuple[str, str]], paths: list[str]
) -> list[str]:
    lines = [
        "",
        "== (c) spread of the encode-side wall (`total_ms`) and of the peak RSS ==",
        "",
    ]
    total = series_by_key(points, "total_ms")
    rss = series_by_key(points, "ru_maxrss_bytes")
    worst: list[tuple[float, tuple[str, str, int], Series]] = []
    for key, series in total.items():
        if series.n >= 2 and series.median > 0:
            worst.append((series.summary()["spread_pct_of_median"], key, series))
    worst.sort(reverse=True)
    lines.append(
        f"{'geometry':<10} {'path':<7} {'dur':>6} {'n':>3} {'min ms':>10} {'med ms':>10} "
        f"{'max ms':>10} {'wall spread%':>12} {'rss spread%':>11}"
    )
    for spread, key, series in worst[:15]:
        geometry, path, seconds = key
        rss_spread = rss[key].summary()["spread_pct_of_median"] if key in rss else float("nan")
        lines.append(
            f"{geometry:<10} {path:<7} {seconds:>5}s {series.n:>3} "
            f"{fmt(series.minimum):>10} {fmt(series.median):>10} {fmt(series.maximum):>10} "
            f"{spread:>11.1f}% {rss_spread:>10.1f}%"
        )
    lines.append("")
    lines.append("The wall spread is the machine speaking, not the encoder: the RSS spread on")
    lines.append("the same points is the control that shows which of the two is stable.")
    return lines


def report_stages(splits: list[dict[str, object]]) -> list[str]:
    lines = [
        "",
        "== (d) per-stage split (median of the staged runs, shares of staged total) ==",
        "",
    ]
    stage_names = (
        ("pcm_decode_ms", "pcm decode"),
        ("condition_ms", "conditioner"),
        ("plan_and_windows_ms", "sched+windows"),
        ("analyze_short_ms", "analyze short"),
        ("analyze_long_ms", "analyze long"),
        ("pack_ms", "vorbis pack"),
        ("container_ms", "container"),
    )
    for split in splits:
        staged = split["staged"]
        medians: dict[str, float] = {}
        for name, _ in stage_names:
            medians[name] = statistics.median([float(sample.get(name, 0.0)) for sample in staged])
        total = statistics.median([float(sample.get("staged_total_ms", 0.0)) for sample in staged])
        encode = statistics.median([float(sample.get("encode_pcm_ms", 0.0)) for sample in staged])
        lines.append(
            f"-- {split['geometry']} {split['seconds']}s "
            f"({split['identity'].get('channels')}ch/{split['identity'].get('sample_rate')}, "
            f"short={staged[0].get('short_frames')} long={staged[0].get('long_frames')} frames, "
            f"n={len(staged)}) --"
        )
        for name, label in stage_names:
            share = (medians[name] / total * 100.0) if total else float("nan")
            lines.append(f"   {label:<15} {medians[name]:>10.3f} ms  {share:>6.1f}%")
        totals = [float(sample.get("staged_total_ms", 0.0)) for sample in staged]
        lines.append(f"   {'staged total':<15} {total:>10.3f} ms  {'100.0':>6}%")
        lines.append(
            f"   {'encode_pcm':<15} {encode:>10.3f} ms   "
            f"(staged-total min/max {min(totals):.3f}/{max(totals):.3f} ms)"
        )
        if split["attribution"]:
            attribution = split["attribution"]
            lines.append(
                "   attribution (single-threaded re-runs, not additive): "
                f"fft_log_curve={float(attribution.get('fft_log_curve_ms', 0)):.3f} ms "
                f"({float(attribution.get('fft_calls', 0)):.0f} calls), "
                f"mdct={float(attribution.get('mdct_ms', 0)):.3f} ms, "
                f"twiddle_model={float(attribution.get('fft_twiddle_recurrence_cost_model_ms', 0)):.3f} ms, "
                f"window_rebuild={float(attribution.get('window_rebuild_ms', 0)):.3f} ms, "
                f"transient={float(attribution.get('transient_ms', 0)):.3f} ms"
            )
        lines.append("")
    return lines


def report_load(points: list[Point], rounds: list[dict[str, object]]) -> list[str]:
    ones = []
    for point in points:
        try:
            ones.append(float(point.load_before.split("/")[0]))
            ones.append(float(point.load_after.split("/")[0]))
        except (ValueError, IndexError):
            continue
    lines = ["", "== machine load during the run ==", ""]
    if ones:
        lines.append(
            f"1-minute load average across point boundaries: "
            f"min={min(ones):.2f} median={statistics.median(ones):.2f} max={max(ones):.2f} "
            f"(16 logical cores)"
        )
    for entry in rounds:
        lines.append(
            f"round {entry['rep']}: busy_cpu={entry['busy_cpu_percent']:.0f}% "
            f"({float(entry['busy_cpu_percent']) / 16.0:.2f} cores) "
            f"wall={entry['wall_s']:.1f}s load={entry['load']}"
        )
    return lines


# ---------------------------------------------------------------------------
# Entry point
# ---------------------------------------------------------------------------


def main(argv: list[str] | None = None) -> int:
    parser = argparse.ArgumentParser(
        description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter
    )
    parser.add_argument("--cargo", default=str(RUSTUP_BIN / "cargo"), help="cargo executable")
    parser.add_argument("--inputs", type=Path, default=DEFAULT_INPUTS)
    parser.add_argument("--scratch", type=Path, default=DEFAULT_SCRATCH)
    parser.add_argument(
        "--durations",
        default=",".join(str(value) for value in DEFAULT_DURATIONS),
        help="comma-separated durations in seconds",
    )
    parser.add_argument(
        "--geometries",
        default=",".join(f"{label}:{selection}" for label, selection in DEFAULT_GEOMETRIES),
        help="comma-separated <label>:<selection> pairs (selection: fixture|two_channel)",
    )
    parser.add_argument(
        "--paths",
        default=",".join(DEFAULT_PATHS),
        help=(
            "comma-separated paths to measure; batch,stream are the paired encode "
            "paths, noop/load/rows/condition/windows the once-per-duration references"
        ),
    )
    parser.add_argument(
        "--reference-durations",
        default="10,300",
        help=(
            "durations at which the reference paths (noop/load/rows/condition/"
            "windows) are measured; the phase workers replay the whole pipeline"
        ),
    )
    parser.add_argument("--reps", type=int, default=3, help="rounds for short durations")
    parser.add_argument(
        "--reps-long",
        type=int,
        default=2,
        help=f"rounds for durations >= {LONG_SECONDS} s",
    )
    parser.add_argument(
        "--contamination-factor",
        type=float,
        default=1.5,
        help="re-run a point whose slowest wall sample exceeds this x its median",
    )
    parser.add_argument("--max-extra-rounds", type=int, default=2)
    parser.add_argument("--skip-generate", action="store_true")
    parser.add_argument("--skip-stages", action="store_true")
    parser.add_argument(
        "--stage-durations",
        default=",".join(str(value) for value in DEFAULT_STAGE_DURATIONS),
        help="durations for the per-stage split (needs the parallel build)",
    )
    parser.add_argument("--stage-runs", type=int, default=3)
    parser.add_argument(
        "--output-durations",
        default=",".join(str(value) for value in DEFAULT_OUTPUT_DURATIONS),
        help=(
            "durations at which `finish` is decomposed into the session's retention "
            "and the container builder's copies (needs stream and stream_push in "
            "--paths); empty to skip"
        ),
    )
    parser.add_argument(
        "--output-samples", type=int, default=2, help="children per output-split column"
    )
    parser.add_argument(
        "--parallel",
        action="store_true",
        help="build the opt-in `parallel` configuration (the default build is scalar)",
    )
    parser.add_argument("--tag", default=None, help="output label (default: parallel|scalar)")
    parser.add_argument("--json", type=Path, default=None, help="result JSON path")
    parser.add_argument(
        "--report-only",
        action="store_true",
        help="skip measurement; re-render the report from --json",
    )
    args = parser.parse_args(argv)

    durations = [int(value) for value in args.durations.split(",") if value]
    geometries = []
    for entry in args.geometries.split(","):
        if not entry:
            continue
        label, _, selection = entry.partition(":")
        geometries.append((label, selection or "fixture"))
    paths = [value for value in args.paths.split(",") if value]
    reference_durations = [
        int(value) for value in args.reference_durations.split(",") if value
    ]
    output_durations = [int(value) for value in args.output_durations.split(",") if value]
    tag = args.tag or ("parallel" if args.parallel else "scalar")
    json_path = args.json or (args.scratch / f"duration-curves-{tag}.json")

    if args.report_only:
        payload = json.loads(json_path.read_text(encoding="utf-8"))
        points = [point_from_json(entry) for entry in payload["points"]]
        lines = []
        lines += report_load(points, payload.get("rounds", []))
        lines += report_memory(points, payload["durations"], geometries_from(payload))
        lines += report_shape(points, payload["durations"], geometries_from(payload))
        lines += report_decomposition(points, payload["durations"], geometries_from(payload))
        lines += report_output(payload.get("output_splits", []))
        lines += report_time(points, payload["durations"], geometries_from(payload), payload["paths"])
        lines += report_spread(points, geometries_from(payload), payload["paths"])
        lines += report_stages(payload.get("stage_splits", []))
        print("\n".join(lines))
        return 0

    if not args.skip_generate:
        command = [
            sys.executable,
            str(REPO / "scripts" / "generate_duration_curve_inputs.py"),
            "--output-dir",
            str(args.inputs),
            "--durations",
            ",".join(str(value) for value in durations),
            "--geometries",
            ",".join(label for label, _ in geometries),
        ]
        print(f"generating inputs: {' '.join(command)}", file=sys.stderr, flush=True)
        if subprocess.run(command, env=cargo_env()).returncode != 0:
            raise SystemExit(2)

    print("building the release measurement harnesses", file=sys.stderr, flush=True)
    binary = build_test_binary(args.cargo, "duration_curves", args.parallel)
    stage_binary = None
    if not args.skip_stages and args.parallel:
        stage_binary = build_test_binary(args.cargo, "stage_timings", True)

    identity = machine_identity()
    print("machine identity:", file=sys.stderr, flush=True)
    for key, value in identity.items():
        print(f"  {key:<22}: {value}", file=sys.stderr)

    points, rounds = run_matrix(
        binary,
        args.inputs,
        durations,
        geometries,
        paths,
        args.reps,
        args.reps_long,
        args.contamination_factor,
        args.max_extra_rounds,
        reference_durations,
    )

    stage_splits: list[dict[str, object]] = []
    if stage_binary is not None:
        stage_durations = [int(value) for value in args.stage_durations.split(",") if value]
        for geometry, selection in geometries:
            for seconds in stage_durations:
                if seconds not in durations:
                    continue
                print(f"stage split: {geometry} {seconds}s", file=sys.stderr, flush=True)
                stage_splits.append(
                    run_stage_split(
                        stage_binary, args.inputs, geometry, selection, seconds, args.stage_runs
                    )
                )

    output_splits: list[dict[str, object]] = []
    if output_durations and args.parallel:
        output_splits = run_output_split(
            binary,
            args.inputs,
            points,
            geometries,
            output_durations,
            args.output_samples,
        )

    payload = {
        "tag": tag,
        "parallel": args.parallel,
        "identity": identity,
        "durations": durations,
        "geometries": [list(entry) for entry in geometries],
        "paths": paths,
        "reps": args.reps,
        "reps_long": args.reps_long,
        "reference_durations": reference_durations,
        "output_durations": output_durations,
        "rounds": rounds,
        "started": points[0].started if points else None,
        "finished": time.time(),
        "points": [point_to_json(point) for point in points],
        "stage_splits": stage_splits,
        "output_splits": output_splits,
    }
    json_path.parent.mkdir(parents=True, exist_ok=True)
    json_path.write_text(json.dumps(payload, indent=2, sort_keys=True) + "\n", encoding="utf-8")
    print(f"\nraw samples: {json_path}")

    lines = []
    lines += report_load(points, rounds)
    lines += report_memory(points, durations, geometries)
    lines += report_shape(points, durations, geometries)
    lines += report_decomposition(points, durations, geometries)
    lines += report_output(output_splits)
    lines += report_time(points, durations, geometries, paths)
    lines += report_spread(points, geometries, paths)
    lines += report_stages(stage_splits)
    print("\n".join(lines))
    return 0


def geometries_from(payload: dict[str, object]) -> list[tuple[str, str]]:
    return [(str(label), str(selection)) for label, selection in payload["geometries"]]  # type: ignore[misc]


def point_to_json(point: Point) -> dict[str, object]:
    return {
        "geometry": point.geometry,
        "path": point.path,
        "seconds": point.seconds,
        "rep": point.rep,
        "fields": point.fields,
        "load_before": point.load_before,
        "load_after": point.load_after,
        "started": point.started,
    }


def point_from_json(entry: dict[str, object]) -> Point:
    return Point(
        geometry=str(entry["geometry"]),
        path=str(entry["path"]),
        seconds=int(entry["seconds"]),
        rep=int(entry["rep"]),
        fields=dict(entry["fields"]),
        load_before=str(entry["load_before"]),
        load_after=str(entry["load_after"]),
        started=float(entry["started"]),
    )


if __name__ == "__main__":
    try:
        raise SystemExit(main())
    except (RuntimeError, KeyError) as error:
        print(f"measurement failed: {error}", file=sys.stderr)
        raise SystemExit(1) from error
