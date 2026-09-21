#!/usr/bin/env python3
"""Generate the frozen transcendental tables for one exact 2013.2 profile.

The default 6ch/44100 profile derives every value from the current analysis
path (the oracle), cross-checks the generated input domain against the
recording produced by ``scripts/record_tmath.py``, then writes the payload,
registers it in the profile manifest, and re-addresses index.json by the
manifest's new SHA-256.

The 2ch/48000 profile derives its coordinate domain analytically (seed-
surface n with the manifest's sample rate and block sizes; the seed-surface
loader guards 44100 and is not used for this profile) and must ship the
twiddle and window constants byte-identical to the 6ch payload. Its
cross-check runs only once a 2ch-domain recording exists; it is skipped
until then.

Run `make golden` and the per-frame parity suites afterwards; the golden
encode and every per-frame value must stay identical.

Usage: python3 scripts/generate_frozen_tables.py [--profile NAME] [--record-dir DIR]
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
# Both trees are resolved from this file, so the script runs from any cwd and
# needs no PYTHONPATH: `src` is the distribution facade, `reference` the
# pure-Python oracle that owns the transcendental recorders.
for entry in (ROOT / "src", ROOT / "reference"):
    if str(entry) not in sys.path:
        sys.path.insert(0, str(entry))

from wwise_wem_reference._tmath import math_bits  # noqa: E402
from wwise_wem_reference.analysis.dsp.transform import vorbis_window  # noqa: E402
from wwise_wem.profiles.bundle import (  # noqa: E402
    WWISE_GENERATION,
    installed_profile_names,
    load_profile_bundle,
)
from wwise_wem_reference.profiles.psychoacoustics.config import load_short_seed_surface  # noqa: E402


def installed_profile_name(generation: str, channels: int, sample_rate: int) -> str:
    """The packaged profile directory whose manifest key is this identity.

    A directory name is a property of the packaged tree, so read it from there
    instead of re-typing it. Read through the index and manifests only: this is
    a generation script, and it must run before — and without — the native
    extension, exactly as the wheel's zip-import check does.
    """
    matches = [
        bundle.name
        for bundle in (
            load_profile_bundle(profile=name, verify_all=False)
            for name in installed_profile_names()
        )
        if (
            bundle.key.generation,
            bundle.key.channels,
            bundle.key.sample_rate,
        )
        == (generation, channels, sample_rate)
    ]
    if len(matches) != 1:
        raise SystemExit(
            f"expected exactly one installed profile for "
            f"{channels}ch/{sample_rate}Hz/{generation}, found {matches}"
        )
    return matches[0]


SIX_CHANNEL_PROFILE = installed_profile_name(WWISE_GENERATION, 6, 44100)
TWO_CHANNEL_PROFILE = installed_profile_name(WWISE_GENERATION, 2, 48000)
PROFILES = (SIX_CHANNEL_PROFILE, TWO_CHANNEL_PROFILE)
FROZEN_RESOURCE = "analysis.frozen-tables"
FROZEN_SCHEMA = "wem.frozen-math.v1"
FROZEN_RELATIVE_PATH = "analysis/frozen-tables.json"
SHORT_SURFACE_N = 128
INDEX_PATH = ROOT / "src/wwise_wem/data/profiles/index.json"
RECORD_DIR_BY_PROFILE = {
    SIX_CHANNEL_PROFILE: ROOT / "tests/data/stage-golden/transcendental",
    TWO_CHANNEL_PROFILE: ROOT / "tests/data/stage-golden/transcendental-2ch-48000",
}


def profile_manifest_path(profile: str) -> Path:
    return ROOT / "src/wwise_wem/data/profiles" / profile / "manifest.json"


def profile_frozen_path(profile: str) -> Path:
    return ROOT / "src/wwise_wem/data/profiles" / profile / "analysis" / "frozen-tables.json"


def f64_hex(value: float) -> str:
    return struct.pack("<d", value).hex()


def f32_hex(value: float) -> str:
    return struct.pack("<f", value).hex()


def hex_to_u64(hex_bytes: str) -> int:
    """Little-endian byte-hex to its integer bit pattern (matches math_bits)."""
    return int.from_bytes(bytes.fromhex(hex_bytes), "little")


def read_recording_pairs(path: Path) -> dict[int, int]:
    data = path.read_bytes()
    pairs = {}
    for offset in range(0, len(data), 16):
        in_bits, out_bits = struct.unpack_from("<QQ", data, offset)
        pairs[in_bits] = out_bits
    return pairs


def main() -> int:
    parser = argparse.ArgumentParser()
    parser.add_argument("--profile", choices=PROFILES, default=SIX_CHANNEL_PROFILE)
    parser.add_argument("--record-dir", default=None)
    args = parser.parse_args()
    profile = args.profile

    manifest_path = profile_manifest_path(profile)
    payload_path = profile_frozen_path(profile)

    if profile == SIX_CHANNEL_PROFILE:
        bundle = load_profile_bundle(profile=profile)
        surface = load_short_seed_surface(
            bundle.runtime_manifest.resource("psychoacoustics.short-seed")
        )
        n, sample_rate = surface.n, surface.sample_rate
        window_sizes = (256, 2048)
    else:
        # Analytic 48k-domain: seed-surface n with the manifest's sample rate
        # and block sizes (load_short_seed_surface must not be used here).
        raw_manifest = json.loads(manifest_path.read_text())
        n = SHORT_SURFACE_N
        sample_rate = int(raw_manifest["key"]["sample_rate"])
        window_sizes = tuple(int(size) for size in raw_manifest["block_sizes"])

    record_dir = (
        Path(args.record_dir) if args.record_dir else RECORD_DIR_BY_PROFILE[profile]
    )

    coordinate_ln = []
    for index in range(n):
        frequency = (float(index) + 0.5) * float(sample_rate) / float(2 * n)
        coordinate_ln.append([f64_hex(frequency), f64_hex(math.log(frequency))])

    fft_twiddles = []
    for length in (2 << k for k in range(11)):
        angle = -2.0 * math.pi / length
        fft_twiddles.append([str(length), f64_hex(math.cos(angle)), f64_hex(math.sin(angle))])

    window_halves = {}
    for size in window_sizes:
        window = vorbis_window(size)
        window_halves[str(size)] = [f32_hex(value) for value in window[: size // 2]]

    if profile == TWO_CHANNEL_PROFILE:
        # The twiddle and window constants are geometry-invariant: the 2ch
        # payload must carry the 6ch constants byte-identical.
        six_payload = json.loads(profile_frozen_path(SIX_CHANNEL_PROFILE).read_text())
        if fft_twiddles != six_payload["fft_twiddles"]:
            raise SystemExit("2ch fft_twiddles drifted from the 6ch constants")
        if window_halves != six_payload["window_halves"]:
            raise SystemExit("2ch window_halves drifted from the 6ch constants")

    recording_checks = 0
    coord_recording = record_dir / "config.ln.in-out.u64le.bin"
    if coord_recording.exists():
        generated = {hex_to_u64(a): hex_to_u64(b) for a, b in coordinate_ln}
        assert generated == read_recording_pairs(coord_recording), "config.ln domain drift"
        recording_checks += 1
    for name in ("spectrum.cos", "spectrum.sin"):
        path = record_dir / f"{name}.in-out.u64le.bin"
        if not path.exists():
            continue
        idx = 1 if name.endswith("cos") else 2
        generated = {
            math_bits(-2.0 * math.pi / int(row[0])): hex_to_u64(row[idx]) for row in fft_twiddles
        }
        assert generated == read_recording_pairs(path), f"{name} domain drift"
        recording_checks += 1
    window_recording = record_dir / "transform.window_short.sin.in-out.u64le.bin"
    if window_recording.exists():
        expected = read_recording_pairs(window_recording)
        for size in window_sizes:
            half = size // 2
            for i in range(half):
                inner_in = math.pi * (i + 0.5) / size
                inner_out = math.sin(inner_in)
                outer_in = math.pi / 2.0 * inner_out**2
                assert math_bits(outer_in) in expected, "window outer input outside recording"
                assert math_bits(inner_in) in expected, "window inner input outside recording"
        recording_checks += 1

    payload = {
        "schema": FROZEN_SCHEMA,
        "profile": profile,
        "coordinate_ln": coordinate_ln,
        "fft_twiddles": fft_twiddles,
        "window_halves": window_halves,
    }

    payload_path.parent.mkdir(parents=True, exist_ok=True)
    payload_path.write_text(json.dumps(payload, indent=2) + "\n")

    digest = hashlib.sha256(payload_path.read_bytes()).hexdigest()
    manifest = json.loads(manifest_path.read_text())
    manifest["resources"][FROZEN_RESOURCE] = {
        "path": FROZEN_RELATIVE_PATH,
        "schema": FROZEN_SCHEMA,
        "sha256": digest,
    }
    manifest_path.write_text(json.dumps(manifest, indent=2) + "\n")

    manifest_digest = hashlib.sha256(manifest_path.read_bytes()).hexdigest()
    index = json.loads(INDEX_PATH.read_text())
    entry = index["profiles"][profile]
    if entry["sha256"] == manifest_digest:
        raise SystemExit(f"index already addresses the {profile} manifest; nothing to do")
    entry["sha256"] = manifest_digest
    INDEX_PATH.write_text(json.dumps(index, indent=2) + "\n")

    print(f"wrote {payload_path.relative_to(ROOT)} sha256={digest}")
    print(f"{profile} manifest sha256={manifest_digest} (index re-addressed)")
    print(f"recording cross-checks passed: {recording_checks}")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
