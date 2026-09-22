#!/usr/bin/env python3
"""Concurrency measurement: run N encodes at once, report what the machine did.

Every other performance number in this tree is a single encode. This script
measures the axis a server plans with: **how the encoder behaves when several
encodes run at the same time**, with the kernel's internal ``parallel`` feature
on and off, on both installed geometries.

The matrix is ``N concurrent encodes x {parallel on, off} x {6ch/44100,
2ch/48000}``. Each cell runs ``--repetitions`` batches; each batch starts N
child processes as close together as the spawn syscall allows and reaps them
with ``os.wait4``, so every child yields its own exit instant, its own
``ru_utime``/``ru_stime`` and its own ``ru_maxrss``.

Reported per cell:

  * **throughput** — encodes per second over the batch (and audio-seconds per
    second, so the two geometries, which are different lengths, compare
    honestly);
  * **per-encode latency** — median and p95 of the caller-visible interval
    (first spawn of the batch to that child's exit), plus the child's own
    reported encode stage so the process floor can be separated from work;
  * **total CPU** — the batch's summed user + sys, and CPU per encode;
  * **peak RSS** — per child and the projected resident for N concurrent
    encodes (N x median per-child peak).

Machine load is part of the result, not a caveat: the load average is sampled
around every batch and reported with each cell, because this machine is shared
and a load average in the tens changes every number here. The script still
exits 0 — a loaded machine changes the numbers, never the exit status.

The two feature configurations are **paired and order-alternated**: within one
repetition both run at the same N back to back, and which of the two goes first
alternates with the repetition index. That cancels slow drift in machine load
against the comparison the curve exists to make.

Ambient reads are deliberate and declared: the clock, the load average, and
``sysctl`` machine identity are what is being measured or what the numbers are
meaningless without. The clock cannot be made deterministic and is not claimed
to be; the *script* is deterministic in its matrix, its order and its output
path.

The instrument is ``crates/wem-core/tests/concurrency_worker.rs``, and **both
arms are that worker**: one test target built in the two feature configurations,
so an arm cannot differ from the other by its entry point or its startup. The
CLI is deliberately not the instrument — it is a bin target that *requires* the
``parallel`` feature (``crates/wem-core/Cargo.toml``), so it does not exist in
the scalar configuration at all, and a matrix whose two arms cannot both be
built is not a comparison. The worker's line and this script move together; the
contract is at the top of that file.

Usage:
  python3 scripts/measure_concurrency.py                 # builds both arms
  python3 scripts/measure_concurrency.py --repetitions 3 --no-json
  python3 scripts/measure_concurrency.py --n-values 1,2,4,8

Both arms are built here, in release, from the crate that holds the kernel:
``cargo test --release -p wem-core --test concurrency_worker --no-run`` with and
without ``--features parallel``. Pass ``--parallel-bin``/``--scalar-bin`` to
measure prebuilt worker executables instead, and ``--cargo`` to pick the
toolchain. Nothing else is required — there is no binary to build by hand — and
the script reads no number as a threshold.

Exit codes: 0 measured, 1 measurement failed, 2 usage/environment error.
"""
from __future__ import annotations

import argparse
import json
import math
import os
import platform
import re
import shutil
import statistics
import subprocess
import sys
import tempfile
import time
from dataclasses import dataclass, field
from pathlib import Path

REPO = Path(__file__).resolve().parents[1]
CARGO_MANIFEST = REPO / "crates" / "Cargo.toml"

# The two installed geometries, each with the committed input that exercises
# it. Both are under 10 s so the whole matrix finishes; the curve is about
# scaling, not about duration.
GEOMETRIES = (
    (
        "6ch/44100",
        REPO / "tests" / "fixtures" / "input.wav",
        6,
        44100,
        139398,
    ),
    (
        "2ch/48000",
        REPO / "tests" / "data" / "2ch-reference" / "stereo_noise.wav",
        2,
        48000,
        48000,
    ),
)

# Both arms come from this one test target (see the module docstring): the
# `parallel` arm is built with `--features parallel`, the `scalar` arm is the
# library's default configuration. Absolute paths, because a relative path
# resolved against a child's working directory is a different file.
CRATES = REPO / "crates"
WORKER_TEST = "concurrency_worker"

# `parallel` is the opt-in configuration (`--features parallel`); `scalar` is the
# library default, which is what a caller who asks for nothing gets.
CONFIGS = ("parallel", "scalar")

DEFAULT_N_VALUES = (1, 2, 3, 4, 6, 8, 16)
DEFAULT_REPETITIONS = 7
OUTPUT = REPO / "docs" / "figures" / "concurrency-samples.json"

# The worker's one line: `wem_concurrency_worker encode_ms=<ms> load_ms=<ms>
# write_ms=<ms> bytes=<n>`. The instrument's name is in the line, so a stray
# line from another harness can never be read as this one.
WORKER_LINE = re.compile(
    r"wem_concurrency_worker encode_ms=([\d.]+) load_ms=([\d.]+) "
    r"write_ms=([\d.]+) bytes=(\d+)"
)


# ---------------------------------------------------------------------------
# Statistics
# ---------------------------------------------------------------------------


def display(path: Path) -> str:
    """A path as a message should name it: repo-relative where it is in the tree.

    ``--output`` may legitimately point outside the repository — a scratch run
    comparing two sessions — and a report line is not the place to fail on it.
    """
    try:
        return str(path.relative_to(REPO))
    except ValueError:
        return str(path)


def percentile(samples: list[float], fraction: float) -> float:
    """Percentile by linear interpolation between order statistics.

    The numpy default, spelled out because the standard library has no
    percentile, and matched to ``scripts/measure_encode_perf.py`` so a reader
    comparing the two documents compares the same estimator.
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


# ---------------------------------------------------------------------------
# Ambient facts
# ---------------------------------------------------------------------------


def _sysctl(key: str) -> str | None:
    if not shutil.which("sysctl"):
        return None
    result = subprocess.run(["sysctl", "-n", key], capture_output=True, text=True)
    return result.stdout.strip() if result.returncode == 0 else None


def machine_identity() -> dict[str, object]:
    """What the numbers came from. Recorded, never interpreted."""
    identity: dict[str, object] = {
        "platform": platform.platform(),
        "machine": platform.machine(),
        "python": platform.python_version(),
        "logical_cpus": os.cpu_count(),
    }
    if sys.platform == "darwin":
        identity["cpu"] = _sysctl("machdep.cpu.brand_string")
        identity["hw_ncpu"] = _sysctl("hw.ncpu")
        identity["hw_memsize_bytes"] = _sysctl("hw.memsize")
    rustc = shutil.which("rustc")
    if rustc:
        result = subprocess.run([rustc, "-vV"], capture_output=True, text=True)
        if result.returncode == 0:
            for line in result.stdout.splitlines():
                if line.startswith("rustc "):
                    identity["rustc"] = line
                elif line.startswith("LLVM"):
                    identity["llvm"] = line
    return identity


def load_triple() -> list[float]:
    """The 1/5/15-minute load average at this instant."""
    return list(os.getloadavg())


def maxrss_unit_divisor() -> int:
    """``ru_maxrss`` to bytes: bytes on macOS, kilobytes on Linux."""
    return 1 if sys.platform == "darwin" else 1024


# ---------------------------------------------------------------------------
# One batch of N concurrent encodes
# ---------------------------------------------------------------------------


@dataclass
class Child:
    """One child process of a batch, as the parent observed it."""

    pid: int
    latency_s: float
    cpu_user_s: float
    cpu_sys_s: float
    maxrss_bytes: int
    encode_ms: float | None


@dataclass
class Batch:
    """One repetition of one matrix cell: N encodes started together."""

    geometry: str
    config: str
    n: int
    repetition: int
    wall_s: float
    spawn_span_s: float
    children: list[Child] = field(default_factory=list)
    load_before: list[float] = field(default_factory=list)
    load_after: list[float] = field(default_factory=list)

    @property
    def throughput_per_s(self) -> float:
        return self.n / self.wall_s

    @property
    def cpu_total_s(self) -> float:
        return sum(c.cpu_user_s + c.cpu_sys_s for c in self.children)

    @property
    def cpu_per_encode_s(self) -> float:
        return self.cpu_total_s / self.n

    @property
    def cores_busy(self) -> float:
        """Cores this batch actually used, measured rather than assumed.

        ``cpu / wall``: the average number of cores that were running this
        batch's own work. A load average says what the machine as a whole was
        doing and counts threads that are not on a core; this says what share
        of the machine the batch itself received, which is the number a
        scaling claim needs.
        """
        return self.cpu_total_s / self.wall_s

    @property
    def peak_rss_bytes(self) -> int:
        """A typical child's peak RSS: the middle one, upper median for even N.

        Per child first, then reduced, because the batch's capacity figure is
        N x one child rather than the sum of N maxima.
        """
        ordered = sorted(c.maxrss_bytes for c in self.children)
        return ordered[len(ordered) // 2]

    @property
    def projected_resident_bytes(self) -> int:
        return self.n * self.peak_rss_bytes


def run_batch(
    binary: Path,
    wav: Path,
    n: int,
    stderr_dir: Path,
    geometry: str,
    config: str,
    repetition: int,
    divisor: int,
) -> Batch:
    """Start N copies of one encode at once and reap each one's own rusage.

    ``os.wait4(-1, 0)`` is what makes this measurable: it returns as soon as
    *any* child exits, naming that child, so each child's exit instant is the
    parent's own observation rather than a position in a sequential reap
    order. Nothing else is a child of this process during a batch, and the
    returned pid is checked against the set that was spawned.
    """
    load_before = load_triple()
    # pid -> (that child's stderr file, the instant it was spawned). The index
    # is not carried separately because the reap order is by exit, not by spawn.
    launched: dict[int, tuple[Path, float]] = {}

    start = time.monotonic()
    for index in range(n):
        out_path = stderr_dir / f"{geometry.replace('/', '-')}-{config}-r{repetition}-n{n}-{index}.log"
        # One worker test per child, in the configuration this arm was built in:
        # the input WAV and the output path travel in the environment because
        # libtest owns this binary's argv.
        argv = [str(binary), "--ignored", "--exact", WORKER_TEST, "--nocapture"]
        child_env = dict(os.environ)
        child_env["WEM_CONCURRENCY_WAV"] = str(wav)
        child_env["WEM_CONCURRENCY_OUTPUT"] = "/dev/null"
        # Both streams to the per-child file: the worker's line is printed on
        # stdout (libtest's `--nocapture`) and its own harness chatter shares
        # that stream, while a failure report arrives on stderr. A pipe would
        # need draining and a full buffer would block a child forever; a file is
        # simpler and the whole output is a few short lines.
        pid = os.posix_spawn(
            str(binary),
            argv,
            child_env,
            file_actions=[
                (
                    os.POSIX_SPAWN_OPEN,
                    1,
                    str(out_path),
                    os.O_WRONLY | os.O_CREAT | os.O_TRUNC,
                    0o644,
                ),
                # stderr to the same file: `dup2(1, 2)` after the open above.
                (os.POSIX_SPAWN_DUP2, 1, 2),
            ],
        )
        launched[pid] = (out_path, time.monotonic())
    spawn_span = time.monotonic() - start

    exits: dict[int, float] = {}
    usages: dict[int, object] = {}
    for _ in range(n):
        pid, status, usage = os.wait4(-1, 0)
        exits[pid] = time.monotonic()
        usages[pid] = usage
        # A crashed child still reports a rusage, and a truncated encode is
        # cheaper than a whole one: timing one would report a speed-up that is
        # really a failure, so it is an error here rather than a fast sample.
        code = os.waitstatus_to_exitcode(status)
        if code != 0:
            raise RuntimeError(
                f"child {pid} of {geometry}/{config}/N={n}/rep={repetition} "
                f"exited {code}; the batch is not a measurement"
            )
    if set(exits) != set(launched):
        raise RuntimeError(
            f"reaped {sorted(exits)} but spawned {sorted(launched)}; a foreign "
            "child was collected"
        )
    wall = max(exits.values()) - start

    children: list[Child] = []
    for pid, (out_path, spawned_at) in launched.items():
        usage = usages[pid]
        children.append(
            Child(
                pid=pid,
                latency_s=exits[pid] - spawned_at,
                cpu_user_s=usage.ru_utime,
                cpu_sys_s=usage.ru_stime,
                maxrss_bytes=usage.ru_maxrss * divisor,
                encode_ms=read_encode_ms(out_path),
            )
        )
    return Batch(
        geometry=geometry,
        config=config,
        n=n,
        repetition=repetition,
        wall_s=wall,
        spawn_span_s=spawn_span,
        children=children,
        load_before=load_before,
        load_after=load_triple(),
    )


def read_encode_ms(path: Path) -> float | None:
    """The child's own reported encode stage, or None if it did not report."""
    try:
        text = path.read_text(encoding="utf-8", errors="replace")
    except OSError:
        return None
    match = WORKER_LINE.search(text)
    return float(match.group(1)) if match else None


# ---------------------------------------------------------------------------
# The two arms
# ---------------------------------------------------------------------------


def build_arm(cargo: str, parallel: bool) -> Path:
    """Build the measurement worker in one feature configuration.

    ``cargo test --no-run`` compiles the test target without running it, and the
    executable's name carries a hash, so its path is read from cargo's own
    artifact stream rather than guessed from a glob. A failed build is a usage
    error (the toolchain cannot produce the arm), not a failed measurement.
    """
    command = [
        cargo,
        "test",
        "--release",
        "-p",
        "wem-core",
        "--test",
        WORKER_TEST,
        "--no-run",
        "--message-format=json",
    ]
    if parallel:
        command += ["--features", "parallel"]
    result = subprocess.run(command, cwd=CRATES, capture_output=True, text=True)
    if result.returncode != 0:
        raise SystemExit(
            f"building the {WORKER_TEST} worker "
            f"({'parallel' if parallel else 'scalar'}) failed:\n{result.stderr[-4000:]}"
        )
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
        if message.get("target", {}).get("name") == WORKER_TEST and message.get("executable"):
            return Path(message["executable"])
    raise SystemExit(f"cargo reported no executable for the {WORKER_TEST} target")


# ---------------------------------------------------------------------------
# The matrix
# ---------------------------------------------------------------------------


def run_matrix(args: argparse.Namespace) -> dict[str, object]:
    geometries = [(name, wav, channels, rate, frames) for name, wav, channels, rate, frames in GEOMETRIES]
    if args.geometry:
        wanted = set(args.geometry)
        geometries = [g for g in geometries if g[0] in wanted]
        if not geometries:
            raise SystemExit(f"--geometry matched nothing; installed: "
                             f"{', '.join(name for name, *_ in GEOMETRIES)}")
    for name, wav, *_ in geometries:
        if not wav.is_file():
            raise SystemExit(f"missing input for {name}: {wav}")
    binaries = {"parallel": args.parallel_bin, "scalar": args.scalar_bin}
    for config, path in binaries.items():
        if not path.is_file():
            raise SystemExit(
                f"missing {config} worker {path}\n"
                f"--{config}-bin overrides the arm this script builds itself, "
                "and it must be a release build of "
                f"crates/wem-core/tests/{WORKER_TEST}.rs in that configuration:\n"
                "  cargo test --release -p wem-core --test "
                f"{WORKER_TEST} --no-run"
                + ("  --features parallel" if config == "parallel" else "")
            )

    divisor = maxrss_unit_divisor()
    batches: list[Batch] = []
    total = len(geometries) * len(CONFIGS) * len(args.n_values) * args.repetitions
    done = 0
    started = time.time()
    with tempfile.TemporaryDirectory(prefix="wem-concurrency-") as tmp:
        stderr_dir = Path(tmp)
        for name, wav, _channels, _rate, _frames in geometries:
            for repetition in range(args.repetitions):
                for n in args.n_values:
                    # Paired and order-alternated: both configurations run at
                    # this N back to back, and which leads alternates with the
                    # repetition so drift in machine load lands on both.
                    order = CONFIGS if repetition % 2 == 0 else tuple(reversed(CONFIGS))
                    for config in order:
                        batch = run_batch(
                            binaries[config],
                            wav,
                            n,
                            stderr_dir,
                            name,
                            config,
                            repetition,
                            divisor,
                        )
                        batches.append(batch)
                        done += 1
                        print(
                            f"  [{done:3d}/{total}] {name:10s} {config:8s} "
                            f"N={n:<3d} rep={repetition} "
                            f"wall={batch.wall_s * 1000:8.2f}ms "
                            f"thr={batch.throughput_per_s:6.2f}/s "
                            f"cpu/enc={batch.cpu_per_encode_s * 1000:7.2f}ms "
                            f"rss/child={batch.peak_rss_bytes / 1e6:5.1f}MB "
                            f"load={batch.load_before[0]:6.1f}",
                            flush=True,
                        )
    elapsed = time.time() - started
    return {
        "batches": batches,
        "geometries": geometries,
        "elapsed_s": elapsed,
    }


# ---------------------------------------------------------------------------
# Aggregation: the numbers the figure and the finding quote
# ---------------------------------------------------------------------------


def aggregate(
    batches: list[Batch], n_values: list[int], audio_s: dict[str, float]
) -> list[dict[str, object]]:
    cells: list[dict[str, object]] = []
    keys = sorted({(b.geometry, b.config, b.n) for b in batches})
    for geometry, config, n in keys:
        group = [b for b in batches if b.geometry == geometry and b.config == config and b.n == n]
        latencies = [c.latency_s for b in group for c in b.children]
        encodes = [c.encode_ms for b in group for c in b.children if c.encode_ms is not None]
        rss = [b.peak_rss_bytes for b in group]
        loads = [value for b in group for value in (b.load_before[0], b.load_after[0])]
        wall = statistics.median(b.wall_s for b in group)
        cells.append(
            {
                "geometry": geometry,
                "config": config,
                "n": n,
                "batches": len(group),
                "samples": len(latencies),
                "throughput_per_s": n / wall,
                # The two geometries are different lengths (3.161 s against
                # 1.000 s), so encodes/s alone would not compare them: this is
                # the same rate as audio-seconds of input per second of wall
                # clock, which is length-free.
                "audio_s_per_s": (n / wall) * audio_s.get(geometry, float("nan")),
                "batch_wall_ms_median": wall * 1000.0,
                "latency_ms_median": statistics.median(latencies) * 1000.0,
                "latency_ms_p95": percentile(latencies, 0.95) * 1000.0,
                "latency_ms_max": max(latencies) * 1000.0,
                "encode_ms_median": statistics.median(encodes) if encodes else None,
                "cpu_total_ms_median": statistics.median(b.cpu_total_s for b in group) * 1000.0,
                "cpu_per_encode_ms_median": statistics.median(b.cpu_per_encode_s for b in group) * 1000.0,
                "cores_busy_median": statistics.median(b.cores_busy for b in group),
                "peak_rss_mb_median": statistics.median(rss) / 1e6,
                "projected_resident_mb": n * statistics.median(rss) / 1e6,
                "load_1min_min": min(loads),
                "load_1min_median": statistics.median(loads),
                "load_1min_max": max(loads),
                "spawn_span_ms_median": statistics.median(b.spawn_span_s for b in group) * 1000.0,
            }
        )
    cells.sort(key=lambda c: (c["geometry"], c["config"], c["n"]))
    add_scaling(cells)
    return cells


def add_scaling(cells: list[dict[str, object]]) -> None:
    """Speedup and the ideal-linear rate, both anchored at that series' N=1.

    The ideal reference is what a reader needs in order to see where scaling
    stops: ``N x`` the single-encode rate is the rate a machine with one free
    core per concurrent encode would deliver.
    """
    baseline = {(c["geometry"], c["config"]): c for c in cells if c["n"] == 1}
    for cell in cells:
        anchor = baseline.get((cell["geometry"], cell["config"]))
        if anchor is None:
            cell["speedup_vs_n1"] = None
            cell["ideal_throughput_per_s"] = None
            continue
        cell["speedup_vs_n1"] = cell["throughput_per_s"] / anchor["throughput_per_s"]
        cell["ideal_throughput_per_s"] = cell["n"] * anchor["throughput_per_s"]


def paired_comparison(batches: list[Batch]) -> list[dict[str, object]]:
    """Parallel against scalar, paired inside one repetition.

    This is the load-insensitive comparison. The two configurations run back to
    back at the same N within one repetition and swap order with the repetition
    index, so a machine whose load drifts — which this one does — moves both
    sides of the ratio together. A ratio above 1 means the kernel's internal
    parallelism is still paying for itself at that N; at or below 1 it is being
    paid for out of somebody else's core.
    """
    index = {(b.geometry, b.config, b.n, b.repetition): b for b in batches}
    keys = sorted({(b.geometry, b.n) for b in batches})
    rows: list[dict[str, object]] = []
    for geometry, n in keys:
        repetitions = sorted(
            {b.repetition for b in batches if b.geometry == geometry and b.n == n}
        )
        throughput, latency, cpu_per_encode = [], [], []
        for repetition in repetitions:
            parallel = index.get((geometry, "parallel", n, repetition))
            scalar = index.get((geometry, "scalar", n, repetition))
            if parallel is None or scalar is None:
                continue
            throughput.append(parallel.throughput_per_s / scalar.throughput_per_s)
            latency.append(
                statistics.median(c.latency_s for c in parallel.children)
                / statistics.median(c.latency_s for c in scalar.children)
            )
            cpu_per_encode.append(parallel.cpu_per_encode_s / scalar.cpu_per_encode_s)
        if not throughput:
            continue
        rows.append(
            {
                "geometry": geometry,
                "n": n,
                "pairs": len(throughput),
                "throughput_ratio_median": statistics.median(throughput),
                "throughput_ratio_min": min(throughput),
                "throughput_ratio_max": max(throughput),
                "latency_ratio_median": statistics.median(latency),
                "cpu_per_encode_ratio_median": statistics.median(cpu_per_encode),
            }
        )
    return rows


def serialise(batch: Batch) -> dict[str, object]:
    return {
        "geometry": batch.geometry,
        "config": batch.config,
        "n": batch.n,
        "repetition": batch.repetition,
        "wall_s": batch.wall_s,
        "spawn_span_s": batch.spawn_span_s,
        "load_before": batch.load_before,
        "load_after": batch.load_after,
        "latency_s": [c.latency_s for c in batch.children],
        "cpu_user_s": [c.cpu_user_s for c in batch.children],
        "cpu_sys_s": [c.cpu_sys_s for c in batch.children],
        "maxrss_bytes": [c.maxrss_bytes for c in batch.children],
        "encode_ms": [c.encode_ms for c in batch.children],
        # Per batch, so the raw record can be re-aggregated later without
        # having to re-derive what "cores busy" meant.
        "cpu_total_s": batch.cpu_total_s,
        "cores_busy": batch.cores_busy,
    }


# ---------------------------------------------------------------------------
# Entry point
# ---------------------------------------------------------------------------


def parse_n_values(text: str) -> list[int]:
    try:
        values = [int(part) for part in text.split(",") if part.strip()]
    except ValueError as error:
        raise SystemExit(f"--n-values is a comma list of integers: {error}") from error
    if not values or any(value < 1 for value in values):
        raise SystemExit("--n-values needs at least one positive integer")
    return sorted(set(values))


def main(argv: list[str] | None = None) -> int:
    parser = argparse.ArgumentParser(
        description="Measure N concurrent encodes across both installed geometries.",
        formatter_class=argparse.RawDescriptionHelpFormatter,
        epilog=(
            "The machine must be quiet for the numbers to describe the machine "
            "rather than the neighbours; the load average is recorded with "
            "every cell either way."
        ),
    )
    parser.add_argument("--n-values", default=",".join(str(n) for n in DEFAULT_N_VALUES),
                        help="comma list of concurrency levels (default: %(default)s)")
    parser.add_argument("--repetitions", type=int, default=DEFAULT_REPETITIONS,
                        help="batches per cell (default: %(default)s)")
    parser.add_argument("--geometry", action="append", default=None,
                        help="restrict to one installed geometry; repeatable")
    parser.add_argument("--cargo", default=shutil.which("cargo") or "cargo",
                        help="cargo used to build the two arms (default: %(default)s)")
    parser.add_argument("--parallel-bin", type=Path, default=None,
                        help="prebuilt release concurrency_worker built with "
                             "--features parallel (default: built here)")
    parser.add_argument("--scalar-bin", type=Path, default=None,
                        help="prebuilt release concurrency_worker in the library's "
                             "default (scalar) configuration (default: built here)")
    parser.add_argument("--output", type=Path, default=OUTPUT,
                        help="where the raw samples are written (default: %(default)s)")
    parser.add_argument("--no-json", action="store_true",
                        help="print the report and write nothing")
    args = parser.parse_args(argv)
    if args.repetitions < 1:
        raise SystemExit("--repetitions must be at least 1")
    args.n_values = parse_n_values(args.n_values)

    print("machine identity:")
    identity = machine_identity()
    for key, value in identity.items():
        print(f"  {key:<18}: {value}")
    print(
        f"\nload now (1/5/15m): "
        f"{'/'.join(f'{value:.1f}' for value in load_triple())}"
    )
    # Both arms, in the two feature configurations of one test target. The
    # matrix is meaningless if the two were not built differently, so the paths
    # are printed before anything is measured and recorded in the JSON.
    args.parallel_bin = args.parallel_bin or build_arm(args.cargo, parallel=True)
    args.scalar_bin = args.scalar_bin or build_arm(args.cargo, parallel=False)
    print(
        f"arms: parallel={display(args.parallel_bin)} "
        f"scalar={display(args.scalar_bin)}"
    )
    print(
        f"matrix: N={args.n_values} x configs={list(CONFIGS)} x "
        f"geometries x {args.repetitions} repetitions\n"
    )

    result = run_matrix(args)
    batches: list[Batch] = result["batches"]
    audio_s = {name: frames / rate for name, _wav, _ch, rate, frames in result["geometries"]}
    cells = aggregate(batches, args.n_values, audio_s)

    headers = ("geometry", "config", "N", "thr/s", "audio-s/s", "lat med",
               "lat p95", "enc med", "cpu batch", "cpu/enc", "cores",
               "rss/child", "resident", "load 1m")
    print("\n" + "  ".join(f"{h:>10s}" for h in headers))
    for cell in cells:
        print("  ".join(f"{value:>10s}" for value in (
            str(cell["geometry"]),
            str(cell["config"]),
            str(cell["n"]),
            f"{cell['throughput_per_s']:.2f}",
            f"{cell['audio_s_per_s']:.2f}",
            f"{cell['latency_ms_median']:.1f}",
            f"{cell['latency_ms_p95']:.1f}",
            f"{cell['encode_ms_median']:.1f}" if cell["encode_ms_median"] else "-",
            f"{cell['cpu_total_ms_median']:.1f}",
            f"{cell['cpu_per_encode_ms_median']:.1f}",
            f"{cell['cores_busy_median']:.2f}",
            f"{cell['peak_rss_mb_median']:.1f}",
            f"{cell['projected_resident_mb']:.0f}",
            f"{cell['load_1min_median']:.0f}",
        )))
    print("\nlatencies, CPU and resident columns are milliseconds, MB and MB.")

    paired = paired_comparison(batches)
    print("\npaired parallel/scalar ratios (median over repetitions; >1 means the")
    print("kernel's internal parallelism is still paying for itself at that N)")
    print(f"  {'geometry':<12}{'N':>4}{'pairs':>7}{'thr ratio':>11}"
          f"{'lat ratio':>11}{'cpu ratio':>11}")
    for row in paired:
        print(f"  {row['geometry']:<12}{row['n']:>4}{row['pairs']:>7}"
              f"{row['throughput_ratio_median']:>11.3f}"
              f"{row['latency_ratio_median']:>11.3f}"
              f"{row['cpu_per_encode_ratio_median']:>11.3f}")

    if args.no_json:
        return 0
    document = {
        "schema": 1,
        "generated_by": "scripts/measure_concurrency.py",
        "rendered_by": "scripts/plot_concurrency_curves.py",
        "machine": identity,
        "matrix": {
            "n_values": args.n_values,
            "configs": list(CONFIGS),
            "geometries": [
                {
                    "name": name,
                    "input": str(wav.relative_to(REPO)),
                    "channels": channels,
                    "sample_rate": rate,
                    "frames": frames,
                    "audio_s": frames / rate,
                }
                for name, wav, channels, rate, frames in result["geometries"]
            ],
            "repetitions": args.repetitions,
            "order": "N ascending; config order alternates with the repetition index",
        },
        "bins": {
            "instrument": f"crates/wem-core/tests/{WORKER_TEST}.rs",
            "parallel": str(args.parallel_bin),
            "scalar": str(args.scalar_bin),
        },
        "load_at_start": load_triple(),
        "elapsed_s": result["elapsed_s"],
        "cells": cells,
        "paired": paired,
        "batches": [serialise(batch) for batch in batches],
    }
    args.output.parent.mkdir(parents=True, exist_ok=True)
    args.output.write_text(
        json.dumps(document, indent=2, sort_keys=False) + "\n", encoding="utf-8"
    )
    print(f"\nwrote {display(args.output)} "
          f"({len(batches)} batches, {sum(len(b.children) for b in batches)} encodes, "
          f"{result['elapsed_s']:.1f}s of wall clock)")
    return 0


if __name__ == "__main__":
    sys.exit(main())
