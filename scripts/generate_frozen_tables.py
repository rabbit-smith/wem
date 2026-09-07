#!/usr/bin/env python3
"""Generate the frozen transcendental tables for the exact 2013.2 profile.

Derives every value from the current analysis path (the oracle), cross-checks
the generated input domain against a capture produced by
``scripts/record_tmath.py``, then writes the payload, registers it in the
profile manifest, and re-addresses index.json by the manifest's new SHA-256.

Run the golden and frame-contract suites afterwards; the fixture encode must
stay byte-identical.

Usage: python3 scripts/generate_frozen_tables.py [--record-dir DIR]
"""
from __future__ import annotations

import argparse
import hashlib
import json
import math
import struct
import sys
from pathlib import Path

ROOT = Path(__file__).resolve().parent.parent
sys.path.insert(0, str(ROOT / "src"))

from wwise_wem_reference._tmath import math_bits  # noqa: E402
from wwise_wem_reference.analysis.dsp.transform import vorbis_window  # noqa: E402
from wwise_wem.profiles.bundle import load_profile_bundle  # noqa: E402
from wwise_wem_reference.profiles.psychoacoustics.config import load_short_seed_surface  # noqa: E402

PROFILE = "wwise2013-6ch-44100"
MANIFEST_PATH = ROOT / "src/wwise_wem/data/profiles" / PROFILE / "manifest.json"
PAYLOAD_PATH = ROOT / "src/wwise_wem/data/profiles" / PROFILE / "analysis/frozen-tables.json"
INDEX_PATH = ROOT / "src/wwise_wem/data/profiles/index.json"


def f64_hex(value: float) -> str:
    return struct.pack("<d", value).hex()


def f32_hex(value: float) -> str:
    return struct.pack("<f", value).hex()


def hex_to_u64(hex_bytes: str) -> int:
    """Little-endian byte-hex to its integer bit pattern (matches math_bits)."""
    return int.from_bytes(bytes.fromhex(hex_bytes), "little")


def read_capture_pairs(path: Path) -> dict[int, int]:
    data = path.read_bytes()
    pairs = {}
    for offset in range(0, len(data), 16):
        in_bits, out_bits = struct.unpack_from("<QQ", data, offset)
        pairs[in_bits] = out_bits
    return pairs


def main() -> int:
    parser = argparse.ArgumentParser()
    parser.add_argument("--record-dir", default=str(ROOT / "tests/data/stage-golden/transcendental"))
    args = parser.parse_args()
    record_dir = Path(args.record_dir)

    bundle = load_profile_bundle(profile=PROFILE)
    surface = load_short_seed_surface(bundle.runtime_manifest.resource("psychoacoustics.short-seed"))
    n, sample_rate = surface.n, surface.sample_rate

    coordinate_ln = []
    for index in range(n):
        frequency = (float(index) + 0.5) * float(sample_rate) / float(2 * n)
        coordinate_ln.append([f64_hex(frequency), f64_hex(math.log(frequency))])

    fft_twiddles = []
    for length in (2 << k for k in range(11)):
        angle = -2.0 * math.pi / length
        fft_twiddles.append([str(length), f64_hex(math.cos(angle)), f64_hex(math.sin(angle))])

    window_halves = {}
    for size in (256, 2048):
        window = vorbis_window(size)
        window_halves[str(size)] = [f32_hex(value) for value in window[: size // 2]]

    payload = {
        "schema": "wem.frozen-math.v1",
        "profile": PROFILE,
        "coordinate_ln": coordinate_ln,
        "fft_twiddles": fft_twiddles,
        "window_halves": window_halves,
    }

    capture_checks = 0
    coord_capture = record_dir / "config.ln.in-out.u64le.bin"
    if coord_capture.exists():
        generated = {hex_to_u64(a): hex_to_u64(b) for a, b in coordinate_ln}
        assert generated == read_capture_pairs(coord_capture), "config.ln domain drift"
        capture_checks += 1
    for name in ("spectrum.cos", "spectrum.sin"):
        path = record_dir / f"{name}.in-out.u64le.bin"
        if not path.exists():
            continue
        idx = 1 if name.endswith("cos") else 2
        generated = {
            math_bits(-2.0 * math.pi / int(row[0])): hex_to_u64(row[idx]) for row in fft_twiddles
        }
        assert generated == read_capture_pairs(path), f"{name} domain drift"
        capture_checks += 1
    window_capture = record_dir / "transform.window_short.sin.in-out.u64le.bin"
    if window_capture.exists():
        expected = read_capture_pairs(window_capture)
        for size in (256, 2048):
            half = size // 2
            for i in range(half):
                inner_in = math.pi * (i + 0.5) / size
                inner_out = math.sin(inner_in)
                outer_in = math.pi / 2.0 * inner_out**2
                assert math_bits(outer_in) in expected, "window outer input outside capture"
                assert math_bits(inner_in) in expected, "window inner input outside capture"
        capture_checks += 1

    PAYLOAD_PATH.parent.mkdir(parents=True, exist_ok=True)
    PAYLOAD_PATH.write_text(json.dumps(payload, indent=2) + "\n")

    digest = hashlib.sha256(PAYLOAD_PATH.read_bytes()).hexdigest()
    manifest = json.loads(MANIFEST_PATH.read_text())
    manifest["resources"]["analysis.frozen-tables"] = {
        "path": "analysis/frozen-tables.json",
        "schema": "wem.frozen-math.v1",
        "sha256": digest,
    }
    MANIFEST_PATH.write_text(json.dumps(manifest, indent=2) + "\n")

    manifest_digest = hashlib.sha256(MANIFEST_PATH.read_bytes()).hexdigest()
    index = json.loads(INDEX_PATH.read_text())
    entry = index["profiles"][PROFILE]
    if entry["sha256"] == manifest_digest:
        raise SystemExit("index already addresses the rewritten manifest; nothing to do")
    entry["sha256"] = manifest_digest
    INDEX_PATH.write_text(json.dumps(index, indent=2) + "\n")

    print(f"wrote {PAYLOAD_PATH.relative_to(ROOT)} sha256={digest}")
    print(f"manifest sha256={manifest_digest} (index re-addressed)")
    print(f"capture cross-checks passed: {capture_checks}")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
