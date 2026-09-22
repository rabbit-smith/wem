#!/usr/bin/env python3
"""Release-build decode measurement: report the numbers, judge nothing.

Measures the **release** decode and prints what it observed:

  * per-case min / median / p95 / max / spread for every tracked WEM — the
    tracked paired-build fixture (`tests/fixtures/reference.wem`, 6 ch/44100)
    and every ``*.wem`` in the two 2-channel corpora — one **child process per
    repetition**, reaped with ``os.wait4`` so each sample carries its own exit
    instant, its own CPU time and its own ``ru_maxrss``;
  * the kernel's per-stage split for the fixture, read from the
    ``decode_stage_timings`` harness
    (``crates/wem-core/tests/decode_stage_timings.rs``), which replays the
    decode through the public kernel API with ``std::time::Instant`` around
    each existing call — no shipping code carries instrumentation for it;
  * **peak RSS against stream length**, one child process per point reading
    that child's own ``wait4`` rusage, over streams generated deterministically
    from the tracked fixture (see ``--rss-points``).

For every series it prints min / median / p95 / max and the spread, plus the
machine identity the numbers came from (a decode time is meaningless without
knowing the CPU, the core count, the OS and the compiler) and the machine's
load average before and after the series (two readings taken at different loads
are not comparable, so the load is recorded next to the numbers rather than
remembered).

It has no thresholds and no recorded baseline: nothing here decides whether a
number is acceptable, and there is no ``--record``. The exit status is 0
whenever a measurement completed and non-zero only on an actual error (a
missing binary, a failed run, unparsable output, a broken harness) — a loaded
machine changes the numbers, never the exit status. A harness comparison that
comes back false (the staged replay is no longer the shipped pipeline) is an
error, because then the numbers are not a measurement of anything.

``--stages`` (default on) compiles and runs the release ``decode_stage_timings``
test target, which needs a Rust toolchain; ``--no-stages`` measures only through
the child processes. ``--rss`` (default on) adds the peak-RSS-against-length
curve; it generates its longer streams under the gitignored
``crates/target/decode-perf/`` tree from ``tests/fixtures/input.wav`` and the
release encoder binary, so it needs both (``--no-rss`` skips it).

Ambient reads are deliberate and declared: the machine identity, the load
average and the clock. Nothing else is read from the environment, nothing is
downloaded, and the only files written are the generated measurement inputs
under ``crates/target/decode-perf/``.

Usage:
  cargo build --release --manifest-path crates/Cargo.toml -p wem-core
  python3 scripts/measure_decode_perf.py
  python3 scripts/measure_decode_perf.py --runs 15 --no-stages
  python3 scripts/measure_decode_perf.py --rss-points 1,4,16,64 --rss-runs 5
  python3 scripts/measure_decode_perf.py --test-bin path/to/decode_stage_timings

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
import wave
from dataclasses import dataclass
from pathlib import Path

REPO = Path(__file__).resolve().parents[1]
CARGO_MANIFEST = REPO / "crates" / "Cargo.toml"
FIXTURE_WEM = REPO / "tests" / "fixtures" / "reference.wem"
FIXTURE_WAV = REPO / "tests" / "fixtures" / "input.wav"
TWO_CHANNEL_DIRS = (
    REPO / "tests" / "data" / "2ch-reference",
    REPO / "tests" / "data" / "2ch-stress",
)
ENCODER_BIN = REPO / "crates" / "target" / "release" / "wwise-wem"

TEST_TARGET = "decode_stage_timings"
# The two reporting entries in that file, selected by name (`--exact`): the
# stage split the driver reads, and the child it spawns once per measurement
# point. Running every ignored test in the file is not what this means.
REPORT_TEST = "decode_stage_timings_report"
CHILD_TEST = "decode_process_point"

DEFAULT_RUNS = 9
DEFAULT_RSS_RUNS = 3
DEFAULT_RSS_POINTS = (1, 2, 4, 8, 16, 32, 64)
DEFAULT_CHUNK_BYTES = 64 * 1024

# Generated measurement inputs: the deterministic longer streams of the RSS
# curve. Under the gitignored `crates/target/` tree, like the duration-curve
# pair, so nothing here is a committed artifact.
SCRATCH = REPO / "crates" / "target" / "decode-perf"
RSS_INPUTS = SCRATCH / "inputs"
RSS_WEMS = SCRATCH / "wems"

CHILD_LINE = re.compile(
    r"^wem_decode_child corpus=(\S+) bytes=(\d+) chunk_bytes=(\d+) channels=(\d+) "
    r"sample_rate=(\d+) frames=(\d+) declared_frames=(\d+) push_ms=([\d.]+) "
    r"finish_ms=([\d.]+) total_ms=([\d.]+) checksum=(\d+)$"
)
HARNESS_LINE = re.compile(r"^wem_decode_perf corpus=(\S+) run=(\d+)((?: \S+)*)$")
HARNESS_FIELD = re.compile(r"(\w+)=(?:([\d.]+)|(\w+))")


# ---------------------------------------------------------------------------
# Statistics
# ---------------------------------------------------------------------------


def percentile(samples: list[float], fraction: float) -> float:
    """Percentile by linear interpolation between order statistics.

    The numpy default method, spelled out because the standard library has no
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


@dataclass
class Series:
    """One named series of measurements.

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


def maxrss_unit_divisor() -> int:
    """``ru_maxrss`` to bytes: bytes on macOS, kilobytes on Linux."""
    return 1 if sys.platform == "darwin" else 1024


# ---------------------------------------------------------------------------
# The child process: one measurement point is one process
# ---------------------------------------------------------------------------


def locate_test_binary(cargo: str) -> Path:
    """Compile the harness and return the test executable cargo reported.

    The executable is located rather than guessed: cargo's hash suffix is not
    a stable name, and spawning the binary *directly* is what makes the per-run
    ``wait4`` rusage the decode process's own instead of cargo's.
    """
    command = [
        cargo,
        "test",
        f"--manifest-path={CARGO_MANIFEST}",
        "--release",
        "-p",
        "wem-core",
        "--test",
        TEST_TARGET,
        "--no-run",
        "--message-format=json",
    ]
    print(f"\n$ {' '.join(command)}", flush=True)
    result = subprocess.run(command, capture_output=True, text=True)
    if result.returncode != 0:
        raise RuntimeError(
            f"cargo could not build the {TEST_TARGET} test target (exit "
            f"{result.returncode}):\n{result.stderr[-4000:]}"
        )
    executables: list[Path] = []
    for line in result.stdout.splitlines():
        if not line.startswith("{"):
            continue
        try:
            message = json.loads(line)
        except json.JSONDecodeError:
            continue
        target = message.get("target") or {}
        if (
            message.get("reason") == "compiler-artifact"
            and message.get("executable")
            and target.get("name") == TEST_TARGET
        ):
            executables.append(Path(message["executable"]))
    if not executables:
        raise RuntimeError(
            f"cargo reported no executable for the {TEST_TARGET} test target; "
            "is the file still under crates/wem-core/tests/?"
        )
    return executables[-1]


@dataclass
class ChildSample:
    """One child decode, as the driver observed it."""

    label: str
    wem: Path
    chunk_bytes: int
    frames: int
    declared_frames: int
    channels: int
    sample_rate: int
    bytes: int
    push_ms: float
    finish_ms: float
    total_ms: float
    maxrss_bytes: int
    cpu_user_s: float
    cpu_sys_s: float


def run_child(
    binary: Path, wem: Path, chunk_bytes: int, label: str, log_dir: Path, tag: str
) -> ChildSample:
    """Spawn the child test binary for one decode and reap its own rusage.

    ``os.wait4`` on a direct child is the point: the resident figure belongs to
    the decode process and not to this driver, and the child's own reported
    timings cover the shipped session only.
    """
    log_path = log_dir / f"{tag}.log"
    environment = dict(
        os.environ,
        WEM_DECODE_WEM=str(wem),
        WEM_DECODE_CHUNK=str(chunk_bytes),
        WEM_DECODE_LABEL=label,
    )
    argv = [str(binary), "--ignored", "--exact", CHILD_TEST, "--nocapture"]
    pid = os.posix_spawn(
        str(binary),
        argv,
        environment,
        file_actions=[
            (
                os.POSIX_SPAWN_OPEN,
                1,
                str(log_path),
                os.O_WRONLY | os.O_CREAT | os.O_TRUNC,
                0o644,
            )
        ],
    )
    _, status, usage = os.wait4(pid, 0)
    code = os.waitstatus_to_exitcode(status)
    text = log_path.read_text(encoding="utf-8", errors="replace")
    if code != 0:
        raise RuntimeError(
            f"the child decoding {wem.name} exited {code}:\n{text[-4000:]}"
        )
    lines = [line for line in text.splitlines() if line.startswith("wem_decode_child ")]
    if not lines:
        raise RuntimeError(
            f"the child decoding {wem.name} reported no wem_decode_child line; "
            "the test may have been filtered out:\n" + text[-4000:]
        )
    match = CHILD_LINE.match(lines[-1])
    if match is None:
        raise RuntimeError(f"unparsable child line: {lines[-1]!r}")
    divisor = maxrss_unit_divisor()
    return ChildSample(
        label=match.group(1),
        wem=wem,
        chunk_bytes=int(match.group(3)),
        channels=int(match.group(4)),
        sample_rate=int(match.group(5)),
        frames=int(match.group(6)),
        declared_frames=int(match.group(7)),
        push_ms=float(match.group(8)),
        finish_ms=float(match.group(9)),
        total_ms=float(match.group(10)),
        bytes=int(match.group(2)),
        maxrss_bytes=usage.ru_maxrss * divisor,
        cpu_user_s=usage.ru_utime,
        cpu_sys_s=usage.ru_stime,
    )


def audio_seconds(sample: ChildSample) -> float:
    return sample.frames / sample.sample_rate if sample.sample_rate else 0.0


# ---------------------------------------------------------------------------
# Case series: every tracked WEM, one child per repetition
# ---------------------------------------------------------------------------


def tracked_cases() -> list[tuple[str, Path]]:
    """The tracked WEMs: the fixture and the two 2-channel corpora.

    ``corpus/`` is ignored and never read here: every number this script
    reports comes from material the repository commits, or from a stream it
    generated deterministically from that material.
    """
    cases: list[tuple[str, Path]] = [("fixture", FIXTURE_WEM)]
    for directory in TWO_CHANNEL_DIRS:
        if not directory.is_dir():
            raise RuntimeError(f"corpus directory missing: {directory}")
        for path in sorted(directory.glob("*.wem")):
            cases.append((path.stem, path))
    return cases


def measure_cases(
    binary: Path, chunk_bytes: int, runs: int, log_dir: Path
) -> list[ChildSample]:
    """Every tracked case, ``runs`` child processes each."""
    samples: list[ChildSample] = []
    for label, wem in tracked_cases():
        if not wem.is_file():
            raise RuntimeError(f"missing tracked WEM: {wem}")
        load_before = load_1m()
        case: list[ChildSample] = []
        for repetition in range(runs):
            case.append(
                run_child(
                    binary,
                    wem,
                    chunk_bytes,
                    label,
                    log_dir,
                    f"case-{label}-{repetition}",
                )
            )
        load_after = load_1m()
        _report_case(label, wem, case, (load_before, load_after))
        samples.extend(case)
    return samples


def _report_case(
    label: str,
    wem: Path,
    case: list[ChildSample],
    load: tuple[float | None, float | None],
) -> None:
    first = case[0]
    print(
        f"\n-- {label} ({wem.relative_to(REPO)}): {first.channels} ch @ "
        f"{first.sample_rate} Hz, {first.frames} frames "
        f"({audio_seconds(first):.3f} s), {first.bytes} WEM bytes, pushed in "
        f"{first.chunk_bytes}-byte chunks"
    )
    for field_name, unit in (
        ("push_ms", "ms"),
        ("finish_ms", "ms"),
        ("total_ms", "ms"),
    ):
        values = [getattr(sample, field_name) for sample in case]
        print(series_from(f"{label}/{field_name}", values, unit=unit, load=load).report())
    print(
        series_from(
            f"{label}/child_ru_maxrss",
            [sample.maxrss_bytes / 1e6 for sample in case],
            unit="MB",
            load=load,
        ).report()
    )
    cpu = [(sample.cpu_user_s + sample.cpu_sys_s) * 1e3 for sample in case]
    print(
        series_from(
            f"{label}/child_cpu_user+sys", cpu, unit="ms", load=load
        ).report()
    )
    declared = {sample.declared_frames for sample in case}
    delivered = {sample.frames for sample in case}
    if delivered != declared:
        raise RuntimeError(
            f"{label}: the child delivered frames {sorted(delivered)} but the "
            f"containers declare {sorted(declared)}"
        )
    median_total = statistics.median(sample.total_ms for sample in case)
    if median_total and audio_seconds(first):
        print(
            f"  {label:<34} {audio_seconds(first):.3f} s audio / median "
            f"{median_total:.3f} ms = {audio_seconds(first) * 1e3 / median_total:.1f}x real time"
        )


# ---------------------------------------------------------------------------
# Kernel stage-split harness
# ---------------------------------------------------------------------------


def run_stage_harness(
    cargo: str, runs: int, chunk_bytes: int
) -> tuple[list[str], tuple[float | None, float | None]]:
    """Run the release stage harness and return its `wem_decode_perf` lines."""
    command = [
        cargo,
        "test",
        f"--manifest-path={CARGO_MANIFEST}",
        "--release",
        "-p",
        "wem-core",
        "--test",
        TEST_TARGET,
        "--",
        "--ignored",
        "--exact",
        REPORT_TEST,
        "--nocapture",
    ]
    print(f"\n$ {' '.join(command)}", flush=True)
    environment = dict(
        os.environ, WEM_DECODE_RUNS=str(runs), WEM_DECODE_CHUNK=str(chunk_bytes)
    )
    load_before = load_1m()
    result = subprocess.run(command, capture_output=True, text=True, env=environment)
    load_after = load_1m()
    if result.returncode != 0:
        raise RuntimeError(
            f"the stage harness exited {result.returncode}:\n{result.stdout[-4000:]}\n"
            f"{result.stderr[-4000:]}"
        )
    lines = [
        line
        for line in (result.stdout + result.stderr).splitlines()
        if line.startswith("wem_decode_perf ")
    ]
    if not lines:
        raise RuntimeError(
            "the stage harness reported no `wem_decode_perf` lines; the test may "
            "have been filtered out"
        )
    return lines, (load_before, load_after)


def parse_harness_line(line: str) -> tuple[str, dict[str, float | bool]]:
    """(corpus, {field: value}) from one `wem_decode_perf` line."""
    match = HARNESS_LINE.match(line)
    if match is None:
        raise RuntimeError(f"unparsable harness line: {line!r}")
    corpus, _run, rest = match.group(1), match.group(2), match.group(3)
    fields: dict[str, float | bool] = {}
    for key, number, word in HARNESS_FIELD.findall(rest):
        if number:
            fields[key] = float(number)
        else:
            # Boolean flags arrive as words; keep them as booleans so a caller
            # can read the comparison rather than a number.
            fields[key] = word in ("true", "True")
    return corpus, fields


STAGE_ORDER = (
    "framing_ms",
    "setup_ms",
    "header_ms",
    "floors_ms",
    "residue_ms",
    "coupling_ms",
    "envelope_ms",
    "ola_ms",
    "interleave_ms",
    "stages_sum_ms",
    "staged_total_ms",
    "session_push_ms",
    "session_finish_ms",
    "session_total_ms",
    "chunked_push_ms",
    "chunked_finish_ms",
    "chunked_total_ms",
)


def report_harness(
    lines: list[str], load: tuple[float | None, float | None]
) -> None:
    buckets: dict[str, dict[str, list[float]]] = {}
    comparisons: dict[str, dict[str, list[bool]]] = {}
    for line in lines:
        corpus, fields = parse_harness_line(line)
        bucket = buckets.setdefault(corpus, {})
        flags = comparisons.setdefault(corpus, {})
        for key, value in fields.items():
            if isinstance(value, bool):
                flags.setdefault(key, []).append(value)
            else:
                bucket.setdefault(key, []).append(value)

    print(
        "\n-- kernel stage split (`staged_total_ms` is the whole replay; each "
        "series' n is printed because the harness repeats the fixture and runs "
        "the 2-channel corpora once each)"
    )
    for corpus, bucket in buckets.items():
        for key in STAGE_ORDER:
            if key in bucket:
                print(series_from(f"{corpus}/{key}", bucket[key], load=load).report())
        for key in ("frames", "declared_frames", "audio_packets", "pcm_steps"):
            if key in bucket:
                print(
                    series_from(
                        f"{corpus}/{key}", bucket[key], unit="count", load=load
                    ).report()
                )
    for corpus, flags in comparisons.items():
        for key, values in flags.items():
            agreed = all(values)
            print(
                f"  {corpus}/{key:<31} "
                f"{'yes (every repetition)' if agreed else 'NO — the split describes another pipeline'}"
            )
    staged_vs_session = [
        staged
        for corpus, bucket in buckets.items()
        if corpus == "fixture"
        for staged in bucket.get("staged_total_ms", [])
    ]
    session = [
        session
        for corpus, bucket in buckets.items()
        if corpus == "fixture"
        for session in bucket.get("session_total_ms", [])
    ]
    if staged_vs_session and session:
        print(
            f"  {'fixture staged vs session total':<34} "
            f"{statistics.median(staged_vs_session) - statistics.median(session):+.3f} ms "
            "(the replay's own bookkeeping; both describe the shipped pipeline)"
        )


# ---------------------------------------------------------------------------
# Peak RSS against stream length
# ---------------------------------------------------------------------------


def fixture_geometry() -> tuple[int, int, int]:
    """The fixture's (channels, sample rate, frames), from its own header."""
    with wave.open(str(FIXTURE_WAV), "rb") as handle:
        return handle.getnchannels(), handle.getframerate(), handle.getnframes()


def ensure_stream(multiple: int, encoder: Path) -> Path:
    """The WEM for ``multiple`` x the fixture's frames, generated if absent.

    Deterministic by construction and reproducible from the repository: the
    program is the tracked fixture's own PCM16 frames repeated ``multiple``
    times, written with the standard library's WAV writer, and encoded by the
    release encoder binary. Nothing is downloaded and nothing outside
    ``crates/target/decode-perf/`` is written.
    """
    if multiple == 1:
        return FIXTURE_WEM
    channels, rate, frames = fixture_geometry()
    wav_path = RSS_INPUTS / f"fixture-{channels}ch{rate}-{multiple}x.wav"
    wem_path = RSS_WEMS / f"fixture-{channels}ch{rate}-{multiple}x.wem"
    if wem_path.is_file():
        return wem_path
    if not encoder.is_file():
        raise RuntimeError(
            f"missing release encoder {encoder}\n"
            "  build it first:  cargo build --release --manifest-path "
            "crates/Cargo.toml -p wem-core"
        )
    wav_path.parent.mkdir(parents=True, exist_ok=True)
    wem_path.parent.mkdir(parents=True, exist_ok=True)
    if not wav_path.is_file():
        with wave.open(str(FIXTURE_WAV), "rb") as source:
            params = source.getparams()
            payload = source.readframes(params.nframes)
        with wave.open(str(wav_path), "wb") as destination:
            destination.setparams(params)
            for _ in range(multiple):
                destination.writeframes(payload)
    started = time.monotonic()
    result = subprocess.run(
        [str(encoder), str(wav_path), "--output", str(wem_path)],
        capture_output=True,
        text=True,
    )
    if result.returncode != 0 or not wem_path.is_file():
        raise RuntimeError(
            f"the encoder exited {result.returncode} on {wav_path}:\n{result.stderr}"
        )
    print(
        f"  generated {wem_path.relative_to(REPO)} "
        f"({frames * multiple} frames, {wem_path.stat().st_size} bytes) via "
        f"{wav_path.name} in {time.monotonic() - started:.2f}s"
    )
    return wem_path


def measure_rss_curve(
    binary: Path,
    encoder: Path,
    points: list[int],
    runs: int,
    chunk_bytes: int,
    log_dir: Path,
    json_path: Path | None = None,
) -> list[tuple[int, list[ChildSample]]]:
    """One child process per point, each reporting its own ``wait4`` rusage.

    ``json_path`` writes the samples for a plotted figure. Like the concurrency
    pair, the record is the source and the image is rendered from it, so the
    figure can be re-made byte for byte without re-measuring -- which matters
    because a re-measurement would carry a different machine load.
    """
    channels, rate, frames = fixture_geometry()
    print(
        f"\n-- peak RSS against stream length: one child process per point, that "
        f"child's own wait4 rusage ({channels} ch @ {rate} Hz, {frames} frames "
        f"per point-multiple; pushed in {chunk_bytes}-byte chunks)"
    )
    print(
        "  the longer streams are the tracked fixture WAV's own PCM16 frames "
        "repeated N times and encoded by the release encoder (deterministic from "
        "the repository; see the docstring)"
    )
    print(
        f"  {'point':>7}  {'frames':>10}  {'audio s':>8}  {'wem MB':>7}  "
        f"{'rss med MB':>10}  {'rss min..max MB':>17}  {'B/frame':>8}  "
        f"{'B/sample':>9}  {'total med ms':>12}  load(1m)"
    )
    curve: list[tuple[int, list[ChildSample]]] = []
    for multiple in points:
        wem = ensure_stream(multiple, encoder)
        load_before = load_1m()
        case: list[ChildSample] = []
        for repetition in range(runs):
            case.append(
                run_child(
                    binary,
                    wem,
                    chunk_bytes,
                    f"{multiple}x",
                    log_dir,
                    f"rss-{multiple}x-{repetition}",
                )
            )
        load_after = load_1m()
        fresh = case[0]
        expected = frames * multiple
        if fresh.frames != expected:
            raise RuntimeError(
                f"{wem.name} decodes {fresh.frames} frames, not the {expected} "
                "the point-multiple asks for; delete it and measure again"
            )
        rss = [sample.maxrss_bytes for sample in case]
        median_rss = statistics.median(rss)
        per_frame = median_rss / fresh.frames
        per_sample = median_rss / (fresh.frames * fresh.channels)
        print(
            f"  {multiple:>6}x  {fresh.frames:>10}  {audio_seconds(fresh):>8.3f}  "
            f"{fresh.bytes / 1e6:>7.3f}  {median_rss / 1e6:>10.2f}  "
            f"{min(rss) / 1e6:>7.2f}..{max(rss) / 1e6:<7.2f}  {per_frame:>8.2f}  "
            f"{per_sample:>9.2f}  "
            f"{statistics.median(s.total_ms for s in case):>12.3f}  "
            f"{_format_load(load_before)}->{_format_load(load_after)}"
        )
        curve.append((multiple, case))
    _report_rss_growth(curve, channels)
    if json_path is not None:
        write_rss_curve_json(json_path, curve, channels, rate, frames, chunk_bytes)
    return curve


def write_rss_curve_json(
    path: Path,
    curve: list[tuple[int, list[ChildSample]]],
    channels: int,
    rate: int,
    fixture_frames: int,
    chunk_bytes: int,
) -> None:
    """Record the curve so the figure can be re-rendered from it."""
    record = {
        "schema": "wwise-wem.decode-memory-curve.v1",
        "script": "measure_decode_perf.py",
        "machine": machine_identity(),
        "chunk_bytes": chunk_bytes,
        "channels": channels,
        "sample_rate": rate,
        "fixture_frames": fixture_frames,
        "points": [
            {
                "multiple": multiple,
                "frames": case[0].frames,
                "audio_seconds": audio_seconds(case[0]),
                "wem_bytes": case[0].bytes,
                "rss_bytes": [sample.maxrss_bytes for sample in case],
                "total_ms": [sample.total_ms for sample in case],
            }
            for multiple, case in curve
        ],
    }
    path.parent.mkdir(parents=True, exist_ok=True)
    path.write_text(
        json.dumps(record, indent=2, sort_keys=True) + "\n", encoding="utf-8"
    )
    print(f"  recorded {len(curve)} points -> {path}")


def _report_rss_growth(
    curve: list[tuple[int, list[ChildSample]]], channels: int
) -> None:
    """The curve's own growth, reported as arithmetic — never as a verdict.

    A flat curve would be two equal median figures; what is printed here is the
    ratio and the fitted slope, so a reader can see which one this is.
    """
    if len(curve) < 2:
        return
    points = [
        (
            statistics.median(sample.frames for sample in case),
            statistics.median(sample.maxrss_bytes for sample in case),
        )
        for _multiple, case in curve
    ]
    (first_frames, first_rss), (last_frames, last_rss) = points[0], points[-1]
    mean_frames = statistics.fmean(frames for frames, _ in points)
    mean_rss = statistics.fmean(rss for _, rss in points)
    covariance = sum(
        (frames - mean_frames) * (rss - mean_rss) for frames, rss in points
    )
    variance = sum((frames - mean_frames) ** 2 for frames, _ in points)
    fitted = covariance / variance if variance else float("nan")
    print(
        f"  stream length grows {last_frames / first_frames:.1f}x from the first "
        f"point to the last; median peak RSS grows "
        f"{last_rss / first_rss:.2f}x ({first_rss / 1e6:.2f} -> "
        f"{last_rss / 1e6:.2f} MB) — a flat curve would be 1.00x"
    )
    print(
        f"  least-squares slope over all {len(points)} points: "
        f"{fitted:.3f} MB per million frames ({fitted:.2f} bytes per frame, "
        f"{fitted / channels:.2f} bytes per decoded f32 sample)"
    )
    print(
        "  the caller discards every delivered sample, so the delivered PCM does "
        "not accumulate in the child: the growth is the session's own retained "
        "state"
    )


# ---------------------------------------------------------------------------
# Entry point
# ---------------------------------------------------------------------------


def parse_points(text: str) -> list[int]:
    try:
        values = sorted({int(part) for part in text.split(",") if part.strip()})
    except ValueError as error:
        raise SystemExit(f"--rss-points is a comma list of integers: {error}") from error
    if not values or any(value < 1 for value in values):
        raise SystemExit("--rss-points needs at least one positive integer")
    return values


def parse_args(argv: list[str]) -> argparse.Namespace:
    parser = argparse.ArgumentParser(
        description=__doc__,
        formatter_class=argparse.RawDescriptionHelpFormatter,
    )
    parser.add_argument(
        "--runs",
        type=int,
        default=DEFAULT_RUNS,
        help=f"child processes per tracked case (default: {DEFAULT_RUNS})",
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
        help="measure only through the child processes",
    )
    parser.add_argument(
        "--rss",
        dest="rss",
        action="store_true",
        default=True,
        help="measure peak RSS against stream length (default; generates the "
        "longer streams under crates/target/decode-perf/)",
    )
    parser.add_argument(
        "--no-rss",
        dest="rss",
        action="store_false",
        help="skip the RSS curve (no generated inputs, no encoder needed)",
    )
    parser.add_argument(
        "--rss-points",
        default=",".join(str(point) for point in DEFAULT_RSS_POINTS),
        help="comma list of fixture-length multiples for the RSS curve "
        "(default: %(default)s)",
    )
    parser.add_argument(
        "--rss-runs",
        type=int,
        default=DEFAULT_RSS_RUNS,
        help=f"child processes per RSS point (default: {DEFAULT_RSS_RUNS})",
    )
    parser.add_argument(
        "--rss-json",
        default=None,
        help="write the RSS curve's samples here, for "
        "scripts/plot_decode_memory_curve.py to render (default: print only)",
    )
    parser.add_argument(
        "--chunk",
        type=int,
        default=DEFAULT_CHUNK_BYTES,
        help=f"bytes per push given to the session (default: {DEFAULT_CHUNK_BYTES})",
    )
    parser.add_argument(
        "--test-bin",
        type=Path,
        default=None,
        help="the decode_stage_timings test executable to spawn (default: ask "
        "cargo to build it and use what it reports)",
    )
    parser.add_argument(
        "--encoder-bin",
        type=Path,
        default=ENCODER_BIN,
        help=f"release wwise-wem binary used to generate the RSS streams "
        f"(default: {ENCODER_BIN})",
    )
    return parser.parse_args(argv)


def main() -> int:
    args = parse_args(sys.argv[1:])
    if args.runs <= 0 or args.rss_runs <= 0:
        print("measure-decode-perf: --runs and --rss-runs must be positive", file=sys.stderr)
        return 2
    if args.chunk < 0:
        print("measure-decode-perf: --chunk must be zero or positive", file=sys.stderr)
        return 2
    rss_points = parse_points(args.rss_points)

    print("machine identity")
    for line in machine_identity():
        print(f"  {line}")
    print(f"  {'load average':<18}: {load_average_line()}")
    print(f"\nrepetitions        : {args.runs} per tracked case, {args.rss_runs} per RSS point")
    print(f"push chunk         : {args.chunk} bytes")

    try:
        cargo = shutil.which("cargo")
        binary = args.test_bin
        if binary is None:
            if cargo is None:
                raise RuntimeError(
                    "cargo is not on PATH; pass --test-bin to spawn a "
                    "pre-built decode_stage_timings test executable"
                )
            binary = locate_test_binary(cargo)
        binary = Path(binary).expanduser()
        if not binary.is_file():
            raise RuntimeError(
                f"no decode_stage_timings test executable at {binary}\n"
                "  build it with:  cargo test --release --manifest-path "
                "crates/Cargo.toml -p wem-core --test decode_stage_timings --no-run"
            )
        print(f"child binary       : {binary}")

        with tempfile.TemporaryDirectory(prefix="wem-decode-perf-") as tmp:
            log_dir = Path(tmp)
            print("\n-- tracked WEMs, one child process per repetition")
            measure_cases(binary, args.chunk, args.runs, log_dir)

            if args.stages:
                if cargo is None:
                    raise RuntimeError(
                        "--stages needs cargo on PATH; rerun with --no-stages to "
                        "measure through the child processes only"
                    )
                harness_lines, harness_load = run_stage_harness(
                    cargo, args.runs, args.chunk
                )
                report_harness(harness_lines, harness_load)

            if args.rss:
                measure_rss_curve(
                    binary,
                    Path(args.encoder_bin).expanduser(),
                    rss_points,
                    args.rss_runs,
                    args.chunk,
                    log_dir,
                    Path(args.rss_json).expanduser() if args.rss_json else None,
                )
    except (RuntimeError, ValueError) as error:
        print(f"measure-decode-perf: {error}", file=sys.stderr)
        return 1
    return 0


if __name__ == "__main__":
    sys.exit(main())
