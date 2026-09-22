#!/usr/bin/env python3
"""Generate the frozen transcendental tables for one exact 2013.2 profile.

The default 6ch/44100 profile derives every value from the current analysis
path (the oracle), then writes the payload and registers it in the profile
manifest by path: the manifest addresses the payload by its path and schema,
so no digest of it is stored beside it.

The 2ch/48000 profile derives its coordinate domain analytically (seed-
surface n with the manifest's sample rate and block sizes; the seed-surface
loader guards 44100 and is not used for this profile) and must ship the
twiddle and window constants byte-identical to the 6ch payload.

Run `make wem-bytes` and the per-frame parity suites afterwards; the reference
encode and every per-frame value must stay identical. That the frozen tables
are the ones the encoder actually reads — and that the live domain is covered
by them — is asserted at test time by
`tests/unit/profiles/test_frozen_tables.py`, which drives a real encode with
the site recorder on and compares the frozen values against a fresh
derivation. This script records nothing to compare against later.

Usage: python3 scripts/generate_frozen_tables.py [--profile NAME]
"""
from __future__ import annotations

import argparse
import json
import math
import struct
import sys
from pathlib import Path

ROOT = Path(__file__).resolve().parent.parent
# Both trees are resolved from this file, so the script runs from any cwd and
# needs no PYTHONPATH: `src` is the distribution facade, `reference` the
# pure-Python oracle that owns the transcendental call sites.
for entry in (ROOT / "src", ROOT / "reference"):
    if str(entry) not in sys.path:
        sys.path.insert(0, str(entry))

from wwise_wem_reference.analysis.dsp.transform import vorbis_window  # noqa: E402

# The recorded profile tree is development material now: it lives untracked
# under `corpus/profiles/`, and this generator is one of the two consumers of
# it (the other is `scripts/generate_profile_code.py`). Nothing here needs the
# native extension.
WWISE_GENERATION = "2013.2"
PROFILE_ROOT = ROOT / "corpus/profiles"
INDEX_PATH = PROFILE_ROOT / "index.json"


def installed_profile_name(generation: str, channels: int, sample_rate: int) -> str:
    """The recorded profile directory whose manifest key is this identity.

    A directory name is a property of the recorded tree, so read it from there
    instead of re-typing it.
    """
    matches = [
        name
        for name, entry in sorted(load_index()["profiles"].items())
        if _manifest_key(entry["manifest"]) == (generation, channels, sample_rate)
    ]
    if len(matches) != 1:
        raise SystemExit(
            f"expected exactly one recorded profile for "
            f"{channels}ch/{sample_rate}Hz/{generation}, found {matches}"
        )
    return matches[0]


def load_index() -> dict:
    return json.loads(INDEX_PATH.read_text())


def _manifest_key(relative: str) -> tuple:
    manifest = json.loads((PROFILE_ROOT / relative).read_text())
    key = manifest["key"]
    return (key["generation"], key["channels"], key["sample_rate"])


SIX_CHANNEL_PROFILE = installed_profile_name(WWISE_GENERATION, 6, 44100)
TWO_CHANNEL_PROFILE = installed_profile_name(WWISE_GENERATION, 2, 48000)
PROFILES = (SIX_CHANNEL_PROFILE, TWO_CHANNEL_PROFILE)
FROZEN_RESOURCE = "analysis.frozen-tables"
FROZEN_SCHEMA = "wem.frozen-math.v1"
FROZEN_RELATIVE_PATH = "analysis/frozen-tables.json"
SHORT_SURFACE_N = 128


def profile_manifest_path(profile: str) -> Path:
    return PROFILE_ROOT / profile / "manifest.json"


def profile_frozen_path(profile: str) -> Path:
    return PROFILE_ROOT / profile / "analysis" / "frozen-tables.json"


def f64_hex(value: float) -> str:
    return struct.pack("<d", value).hex()


def f32_hex(value: float) -> str:
    return struct.pack("<f", value).hex()


def main() -> int:
    parser = argparse.ArgumentParser()
    parser.add_argument("--profile", choices=PROFILES, default=SIX_CHANNEL_PROFILE)
    args = parser.parse_args()
    profile = args.profile

    manifest_path = profile_manifest_path(profile)
    payload_path = profile_frozen_path(profile)

    # Both profiles take their geometry from the recorded manifest: the seed
    # surface is n=128 at the profile's own rate, and the block sizes are the
    # manifest's. The frozen payload is generated *before* the kernel carries
    # it, so nothing here may read the compiled artifact.
    raw_manifest = json.loads(manifest_path.read_text())
    n = SHORT_SURFACE_N
    sample_rate = int(raw_manifest["key"]["sample_rate"])
    window_sizes = tuple(int(size) for size in raw_manifest["block_sizes"])

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

    payload = {
        "schema": FROZEN_SCHEMA,
        "profile": profile,
        "coordinate_ln": coordinate_ln,
        "fft_twiddles": fft_twiddles,
        "window_halves": window_halves,
    }

    payload_path.parent.mkdir(parents=True, exist_ok=True)
    payload_path.write_text(json.dumps(payload, indent=2) + "\n")

    # The manifest and the index address the payload by path and schema. The
    # bytes are the payload's own: no digest of it is stored beside it, and
    # re-running this generator rewrites the same files.
    manifest = json.loads(manifest_path.read_text())
    manifest["resources"][FROZEN_RESOURCE] = {
        "path": FROZEN_RELATIVE_PATH,
        "schema": FROZEN_SCHEMA,
    }
    manifest_path.write_text(json.dumps(manifest, indent=2) + "\n")

    print(f"wrote {payload_path.relative_to(ROOT)}")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
