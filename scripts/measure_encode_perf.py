#!/usr/bin/env python3
"""Release-build encode measurement: report the numbers, judge nothing.

Measures the **release** build and prints what it observed:

  * the CLI's own stage timers (``wwise-wem --time``) for the reference
    fixture and for every ``*.wav`` in the two 2-channel corpora;
  * the kernel's per-stage split for the fixture, read from the
    ``stage_timings`` harness (``crates/wem-core/tests/stage_timings.rs``),
    which replays the encode through the public kernel API with
    ``std::time::Instant`` around each existing call — no shipping code
    carries instrumentation for it.

For every series it prints min / median / p95 / max and the spread, plus the
machine identity the numbers came from (a median encode time is meaningless
without knowing the CPU, the core count, the OS and the compiler) and the
machine's load average at the time (two readings taken at different loads are
not comparable, so the load is recorded next to the numbers rather than
remembered).

It has no thresholds and no recorded baseline: nothing here decides whether a
number is acceptable, and there is no ``--record``. The exit status is 0
whenever a measurement completed and non-zero only on an actual error (a
missing binary, a failed run, unparsable output, a broken harness) — a loaded
machine changes the numbers, never the exit status.

``--stages`` (default on) compiles and runs the release ``stage_timings``
test target, which needs a Rust toolchain; ``--no-stages`` measures only
through the CLI.

Ambient reads are deliberate and declared: the machine identity and the load
average are part of the result, and the clock is what is being measured.
Nothing else is read from the environment, and nothing is written anywhere.

Usage:
  cargo build --release -p wem-core        # or: make rust-bench
  python3 scripts/measure_encode_perf.py
  python3 scripts/measure_encode_perf.py --runs 15 --no-stages
  python3 scripts/measure_encode_perf.py --bin /path/to/wwise-wem

Exit codes: 0 measured, 1 measurement failed, 2 usage/environment error.
"""
from __future__ import annotations

import argparse
import math
import os
import platform
import re
import shutil
import statistics
import subprocess
import sys
from dataclasses import dataclass
from pathlib import Path

REPO = Path(__file__).resolve().parents[1]
FIXTURE = REPO / "tests" / "fixtures" / "input.wav"
DEFAULT_BIN = REPO / "crates" / "target" / "release" / "wwise-wem"
TWO_CHANNEL_DIRS = (
    REPO / "tests" / "data" / "2ch-reference",
    REPO / "tests" / "data" / "2ch-stress",
)
DEFAULT_RUNS = 9
CARGO_MANIFEST = REPO / "crates" / "Cargo.toml"
STAGE_TEST = "stage_timings"
# The one reporting test in that file this driver consumes. Selected by name
# (`--exact`): the file also carries the duration-curve driver
# (`stage_timings_duration_report`), which `scripts/measure_duration_curves.py`
# runs with its own environment and which fails without it, so running every
# ignored test in the file is not what this measurement means.
STAGE_REPORT = "stage_timings_report"

CLI_STAGES = ("wav_load", "profile_assembly", "encode", "output_write")
CLI_STAGE_PATTERN = re.compile(
    r"stages: wav_load ([\d.]+)ms \| profile_assembly ([\d.]+)ms \| "
    r"encode ([\d.]+)ms \| output_write ([\d.]+)ms"
)
# `wem_perf <key>=<value> …` / `wem_perf_attribution <key>=<value> …`
HARNESS_LINE_PATTERN = re.compile(r"^(wem_perf(?:_attribution)?) (\S+) (\S+)((?: \S+)*)$")
HARNESS_FIELD_PATTERN = re.compile(r"(\w+)=(?:([\d.]+)|(\w+))")


# ---------------------------------------------------------------------------
# Statistics
# ---------------------------------------------------------------------------


def percentile(samples: list[float], fraction: float) -> float:
    """Percentile by linear interpolation between order statistics.

    The numpy default method, spelled out because the standard library has
    no percentile: ``sorted[s]`` interpolated between the two neighbouring
    order statistics. An empty series has no percentile and is not asked for
    one.
    """
    if not samples:
        raise ValueError("percentile of an empty series")
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
    """One named series of measurements, in milliseconds.

    ``load`` is the 1-minute load average before and after the runs that
    produced the samples; both are ``None`` on a platform without one.
    """

    label: str
    unit: str
    samples: list[float]
    load: tuple[float | None, float | None] | None = None

    def report(self) -> str:
        values = self.samples
        median = statistics.median(values)
        spread = max(values) - min(values)
        spread_share = (spread / median * 100.0) if median else float("nan")
        line = (
            f"  {self.label:<34} n={len(values):<3} "
            f"min={min(values):9.3f}  median={median:9.3f}  "
            f"p95={percentile(values, 0.95):9.3f}  max={max(values):9.3f}  "
            f"spread={spread:8.3f} {self.unit} ({spread_share:5.1f}% of median)"
        )
        if self.load is not None:
            before, after = self.load
            line += f"  load(1m) {_format_load(before)}->{_format_load(after)}"
        return line


def series_from(
    label: str,
    samples: list[float],
    unit: str = "ms",
    load: tuple[float | None, float | None] | None = None,
) -> Series:
    return Series(label=label, unit=unit, samples=samples, load=load)


# ---------------------------------------------------------------------------
# Machine identity and load
# ---------------------------------------------------------------------------


def load_1m() -> float | None:
    """The 1-minute load average, or ``None`` where the platform has none.

    A reading is only comparable with another taken at a similar load, so the
    value is recorded with the samples instead of being assumed.
    """
    try:
        return float(os.getloadavg()[0])
    except (AttributeError, OSError):
        return None


def _format_load(value: float | None) -> str:
    return "n/a" if value is None else f"{value:.2f}"


def load_average_line() -> str:
    """The 1/5/15-minute load averages, as one reported line."""
    try:
        one, five, fifteen = os.getloadavg()
    except (AttributeError, OSError):
        return "unavailable on this platform"
    return f"{one:.2f}, {five:.2f}, {fifteen:.2f} (1, 5, 15 min)"


def machine_identity() -> list[str]:
    """What the numbers below came from. Reported, never interpreted."""
    lines = [
        f"platform          : {platform.platform()}",
        f"machine           : {platform.machine()}",
        f"python            : {platform.python_version()} ({sys.executable})",
        f"logical cpus      : {os.cpu_count()}",
    ]
    if sys.platform == "darwin":
        for key, label in (
            ("machdep.cpu.brand_string", "cpu"),
            ("hw.ncpu", "hw.ncpu"),
            ("hw.memsize", "hw.memsize (bytes)"),
        ):
            value = _sysctl(key)
            if value:
                lines.append(f"{label:<18}: {value}")
    elif sys.platform.startswith("linux"):
        cpuinfo = Path("/proc/cpuinfo")
        if cpuinfo.is_file():
            for line in cpuinfo.read_text(encoding="utf-8").splitlines():
                if line.startswith("model name"):
                    lines.append(f"cpu               : {line.split(':', 1)[1].strip()}")
                    break
    rustc = shutil.which("rustc")
    if rustc:
        result = subprocess.run([rustc, "-vV"], capture_output=True, text=True)
        if result.returncode == 0:
            for line in result.stdout.splitlines():
                if line.startswith(("rustc ", "host:", "LLVM")):
                    lines.append(f"toolchain         : {line}")
    return lines


def _sysctl(key: str) -> str | None:
    if not shutil.which("sysctl"):
        return None
    result = subprocess.run(["sysctl", "-n", key], capture_output=True, text=True)
    return result.stdout.strip() if result.returncode == 0 else None


# ---------------------------------------------------------------------------
# CLI measurement
# ---------------------------------------------------------------------------


def measure_cli_stages(
    bin_path: Path, wav: Path, runs: int
) -> tuple[dict[str, list[float]], tuple[float | None, float | None]]:
    """One `--time` run per repetition, split into the CLI's own stages.

    Returns the per-stage samples and the 1-minute load average before and
    after the runs that produced them.
    """
    samples: dict[str, list[float]] = {stage: [] for stage in CLI_STAGES}
    samples["total"] = []
    load_before = load_1m()
    for _ in range(runs):
        result = subprocess.run(
            [str(bin_path), str(wav), "--output", "/dev/null", "--time"],
            capture_output=True,
            text=True,
        )
        if result.returncode != 0:
            raise RuntimeError(
                f"{bin_path.name} exited {result.returncode} on {wav.name}:\n"
                f"{result.stderr}"
            )
        match = CLI_STAGE_PATTERN.search(result.stderr)
        if match is None:
            raise RuntimeError(f"--time output missing the stage line:\n{result.stderr}")
        for stage, value in zip(CLI_STAGES, match.groups()):
            samples[stage].append(float(value))
        samples["total"].append(sum(float(value) for value in match.groups()))
    return samples, (load_before, load_1m())


def report_cli_corpus(
    bin_path: Path, label: str, wav: Path, runs: int
) -> tuple[dict[str, list[float]], tuple[float | None, float | None]]:
    samples, load = measure_cli_stages(bin_path, wav, runs)
    for stage in (*CLI_STAGES, "total"):
        print(series_from(f"{label}/{stage}", samples[stage], load=load).report())
    return samples, load


# ---------------------------------------------------------------------------
# Kernel stage-split harness
# ---------------------------------------------------------------------------


def run_stage_harness(cargo: str, runs: int) -> tuple[list[str], tuple[float | None, float | None]]:
    """Run the release `stage_timings` harness and return its `wem_perf` lines.

    The harness is `#[ignore]`d so the ordinary workspace run never executes
    it; `--ignored --exact <name> --nocapture` is what makes it report — the
    exact name, because the other ignored test in that file belongs to the
    duration-curve driver and needs an environment this one does not set. The
    load average before and after the subprocess is returned with the lines,
    because the harness runs for minutes and the load it saw is part of the
    result.
    """
    command = [
        cargo,
        "test",
        f"--manifest-path={CARGO_MANIFEST}",
        "--release",
        "-p",
        "wem-core",
        "--test",
        STAGE_TEST,
        "--",
        "--ignored",
        "--exact",
        STAGE_REPORT,
        "--nocapture",
    ]
    print(f"\n$ {' '.join(command)}", flush=True)
    load_before = load_1m()
    result = subprocess.run(command, capture_output=True, text=True)
    load_after = load_1m()
    if result.returncode != 0:
        raise RuntimeError(
            f"the stage harness exited {result.returncode}:\n{result.stdout[-4000:]}\n"
            f"{result.stderr[-4000:]}"
        )
    lines = [
        line
        for line in (result.stdout + result.stderr).splitlines()
        # `wem_perf_fixture` is a header line, not a sample series.
        if line.startswith(("wem_perf ", "wem_perf_attribution "))
    ]
    if not lines:
        raise RuntimeError(
            "the stage harness reported no `wem_perf` lines; the test may have "
            "been filtered out"
        )
    return lines, (load_before, load_after)


def parse_harness_line(line: str) -> tuple[str, str, dict[str, float]]:
    """(kind, corpus, {field: value}) from one `wem_perf` line."""
    match = HARNESS_LINE_PATTERN.match(line)
    if match is None:
        raise RuntimeError(f"unparsable harness line: {line!r}")
    kind, corpus, run, rest = match.group(1), match.group(2), match.group(3), match.group(4)
    if not run.startswith("run="):
        raise RuntimeError(f"harness line is missing its run index: {line!r}")
    fields: dict[str, float] = {}
    for key, number, word in HARNESS_FIELD_PATTERN.findall(rest):
        if number:
            fields[key] = float(number)
        else:
            # Boolean flags arrive as words; keep them as 0/1 so a caller can
            # still aggregate them without a second parser.
            fields[key] = 1.0 if word in ("true", "True") else 0.0
    return kind, corpus, fields


def report_harness(lines: list[str], load: tuple[float | None, float | None]) -> None:
    staged: dict[str, dict[str, list[float]]] = {}
    attribution: dict[str, dict[str, list[float]]] = {}
    for line in lines:
        kind, corpus, fields = parse_harness_line(line)
        target = staged if kind == "wem_perf" else attribution
        bucket = target.setdefault(corpus, {})
        for key, value in fields.items():
            if key == "run":
                continue
            bucket.setdefault(key, []).append(value)

    print(
        "\n-- kernel stage split (`plan_and_windows_ms` contains "
        "`select_modes_ms`, so it is the term that belongs in the sum; the "
        "harness repeats each corpus on its own schedule — read each series' n)"
    )
    for corpus, bucket in staged.items():
        order = [
            "pcm_decode_ms",
            "condition_ms",
            "select_modes_ms",
            "plan_modes_ms",
            "plan_and_windows_ms",
            "analyze_short_ms",
            "analyze_long_ms",
            "pack_ms",
            "container_ms",
            "staged_total_ms",
            "encode_pcm_ms",
        ]
        for key in order:
            if key in bucket:
                print(series_from(f"{corpus}/{key}", bucket[key], load=load).report())
        if "staged_matches_encode_pcm" in bucket:
            matched = all(value == 1.0 for value in bucket["staged_matches_encode_pcm"])
            print(
                f"  {'staged replay == encode_pcm':<34} "
                f"{'yes (every repetition)' if matched else 'NO — the split describes another pipeline'}"
            )

    print(
        "\n-- attribution (re-measured on the same material, single-threaded; "
        "not additive with the split above)"
    )
    for corpus, bucket in attribution.items():
        for key in (
            "fft_log_curve_ms",
            "fft_twiddle_recurrence_cost_model_ms",
            "mdct_ms",
            "mdct_f32_roundtrip_ms",
            "window_rebuild_ms",
            "transient_ms",
        ):
            if key in bucket:
                print(series_from(f"{corpus}/{key}", bucket[key], load=load).report())
        for key in (
            "fft_calls",
            "twiddle_steps",
            "mdct_calls",
            "window_rebuild_calls",
            "transient_quanta",
        ):
            if key in bucket:
                print(
                    series_from(f"{corpus}/{key}", bucket[key], unit="count", load=load).report()
                )


# ---------------------------------------------------------------------------
# Entry point
# ---------------------------------------------------------------------------


def parse_args(argv: list[str]) -> argparse.Namespace:
    parser = argparse.ArgumentParser(
        description=__doc__,
        formatter_class=argparse.RawDescriptionHelpFormatter,
    )
    parser.add_argument(
        "--bin",
        default=str(DEFAULT_BIN),
        help=f"release wwise-wem binary (default: {DEFAULT_BIN})",
    )
    parser.add_argument(
        "--runs",
        type=int,
        default=DEFAULT_RUNS,
        help=f"repetitions per series (default: {DEFAULT_RUNS})",
    )
    parser.add_argument(
        "--stages",
        dest="stages",
        action="store_true",
        default=True,
        help="run the per-stage kernel harness (default; compiles the release "
        "test target, needs a Rust toolchain)",
    )
    parser.add_argument(
        "--no-stages",
        dest="stages",
        action="store_false",
        help="measure through the CLI only; no Rust toolchain needed",
    )
    return parser.parse_args(argv)


def main() -> int:
    args = parse_args(sys.argv[1:])
    if args.runs <= 0:
        print("measure-encode-perf: --runs must be positive", file=sys.stderr)
        return 2
    bin_path = Path(args.bin).expanduser()
    if not bin_path.is_file():
        print(
            f"measure-encode-perf: no release binary at {bin_path}.\n"
            "  build it first:  cargo build --release -p wem-core   (or: make rust-bench)",
            file=sys.stderr,
        )
        return 2

    print("machine identity")
    for line in machine_identity():
        print(f"  {line}")
    print(f"  {'load average':<18}: {load_average_line()}")
    print(f"\nrelease binary     : {bin_path}")
    print(f"repetitions        : {args.runs}")

    all_series: list[Series] = []
    try:
        print(f"\n-- fixture encode ({FIXTURE.name}), CLI stage timers")
        fixture_samples, fixture_load = report_cli_corpus(bin_path, "fixture", FIXTURE, args.runs)

        print("\n-- 2-channel corpora, CLI stage timers")
        corpora: list[Path] = []
        for directory in TWO_CHANNEL_DIRS:
            if not directory.is_dir():
                raise RuntimeError(f"corpus directory missing: {directory}")
            corpora.extend(sorted(directory.glob("*.wav")))
        if not corpora:
            raise RuntimeError("no 2-channel corpus WAVs found")
        for wav in corpora:
            report_cli_corpus(bin_path, wav.stem, wav, args.runs)

        if args.stages:
            cargo = shutil.which("cargo")
            if cargo is None:
                raise RuntimeError(
                    "--stages needs cargo on PATH; rerun with --no-stages to "
                    "measure through the CLI only"
                )
            harness_lines, harness_load = run_stage_harness(cargo, args.runs)
            report_harness(harness_lines, harness_load)

        print("\nsummary (fixture, CLI stage timers)")
        for stage in (*CLI_STAGES, "total"):
            all_series.append(series_from(stage, fixture_samples[stage], load=fixture_load))
        for series in all_series:
            print(series.report())
        realtime = fixture_samples["encode"]
        seconds = _fixture_seconds()
        median_encode = statistics.median(realtime)
        if seconds and median_encode:
            print(
                f"  fixture audio {seconds:.3f} s / median encode {median_encode:.2f} ms "
                f"= {seconds * 1e3 / median_encode:.1f}x real time"
            )
    except (RuntimeError, ValueError) as error:
        print(f"measure-encode-perf: {error}", file=sys.stderr)
        return 1
    return 0


def _fixture_seconds() -> float:
    """Fixture duration from its own header (frames / sample rate)."""
    import wave

    with wave.open(str(FIXTURE), "rb") as handle:
        return handle.getnframes() / handle.getframerate()


if __name__ == "__main__":
    sys.exit(main())
