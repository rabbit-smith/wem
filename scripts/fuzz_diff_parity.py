#!/usr/bin/env python3
"""Random PCM differential parity: reference oracle vs native kernel.

The executable definition of oracle-vs-kernel parity at scale: every
stream is encoded once by the pure-Python reference oracle (one-shot) and
once by the native kernel (streaming lifecycle with a randomized
frame-aligned chunk split), and the two containers must be byte-identical.
The golden fixture is checked three-way (oracle == native == the accepted
reference WEM).

Fully deterministic: a fixed seed set, no network, no clock, no ambient
state; a re-run reproduces the same stream set byte-for-byte.

Usage:
  PYTHONPATH=src:reference python3 scripts/fuzz_diff_parity.py --pr
      PR gate: golden case + a small deterministic differential set
      (budget < 1 minute).
  PYTHONPATH=src:reference python3 scripts/fuzz_diff_parity.py --full
      Nightly: golden case + the full deterministic differential set
      (budget < 30 minutes including oracle cost).

Exit codes: 0 pass, 1 parity failure (the first mismatch is reported with
case id, seed, geometry, and digests), 2 usage/environment error.
"""
from __future__ import annotations

import argparse
import hashlib
import os
import random
import struct
import sys
import time
from pathlib import Path

REPO = Path(__file__).resolve().parents[1]
FIXTURES = REPO / "tests" / "fixtures"
PROFILE_NAME = "wwise2013-6ch-44100"
CHANNELS = 6
SAMPLE_RATE = 44100
EXPECTED_REFERENCE_SHA256 = (
    "17851d26c6210b85e498ae0452d2562d7b9e2c3e9e795c459656b9c9d8d35247"
)
MIN_FRAMES = 4096
MAX_CHUNKS = 8

# PR gate: small deterministic set (golden + these cases, < 1 minute).
PR_CASES = 10
PR_MAX_FRAMES = 6144
PR_SEED_BASE = 20130701

# Nightly: the full deterministic set (golden + these cases, < 30 minutes).
FULL_CASES = 200
FULL_MAX_FRAMES = 8192
FULL_SEED_BASE = 20130712

# 2ch/48000 parity contract: a fixed, named geometry item set pinned into
# the contract (never randomized at run time).  The canonical seed list is
# deterministic; the PR tier runs the leading slice and the full tier runs
# the complete set, so the PR set is a strict subset of the full set.  A
# separate seed family (302407xx) keeps the 2ch items disjoint from the
# 6ch differential seeds (201307xx).
TWO_CH_PROFILE = "wwise2013-2ch-48000"
TWO_CH_SAMPLE_RATE = 48000
_TWO_CH_48K_SEEDS = (
    30240701,
    30240702,
    30240703,
    30240704,
    30240705,
    30240706,
    30240707,
    30240708,
    30240709,
    30240710,
    30240711,
    30240712,
    30240713,
    30240714,
    30240715,
    30240716,
    30240717,
    30240718,
    30240719,
    30240720,
)
PR_TWO_CH_SEEDS = _TWO_CH_48K_SEEDS[:5]
FULL_TWO_CH_SEEDS = _TWO_CH_48K_SEEDS


def _sha256_hex(data: bytes) -> str:
    return hashlib.sha256(data).hexdigest()


def _random_stream(
    seed: int, channels: int, min_frames: int, max_frames: int
) -> tuple[int, bytes]:
    """One deterministic random PCM stream: (frames, interleaved s16le)."""
    rng = random.Random(seed)
    frames = rng.randint(min_frames, max_frames)
    payload = bytearray(frames * channels * 2)
    rng.seed(seed ^ 0x5EED)
    for index in range(0, len(payload), 2):
        payload[index : index + 2] = struct.pack(
            "<h", rng.randint(-32768, 32767)
        )
    return frames, bytes(payload)


def _random_chunks(seed: int, frames: int, channels: int) -> list[slice]:
    """A deterministic frame-aligned chunk tiling of the stream."""
    rng = random.Random(seed ^ 0xC0FFEE)
    chunk_count = rng.randint(1, MAX_CHUNKS)
    cuts = sorted(rng.randint(1, frames - 1) for _ in range(chunk_count - 1))
    cuts = sorted(set(cuts))
    chunk_count = len(cuts)
    boundaries = [0, *cuts, frames]
    return [
        slice(lo * channels * 2, hi * channels * 2)
        for lo, hi in zip(boundaries, boundaries[1:])
    ]


def _golden_case(native) -> None:
    """Oracle == native == accepted reference WEM, three-way on the fixture."""
    from wwise_wem.adapters.wav import read_pcm16
    from wwise_wem import load_wem_profile

    from wwise_wem_reference import python_engine
    from wwise_wem_reference.container.model import ContainerPlan

    reference = (FIXTURES / "reference.wem").read_bytes()
    if _sha256_hex(reference) != EXPECTED_REFERENCE_SHA256:
        raise RuntimeError(
            f"reference.wem fixture drifted: {_sha256_hex(reference)}"
        )

    profile = load_wem_profile(PROFILE_NAME)
    pcm = read_pcm16(FIXTURES / "input.wav")

    container = ContainerPlan.from_profile(profile)
    oracle = python_engine.encode_pcm_python(
        profile=profile, container=container, pcm=pcm
    )
    oracle_bytes = bytes(oracle.data)

    rows = [
        [int(sample * 32768.0) for sample in row] for row in pcm.channels
    ]
    native_result = native.Encoder(PROFILE_NAME).encode_pcm(SAMPLE_RATE, rows)
    native_bytes = bytes(native_result.data)

    if not (oracle_bytes == reference == native_bytes):
        raise RuntimeError(
            "golden parity failed: oracle={o} native={n} reference={r}".format(
                o=_sha256_hex(oracle_bytes),
                n=_sha256_hex(native_bytes),
                r=EXPECTED_REFERENCE_SHA256,
            )
        )
    print("golden: oracle == native == reference.wem (byte-identical)")


def _differential_case(
    native,
    case_id: int,
    seed: int,
    min_frames: int,
    max_frames: int,
    profile,
    channels: int,
    sample_rate: int,
    profile_name: str,
) -> None:
    """Oracle one-shot vs native streaming with a random chunk split."""
    from wwise_wem_reference import python_engine
    from wwise_wem_reference.container.model import ContainerPlan

    frames, pcm_bytes = _random_stream(seed, channels, min_frames, max_frames)
    chunks = _random_chunks(seed, frames, channels)
    if len(chunks) < 1 or chunks[0].start != 0:
        raise RuntimeError("chunk tiling malformed")

    rows = struct.unpack(f"<{frames * channels}h", pcm_bytes)
    pcm_channels = tuple(
        tuple(rows[frame * channels + c] for frame in range(frames))
        for c in range(channels)
    )
    # Public float domain: sample / 32768 (exactly representable in binary).
    pcm_float = tuple(
        tuple(value / 32768.0 for value in channel) for channel in pcm_channels
    )

    container = ContainerPlan.from_profile(profile)
    oracle = python_engine.encode_pcm_python(
        profile=profile,
        container=container,
        pcm=_pcm_buffer(sample_rate, pcm_float, frames),
    )
    oracle_bytes = bytes(oracle.data)

    setup_sha = profile.setup_sha256
    session = native.StreamSession()
    session.start(setup_sha, name=profile_name)
    for chunk in chunks:
        session.push(pcm_bytes[chunk])
    complete = session.finish()
    native_bytes = bytes(complete.bytes)

    if oracle_bytes != native_bytes:
        raise RuntimeError(
            "differential parity failed: case={case} seed={seed} geometry="
            "{channels}ch/{rate}Hz frames={frames} chunks={chunks} oracle={o} "
            "native={n}".format(
                case=case_id,
                seed=seed,
                channels=channels,
                rate=sample_rate,
                frames=frames,
                chunks=len(chunks),
                o=_sha256_hex(oracle_bytes),
                n=_sha256_hex(native_bytes),
            )
        )


def _pcm_buffer(sample_rate: int, channels: tuple[tuple[float, ...], ...],
                frames: int):
    from wwise_wem.model import PcmBuffer

    return PcmBuffer(sample_rate, channels)


def _parse_args(argv: list[str]) -> argparse.Namespace:
    parser = argparse.ArgumentParser(
        description=__doc__,
        formatter_class=argparse.RawDescriptionHelpFormatter,
    )
    mode = parser.add_mutually_exclusive_group(required=True)
    mode.add_argument(
        "--pr",
        action="store_true",
        help="PR gate: golden + small deterministic set (< 1 minute)",
    )
    mode.add_argument(
        "--full",
        action="store_true",
        help="Nightly: golden + full deterministic set (< 30 minutes)",
    )
    parser.add_argument(
        "--cases",
        type=int,
        default=None,
        help="differential case count (overrides the mode default)",
    )
    args = parser.parse_args(argv)
    if args.pr:
        args.case_count = PR_CASES if args.cases is None else args.cases
        args.max_frames = PR_MAX_FRAMES
        args.seed_base = PR_SEED_BASE
    else:
        args.case_count = FULL_CASES if args.cases is None else args.cases
        args.max_frames = FULL_MAX_FRAMES
        args.seed_base = FULL_SEED_BASE
    return args


def main() -> int:
    args = _parse_args(sys.argv[1:])

    try:
        import wwise_wem_reference  # noqa: F401
    except ImportError as error:
        print(
            "fuzz_diff_parity: the reference oracle is required "
            "(PYTHONPATH must include the repository's reference/ tree)",
            file=sys.stderr,
        )
        print(f"  {error}", file=sys.stderr)
        return 2

    try:
        import wwise_wem._core as native
    except ImportError as error:
        print(
            "fuzz_diff_parity: the native extension wwise_wem._core is "
            "required (pip install -e . or install a maturin wheel)",
            file=sys.stderr,
        )
        print(f"  {error}", file=sys.stderr)
        return 2

    # The kernel resolves profile data from WEM_DATA_DIR or the repository
    # layout; pin it explicitly so the script is cwd-independent.
    profiles_dir = REPO / "src" / "wwise_wem" / "data" / "profiles"
    os.environ["WEM_DATA_DIR"] = str(profiles_dir)

    from wwise_wem import load_wem_profile

    profile = load_wem_profile(PROFILE_NAME)
    # The 2ch/48k geometry item uses its own installed profile.
    two_ch_profile = load_wem_profile(TWO_CH_PROFILE)
    two_ch_seeds = (
        PR_TWO_CH_SEEDS if args.pr else FULL_TWO_CH_SEEDS
    )

    start = time.monotonic()
    _golden_case(native)
    for case_id in range(args.case_count):
        seed = args.seed_base + case_id
        started = time.monotonic()
        _differential_case(
            native,
            case_id,
            seed,
            MIN_FRAMES,
            args.max_frames,
            profile,
            CHANNELS,
            SAMPLE_RATE,
            PROFILE_NAME,
        )
        elapsed = time.monotonic() - started
        print(
            f"case {case_id:03d}: seed={seed} ok ({elapsed:.1f}s)"
        )

    # Fixed 2ch/48k parity items: the seeds are pinned into the contract and
    # both tiers run their (nested) sets.
    for seed in two_ch_seeds:
        started = time.monotonic()
        _differential_case(
            native,
            seed,
            seed,
            MIN_FRAMES,
            args.max_frames,
            two_ch_profile,
            2,
            TWO_CH_SAMPLE_RATE,
            TWO_CH_PROFILE,
        )
        elapsed = time.monotonic() - started
        print(
            f"case 2ch : seed={seed} ok ({elapsed:.1f}s)"
        )

    total = time.monotonic() - start
    print(
        f"fuzz_diff_parity OK: golden + {args.case_count} differential + "
        f"{len(two_ch_seeds)} 2ch cases byte-identical in {total:.0f}s"
    )
    return 0


if __name__ == "__main__":
    sys.exit(main())
