#!/usr/bin/env python3
"""Paired-corpus toolkit (Wwise WEM encoder calibration).

Generates deterministic test PCM plus a minimal Wwise 2013 project template
and a headless CLI invocation, so a user with a Wwise install can produce
reference WEM bytes to cross-check this implementation.

This tool is repo infrastructure. It is offline, deterministic, and writes
only to destinations it is explicitly given. It does NOT touch trusted
versioned assets; its outputs are scratch/reference material.

Subcommands:
  gen-wavs       Deterministically generate signed-16 PCM test WAVs (2ch/48k).
  gen-project    Emit a minimal Wwise 2013 project template (XML).
  emit-command   Print a Wwise headless/CLI invocation string.

Design notes:
  * The WAVs are 2-channel / 48000 Hz / signed-16 PCM to match the paired
    build's 2ch/48k quality profile (profile idx 0/2, rate band 45000-50000).
  * Lengths exercise the encoder's edge cases: 4096 (minimum accepted),
    4097 (just over), 8192 (two short blocks), and 30 s (long stream).
  * Content types exercise different encoder paths: tonal sweep, broadband
    noise, a pluck-like transient (transient detection), and a silence
    boundary.
  * The Wwise project template and CLI invocation are best-effort: exact
    Wwise 2013.2 project internals and the official batch tool are not
    reproduced verbatim; uncertain fields are emitted as <PLACEHOLDER:...>
    and the tool warns on stdout so a human can adjust them.
"""
from __future__ import annotations

import argparse
import json
import math
import random
import struct
import sys
import wave
from pathlib import Path

# --------------------------------------------------------------------------
# Fixed corpus geometry (do not change without updating the docs above).
# --------------------------------------------------------------------------
SAMPLE_RATE = 48000
CHANNELS = 2
BITS_PER_SAMPLE = 16
BYTE_RATE = SAMPLE_RATE * CHANNELS * (BITS_PER_SAMPLE // 8)

# Length set in frames: {4096, 4097, 8192, 30 s}. 30 s @ 48 kHz = 1,440,000.
LENGTHS = (4096, 4097, 8192, 30 * SAMPLE_RATE)

# Content types (deterministic generators below).
CONTENT_TYPES = (
    "sine_sweep",
    "white_noise",
    "pluck_transient",
    "silence_boundary",
)

# Fixed seeds so every run is byte-identical.
_SEED_NOISE = 0x5EED1
_SEED_SWEEP = 0x5EED2


def _clamp16(x: float) -> int:
    """Round to signed-16 with saturation (deterministic)."""
    n = int(round(x))
    if n > 32767:
        return 32767
    if n < -32768:
        return -32768
    return n


def gen_sine_sweep(frames: int, ch: int) -> list[list[int]]:
    """A 200 Hz -> 8 kHz logarithmic sweep, phase-continuous, per channel.

    Channel 1 leads channel 2 by a fixed 90-degree phase offset so the
    stereo pair is decorrelated but deterministic.
    """
    f0, f1 = 200.0, 8000.0
    out: list[list[int]] = []
    phase = [0.0, 0.0]
    off = [0.0, math.pi / 2.0]
    for n in range(frames):
        frac = n / frames
        freq = f0 * (f1 / f0) ** frac
        row: list[int] = []
        for c in range(ch):
            phase[c] += 2.0 * math.pi * freq / SAMPLE_RATE
            row.append(_clamp16(0.8 * 32767.0 * math.sin(phase[c] + off[c])))
        out.append(row)
    return out


def gen_white_noise(frames: int, ch: int) -> list[list[int]]:
    """Deterministic white noise (full-range, seeded)."""
    rng = random.Random(_SEED_NOISE)
    out: list[list[int]] = []
    for _ in range(frames):
        row: list[int] = []
        for _c in range(ch):
            row.append(_clamp16(0.9 * 32767.0 * rng.uniform(-1.0, 1.0)))
        out.append(row)
    return out


def gen_pluck_transient(frames: int, ch: int) -> list[list[int]]:
    """A pluck-like transient: sharp attack, exponentially decaying tone.

    Exercises transient detection; a short broadband burst then a decaying
    440 Hz partial on channel 0, and a quiet echo on channel 1.
    """
    out: list[list[int]] = []
    burst = 256  # samples of broadband attack
    decay = 180.0  # decay rate (higher = faster)
    amp = 0.9
    phase = 0.0
    for n in range(frames):
        row: list[int] = []
        if n < burst:
            # broadband attack (deterministic ramped noise-free triangle)
            v = amp * (1.0 - n / burst) * math.sin(2.0 * math.pi * 1200.0 * n / SAMPLE_RATE)
            row.append(_clamp16(v * 32767.0))
            row.append(_clamp16(0.5 * v * 32767.0))
        else:
            env = math.exp(-decay * (n - burst) / SAMPLE_RATE)
            phase += 2.0 * math.pi * 440.0 / SAMPLE_RATE
            v0 = amp * env * math.sin(phase)
            row.append(_clamp16(v0 * 32767.0))
            row.append(_clamp16(0.4 * v0 * 32767.0))
        out.append(row)
    return out


def gen_silence_boundary(frames: int, ch: int) -> list[list[int]]:
    """Mostly silence with a short 1 kHz tone at the very start.

    Exercises silence/boundary handling (codec should switch to long/silence
    mode after the tone).
    """
    tone = 96  # 96 samples of tone at the start
    amp = 0.6
    out: list[list[int]] = []
    for n in range(frames):
        row: list[int] = []
        if n < tone:
            v = amp * math.sin(2.0 * math.pi * 1000.0 * n / SAMPLE_RATE)
            row.append(_clamp16(v * 32767.0))
            row.append(_clamp16(v * 32767.0))
        else:
            row.append(0)
            row.append(0)
        out.append(row)
    return out


_GENERATORS = {
    "sine_sweep": gen_sine_sweep,
    "white_noise": gen_white_noise,
    "pluck_transient": gen_pluck_transient,
    "silence_boundary": gen_silence_boundary,
}


def _write_wav(path: Path, frames: list[list[int]]) -> None:
    with wave.open(str(path), "wb") as w:
        w.setnchannels(CHANNELS)
        w.setsampwidth(2)  # 16-bit
        w.setframerate(SAMPLE_RATE)
        for row in frames:
            for c in row:
                w.writeframes(struct.pack("<h", c))


def cmd_gen_wavs(args: argparse.Namespace) -> int:
    out_dir = Path(args.out)
    out_dir.mkdir(parents=True, exist_ok=True)

    files: list[dict[str, object]] = []
    manifest: dict[str, object] = {
        "schema": "wem.rs-corpus-manifest.v1",
        "sample_rate": SAMPLE_RATE,
        "channels": CHANNELS,
        "bits_per_sample": BITS_PER_SAMPLE,
        "byte_rate": BYTE_RATE,
        "files": files,
    }

    for length in LENGTHS:
        for ctype in CONTENT_TYPES:
            fname = f"{length}_{ctype}.wav"
            fpath = out_dir / fname
            frames = _GENERATORS[ctype](length, CHANNELS)
            _write_wav(fpath, frames)
            files.append(
                {
                    "path": fname,
                    "content": ctype,
                    "frames": length,
                    "seconds": length / SAMPLE_RATE,
                }
            )

    manifest_path = out_dir / "manifest.json"
    with open(manifest_path, "w") as f:
        json.dump(manifest, f, indent=1, sort_keys=True)
        f.write("\n")

    print(f"gen-wavs: wrote {len(files)} WAVs + manifest to {out_dir}")
    return 0


# --------------------------------------------------------------------------
# Wwise 2013 project template (best-effort; placeholders for unknowns).
# --------------------------------------------------------------------------
# NOTE: The exact Wwise 2013.2 project serialization is proprietary. The XML
# below is a minimal, clearly-labelled template intended as a starting point
# for a human with a real Wwise install. Uncertain fields are PLACEHOLDERs.


def _project_template(channels: int, sample_rate: int, plugin_id: int, quality: int) -> str:
    # Return an XML template string.
    return f"""<?xml version="1.0" encoding="utf-8"?>
<!--
  Minimal Wwise 2013.2 project template for WEM calibration.
  Generated by scripts/make_rs_corpus.py gen-project.
  BEST-EFFORT: adjust against your actual Wwise install before use.
  PLACEHOLDER markers indicate fields that are not verifiable without a
  real Wwise environment and must be filled in manually.
-->
<AudioProject version="2013.2.0.6566" <!-- PLACEHOLDER:version -->
             guid="{_guid_for('project')}">
  <WorkUnit>
    <Name>rs_corpus</Name>
    <Settings>
      <Global>
        <!-- Target geometry: matches the corpus WAVs. -->
        <Channels>{channels}</Channels>
        <SampleRate>{sample_rate}</SampleRate>
      </Global>
    </Settings>
  </WorkUnit>
  <!-- Vorbis (WEM) codec configuration. -->
  <Codec>
    <Name>Vorbis</Name>
    <PluginId>{plugin_id}</PluginId>
    <!-- PLACEHOLDER:codec-settings : exact codec fields differ by build -->
    <Settings>
      <!-- Quality factor: one of the two calibration points (q4 or q6). -->
      <QualityFactor>{quality}</QualityFactor>
      <!-- PLACEHOLDER:block-sizes : short/long block configuration -->
      <ShortBlockSize>256</ShortBlockSize>
      <LongBlockSize>2048</LongBlockSize>
    </Settings>
  </Codec>
  <!-- Sound objects: one per corpus WAV. Fill paths + import settings. -->
  <Sounds>
    <Sound guid="{_guid_for('sound')}">
      <Name>rs_corpus_input</Name>
      <!-- PLACEHOLDER:sound-file : absolute path to a corpus WAV -->
      <File>{_placeholder("input-wav-path")}</File>
      <Codec>Vorbis</Codec>
      <QualityFactor>{quality}</QualityFactor>
    </Sound>
  </Sounds>
</AudioProject>
"""


def _guid_for(kind: str) -> str:
    # Deterministic pseudo-GUID (not a real UUID; just stable per kind): the
    # kind's own bytes, hex-spelled and padded to the GUID shape. Derived
    # directly from the name, so nothing here hashes anything.
    h = kind.encode("utf-8").hex().ljust(32, "0")[:32]
    return (
        h[0:8] + "-" + h[8:12] + "-" + h[12:16] + "-" + h[16:20] + "-" + h[20:32]
    )


def _placeholder(what: str) -> str:
    return f"<PLACEHOLDER:{what}>"


def cmd_gen_project(args: argparse.Namespace) -> int:
    warnings: list[str] = []
    for q in (args.quality,):
        warnings.append(
            f"gen-project: quality q{q} written; confirm QualityFactor semantics "
            "and codec fields against your Wwise 2013.2 build."
        )
    warnings.append(
        "gen-project: template is best-effort; fill all PLACEHOLDER fields "
        "(version, codec-settings, sound-file paths) in a real Wwise project."
    )
    for w in warnings:
        print(f"WARNING: {w}", file=sys.stderr)

    out_path = (
        Path(args.out)
        if args.out
        else Path("rs_corpus_2013_q4.xml")
    )
    text = _project_template(
        channels=CHANNELS,
        sample_rate=SAMPLE_RATE,
        plugin_id=args.plugin_id,
        quality=args.quality,
    )
    out_path.write_text(text, encoding="utf-8")
    print(f"gen-project: wrote template to {out_path}")
    return 0


# --------------------------------------------------------------------------
# Headless CLI invocation.
# --------------------------------------------------------------------------
def cmd_emit_command(args: argparse.Namespace) -> int:
    # NOTE: There is no official standalone "WwiseCLI" that batch-converts
    # WAV->WEM with a quality factor. The command below is a plausible
    # invocation for a user's headless Wwise environment; replace
    # <wwise-cli> and flags with the real tool/flags for your install.
    project_dir = args.project_dir
    cmd = (
        f"<wwise-cli> convert "
        f"--project {project_dir} "
        f"--codec vorbis "
        f"--plugin-id {args.plugin_id} "
        f"--channels {CHANNELS} "
        f"--sample-rate {SAMPLE_RATE} "
        f"--quality {args.quality} "
        f"--format wem "
        f"--out <output-dir>"
    )
    # Emit two variants (q4 and q6) when --both is set.
    if args.both:
        for q in (4, 6):
            line = cmd.replace(f"--quality {args.quality}", f"--quality {q}")
            print(f"# quality q{q}")
            print(line)
    else:
        print(cmd)
    print(
        "\n# NOTE: <wwise-cli> and flags are placeholders; adapt to your\n"
        "# actual Wwise 2013.2 headless tool. The exact batch command is\n"
        "# not standardized across installs.",
        file=sys.stderr,
    )
    return 0


def build_parser() -> argparse.ArgumentParser:
    p = argparse.ArgumentParser(
        description=__doc__,
        formatter_class=argparse.RawDescriptionHelpFormatter,
    )
    sub = p.add_subparsers(dest="cmd", required=True)

    g = sub.add_parser("gen-wavs", help="Generate deterministic test WAVs.")
    g.add_argument("--out", required=True, help="Output directory.")
    g.set_defaults(func=cmd_gen_wavs)

    pr = sub.add_parser("gen-project", help="Emit Wwise 2013 project template.")
    pr.add_argument("--out", help="Output file (default: rs_corpus_2013_q4.xml).")
    pr.add_argument("--quality", type=int, default=4, help="Quality factor (default 4).")
    pr.add_argument(
        "--plugin-id", type=int, default=4, help="Vorbis plugin id (default 4)."
    )
    pr.set_defaults(func=cmd_gen_project)

    ec = sub.add_parser("emit-command", help="Print Wwise headless CLI invocation.")
    ec.add_argument("project_dir", help="Wwise project directory.")
    ec.add_argument("--quality", type=int, default=4, help="Quality factor (default 4).")
    ec.add_argument("--plugin-id", type=int, default=4, help="Vorbis plugin id (default 4).")
    ec.add_argument("--both", action="store_true", help="Emit q4 and q6 variants.")
    ec.set_defaults(func=cmd_emit_command)

    return p


def main(argv: list[str] | None = None) -> int:
    parser = build_parser()
    args = parser.parse_args(argv)
    return args.func(args)


if __name__ == "__main__":
    sys.exit(main())
