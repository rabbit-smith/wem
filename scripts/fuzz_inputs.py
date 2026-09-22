#!/usr/bin/env python3
"""Run bounded, coverage-guided WEM/WAV fuzzing seeded by committed fixtures.

Requires cargo-fuzz and a nightly Rust toolchain. Corpus growth and crash
artifacts stay under crates/fuzz's ignored directories; fixtures are never
modified. A crash, timeout or failed build is a nonzero exit.
"""
from __future__ import annotations

import argparse
import os
import shutil
import subprocess
from pathlib import Path

ROOT = Path(__file__).resolve().parents[1]


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--seconds", type=int, default=60, help="time budget per target (default: 60)")
    args = parser.parse_args()
    if args.seconds <= 0:
        parser.error("--seconds must be positive")
    # cargo-fuzz starts cargo/rustc by name: keep both on the selected
    # toolchain even when a system installation precedes rustup on PATH.
    toolchain = Path(subprocess.check_output(
        ["rustup", "which", "--toolchain", "nightly", "cargo"], text=True,
    ).strip()).parent
    environment = dict(os.environ)
    environment["PATH"] = str(toolchain) + os.pathsep + environment.get("PATH", "")
    environment["RUSTC"] = str(toolchain / "rustc")
    environment.pop("CARGO_ENCODED_RUSTFLAGS", None)
    for target, suffix, maximum in [("wem_decode", "wem", 131072), ("wav_parse", "wav", 2097152)]:
        corpus = ROOT / "crates" / "fuzz" / "corpus" / target
        corpus.mkdir(parents=True, exist_ok=True)
        sources = [ROOT / "tests" / "fixtures" / f"{'reference' if suffix == 'wem' else 'input'}.{suffix}"]
        sources.extend(sorted((ROOT / "tests" / "data" / "2ch-reference").glob(f"*.{suffix}")))
        sources.extend(sorted((ROOT / "tests" / "data" / "2ch-stress").glob(f"*.{suffix}")))
        for source in sources:
            shutil.copyfile(source, corpus / source.name)
        result = subprocess.run(
            ["rustup", "run", "nightly", "cargo", "fuzz", "run", target, "--",
             f"-max_total_time={args.seconds}", f"-max_len={maximum}",
             "-timeout=10", "-rss_limit_mb=2048"], cwd=ROOT / "crates", env=environment,
        )
        if result.returncode:
            return result.returncode
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
