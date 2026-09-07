#!/usr/bin/env python3
"""Capture the live transcendental input domain for the exact Wwise 2013.2 profile.

Runs one full fixture encoding with ``WEM_TMATH_RECORD`` enabled (the variable
must be set before importing the package, which this script guarantees), writes
per-site ``(input bits, output bits)`` records under
``tests/data/stage-golden/transcendental/``, then reports:

- unique input count per site (0 = site not on the live path);
- for ``floor_fit.ln``: the minimum distance of the dB-quant intermediate to an
  integer boundary, i.e. how much libm ULP disagreement the profile can absorb
  before ``q`` changes;
- an ``enumerable`` verdict for sites with a small unique-input set.

Usage: python3 scripts/record_tmath.py [output-dir]
"""
from __future__ import annotations

import json
import os
import struct
import sys
from pathlib import Path

ROOT = Path(__file__).resolve().parent.parent
OUT_DIR = Path(sys.argv[1]) if len(sys.argv) > 1 else ROOT / "tests/data/stage-golden/transcendental"

os.environ["WEM_TMATH_RECORD"] = str(OUT_DIR)
sys.path.insert(0, str(ROOT / "src"))

from wwise_wem import encode_wav  # noqa: E402
from wwise_wem_reference._tmath import write_recording  # noqa: E402


def read_pairs(path: Path) -> list[tuple[float, float]]:
    data = path.read_bytes()
    if len(data) % 16:
        raise ValueError(f"{path} is not a multiple of 16 bytes")
    pairs = []
    for offset in range(0, len(data), 16):
        in_bits, out_bits = struct.unpack_from("<QQ", data, offset)
        pairs.append(
            (
                struct.unpack("<d", struct.pack("<Q", in_bits))[0],
                struct.unpack("<d", struct.pack("<Q", out_bits))[0],
            )
        )
    return pairs


def main() -> int:
    result = encode_wav(ROOT / "tests/fixtures/input.wav")
    counts = write_recording()

    stats: dict[str, dict[str, object]] = {}
    for name, count in counts.items():
        site: dict[str, object] = {"unique_inputs": count, "on_live_path": count > 0}
        stats[name] = site

    floor_path = OUT_DIR / "floor_fit.ln.in-out.u64le.bin"
    if floor_path.exists() and counts.get("floor_fit.ln", 0):
        worst = float("inf")
        worst_input = 0.0
        near = 0
        for ln_in, ln_out in read_pairs(floor_path):
            x = ln_out * 4.34294480 * 7.3142857 + 1023.5
            margin = abs(x - round(x))
            if margin < worst:
                worst, worst_input = margin, ln_in
            near += margin < 1e-9
        stats["floor_fit.ln"].update(
            {
                "min_boundary_margin": worst,
                "min_margin_input": worst_input,
                "records_within_1e-9": near,
                "enumerable": False,
            }
        )
    for name in ("spectrum.cos", "spectrum.sin", "config.ln", "transform.cos", "transform.sin"):
        count = counts.get(name, 0)
        if name in stats:
            stats[name]["enumerable"] = 0 < count <= 4096

    OUT_DIR.mkdir(parents=True, exist_ok=True)
    (OUT_DIR / "stats.json").write_text(json.dumps(stats, indent=2, sort_keys=True) + "\n")

    print(f"encode ok: {result.stats.audio_packets} packets, {result.stats.bytes} bytes")
    for name, site in sorted(stats.items()):
        print(f"{name}: {json.dumps(site)}")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
