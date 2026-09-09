# Public interface contract

This page defines the supported user-facing contract for the 0.x encoder.
Implementation modules may be reorganized without preserving their current
functions or classes. Only the API and CLI described here are compatibility
targets.

## Supported Python API

Import supported names from the package root:

```python
from wwise_wem import (
    ContainerMetadata,
    Encoder,
    EncoderProfile,
    EncodeResult,
    EncodeStats,
    PacketResult,
    PcmBuffer,
    ProfileKey,
    ProfileRegistry,
    SetupConfig,
    WwiseVorbisProfile,
    encode_pcm_wav,
    encode_raw_pcm,
    encode_wav,
    load_wem_profile,
    read_pcm16_wav,
    read_pcm_wav,
    read_raw_pcm,
    resolve_wem_profile,
)
```

### Encode typed PCM

`Encoder` is the canonical state-owning entry point for PCM already held in
memory. Construct it from an installed profile, then pass an immutable
`PcmBuffer` to `encode_pcm()`:

```python
profile = load_wem_profile("wwise2013-6ch-44100")
encoder = Encoder(profile)
pcm = PcmBuffer(sample_rate=44100, channels=channel_rows)
result = encoder.encode_pcm(pcm)
```

Each `encode_pcm()` call creates an independent analysis session and returns
an immutable `EncodeResult`. The PCM geometry must match the selected profile.
`Encoder` owns the validated profile, setup and codebooks; callers do not pass
mutable analysis state into the encoder.

The typed direction for new integrations is `Encoder.encode_pcm()` for
in-memory samples and `encode_wav()` for file input. Both return
`EncodeResult`.

### Encode a WAV

```python
result = encode_wav(
    wav_path,
    profile=None,
)
```

- `wav_path` is a `pathlib.Path` to an uncompressed signed-16 PCM WAV.
- With no `profile`, the encoder resolves a profile from the WAV channel
  count and sample rate.
- `profile="wwise2013-6ch-44100"` selects the current profile explicitly.
- The return value is an immutable `EncodeResult`.

`result.data` contains the completed WEM bytes, `result.stats` is an
`EncodeStats`, and `result.sha256` computes the hexadecimal SHA-256 digest of
the completed bytes.

For the current golden input, `result.stats` has these exact public values:

```python
EncodeStats(
    pcm_frames=139398,
    channels=6,
    audio_packets=205,
    short_packets=77,
    long_packets=128,
    bytes=108771,
    metadata_source="profile:wwise2013-6ch-44100",
)
```

`audio_packets` excludes the setup packet.

### Encode extended PCM inputs

Two additional encoder entries accept PCM beyond the signed-16 golden path.
Both convert at the adapter boundary, so the encoder consumes only in-domain
samples:

```python
result = encode_pcm_wav(
    wav_path,
    profile=None,
)

result = encode_raw_pcm(
    data,
    sample_rate=44100,
    channels=6,
    bits_per_sample=24,
    profile=None,
)
```

- `encode_pcm_wav` reads a RIFF/WAVE file in one of the supported formats:
  format tag 1 (PCM) with 16-bit or 24-bit samples, or format tag 3 (IEEE
  float) with 32-bit samples. 16-bit files behave exactly like
  `encode_wav`. The geometry is read from the file; the profile resolution
  rules are the same.
- `encode_raw_pcm` reads an unframed little-endian PCM byte string with the
  full geometry supplied explicitly: `bits_per_sample` is 16 (signed), 24
  (signed), or 32 (IEEE-754 float32).
- `read_pcm_wav(path)` and `read_raw_pcm(data, sample_rate=..., channels=..., bits_per_sample=...)`
  return the same `PcmBuffer` values without encoding, for use with
  `Encoder.encode_pcm()`. Both are lazy root exports.

Conversion rules (deterministic; pure integer/fixed-point arithmetic with
explicit saturation, no transcendental calls):

| Source format | Rule |
|---|---|
| 16-bit PCM | No conversion; the historical `value / 32768.0` domain is used as-is. |
| 24-bit signed PCM | Round-to-nearest, ties away from zero: `s >= 0` maps to `(s + 128) >> 8`, `s < 0` to `-((-s + 128) >> 8)`; then saturate to `[-32768, 32767]`. The upper band `s >= 8388480` rounds to 32768 and saturates to 32767; `-8388608` maps exactly to `-32768`. |
| 32-bit IEEE float | Reject non-finite values (NaN, +/-Inf) with `ValueError`; scale by `* 32768.0`; round-to-nearest, ties away from zero; then saturate to `[-32768, 32767]`. `1.0` saturates to 32767; `-1.0` maps exactly to `-32768`. |

Rejection conditions (all at the adapter boundary, before any encode
state is created):

| Entry point | Condition | Error |
|---|---|---|
| `encode_pcm_wav` / `read_pcm_wav` | Not a RIFF/WAVE container, missing or malformed `fmt`/`data` chunk | `ValueError` |
| | Bits per sample not a whole byte count, non-positive geometry | `ValueError` |
| | Format outside the three supported tags/widths | `ValueError` |
| | Data not aligned to whole frames, or zero frames | `ValueError` |
| | Non-finite 32-bit float sample | `ValueError` |
| `encode_raw_pcm` / `read_raw_pcm` | `data` not bytes-like | `TypeError` |
| | `sample_rate` / `channels` not a positive integer | `TypeError` / `ValueError` |
| | `bits_per_sample` not one of 16, 24, 32 | `ValueError` |
| | Byte length not a multiple of `channels * bytes_per_sample` | `ValueError` |
| | Zero frames, or non-finite 32-bit float sample | `ValueError` |
| both | Profile geometry mismatch, or fewer than 4096 frames | `ValueError` (encoder rules) |

Converted inputs (24-bit, float32) are deterministic: encoding the same
bytes always produces the same WEM, and the native kernel and the reference
oracle (a test asset that verifies it) produce identical bytes for them. They
are **not** promised to be
bit-exact against Wwise's own import of the same audio: the project's
bit-exact guarantee covers the signed-16 path only.

### Typed value models

The package root also exports immutable adapter-boundary models:

- `PcmBuffer(sample_rate, channels)` validates equal channel lengths and
  exposes `channel_count` and `frame_count`;
- `ContainerMetadata` represents the fixed Wwise Vorbis fmt66 fields and
  converts with `from_fmt_dict()` / `to_fmt_dict()`;
- `SetupConfig` holds setup bytes, channel count, book IDs and a recursively
  read-only parsed view;
- `PacketResult` holds immutable packet bytes and the short/long mode triple;
- `EncodeStats` converts with `from_legacy_dict()` / `to_legacy_dict()`. The
  legacy dictionary form carries no provenance entry: there is a single
  execution path (the native kernel), so nothing records which engine ran.

These types are stable public adapters. Internal analysis buffers and
transform state remain outside the public contract.

### Legacy Python compatibility

`read_pcm16_wav(path)` remains available as a compatibility helper that
returns the historical ``(rate, frames, channel_rows)`` tuple in the
``value / 32768.0`` float domain. It is a read-only input adapter, not an
encoder entry point. `Encoder` and this helper are loaded lazily. Importing
`wwise_wem` alone does not import `application.encoder`, transform, floor,
residue or analysis-session implementation modules.

### Resolve profiles

```python
profile = resolve_wem_profile(channels=6, sample_rate=44100)
same = load_wem_profile("wwise2013-6ch-44100")
```

Both calls return an `EncoderProfile`; `WwiseVorbisProfile` remains its
compatibility alias. `PROFILE_REGISTRY` is a read-only `ProfileRegistry` with
`get()`, `resolve()` and `list()` operations. `ProfileKey(6, 44100)` preserves
the legacy geometry constructor while carrying the complete generation,
channel-layout and quality/setup identity. The 6ch profile shown here is one
of two installed profiles (the other is `wwise2013-2ch-48000`);
`PROFILE_REGISTRY.list()` reports both. The supported profile exposes:

- `name == "wwise2013-6ch-44100"`
- `channels == 6`
- `sample_rate == 44100`
- `block_sizes == (256, 2048)`
- `setup_packet()` returning the checked 201-byte setup packet
- `runtime_manifest()` returning the installed profile-manifest identity for that profile

`PROFILES` is the registry's read-only name view. Geometry-only resolution
reports ambiguity when multiple complete identities share the same channel
count and sample rate; callers can then resolve with a complete `ProfileKey`.

The representation of fmt metadata, table paths, registries and packaged
resources is an implementation detail.

### Optional quality factor

`load_wem_profile` and `resolve_wem_profile` each accept an optional
keyword `quality`:

```python
profile = load_wem_profile("wwise2013-6ch-44100", quality=4.0)
profile = resolve_wem_profile(6, 44100, quality=4.0)
```

With `quality` omitted (the default), the behavior is identical to the
historical signature: the cached registry instance is returned unchanged and
`profile.quality` is `None`. With a quality value, a copy of the profile
carrying that factor is returned; the registered profile is never mutated.
`quality` must be a finite number (a `ValueError` otherwise).

Quality is a profile-internal psychoacoustic interpolation parameter: a
profile optionally ships an `analysis/quality-curves.json` (schema
`wem.quality-curves.v1`) that interpolates selected psychoacoustic fields
along a fixed breakpoint table. Requesting a quality from a profile that has
no such resource is a configuration error (`ValueError`); the installed
built-in profile does not carry quality curves, so its encode output is
unchanged. A `quality` value outside the recorded control points is clamped
to the nearest control point, and the assembly records that the value was
extrapolated (honesty flag).

The encode entry points (`encode_wav`, `encode_pcm_wav`, `encode_raw_pcm`)
and the CLI (`--quality`) forward an optional `quality` to profile selection;
with no quality the behavior is identical to before.

## Supported CLI

The command remains a single encode command in 0.x:

```text
python -m wwise_wem INPUT.wav --output OUTPUT.wem [OPTIONS]
wwise-wem INPUT.wav --output OUTPUT.wem [OPTIONS]
```

Supported options are:

| Option | Contract |
|---|---|
| `--output PATH` | Required destination WEM. |
| `--profile wwise2013-6ch-44100` | Explicit built-in profile. |
| `--quality N` | Optional psychoacoustic quality factor forwarded to profile selection. |
| `--wwise-version 2013` | Assert the supported Wwise generation. |
| `--channels N` | Assert the WAV channel count before encoding. |
| `--sample-rate HZ` | Assert the WAV sample rate before encoding. |
| `--expect-sha256 HASH` | Fail if the completed WEM digest differs. |
| `--help` | Print usage and the supported options. |

If `--profile` is omitted, profile selection is automatic. A failed header
assertion does not create the output file.

## Errors

Python calls raise `ValueError` for user input or configuration errors,
including:

- unknown profile name;
- unsupported channel-count/sample-rate geometry;
- a WAV that is not uncompressed signed-16 PCM (this applies to
  `encode_wav`, which stays signed-16 only; `encode_pcm_wav` documents its
  own supported-format rejection above);
- an unsupported WAV format tag/width (extended entries), non-finite float
  samples, misaligned or empty raw PCM payloads, malformed raw-PCM geometry
  (see the rejection table above);
- WAV geometry that differs from the selected profile metadata;
- a packaged built-in setup resource whose checksum differs from its profile
  declaration.

CLI argument/header errors exit nonzero and write a diagnostic to stderr.
Exact prose and traceback formatting are not part of the contract.

## Bit-exact acceptance invariant

For `tests/fixtures/input.wav`, automatic and explicit profile selection must
produce identical results:

- 205 audio packets: 77 short and 128 long;
- 108,771-byte WEM;
- byte-identical to `tests/fixtures/reference.wem`;
- SHA-256
  `17851d26c6210b85e498ae0452d2562d7b9e2c3e9e795c459656b9c9d8d35247`.

These are compatibility gates for refactoring, not general promises that all
inputs have the same size or packet counts.

The bit-exact guarantee is scoped to signed-16 PCM input. Extended input
forms (24-bit PCM, 32-bit IEEE-float PCM, and raw PCM in any of the three
supported depths) are mapped into the signed-16 domain by the documented
adapter conversion rules above before encoding; those conversions are
deterministic, but the project does not promise that
the resulting WEM bytes match Wwise's own import path for the same audio.

## Open questions

### Reference padding behaviour unverified

Inputs shorter than 4096 frames are rejected explicitly (`ValueError`,
equivalent to the `INPUT_TOO_SHORT` contract error) on every entry point,
including the extended entries. How the Wwise reference implementation pads
such inputs is unverified; no padding behaviour is implemented until an
official Wwise reference sample for a short input is available. Until then,
the explicit rejection is the only supported behaviour for short inputs.

## Execution path (single)

The facade has a single execution path: byte-producing calls
(`Encoder.encode_pcm()`, `encode_wav()`, and the CLI) run on the in-package
native extension `wwise_wem._core` (built from `crates/wem-python` by the
maturin build backend and embedded in the wheel).  There is no engine
selection, no environment variable, and no fallback: the extension is a
required runtime asset, and a missing one surfaces as the ordinary
`ImportError` that the Python import machinery raises for
`wwise_wem._core` — the facade invents no second way for it to fail.

The pure-Python reference implementation ships only with the development
source tree (`reference/wwise_wem_reference`, importable when `reference/`
is on `PYTHONPATH`); the test suites import it directly as the oracle that
checks the kernel byte-for-byte.

Input-domain rules:

- The native kernel consumes integer signed-16 samples. Float PCM is
  accepted only when every sample is exactly an integer multiple of
  `1/32768` within the signed-16 range (the domain produced by
  `read_pcm16_wav` and `read_pcm16`). An out-of-domain sample raises a
  plain `ValueError` from the facade, before the kernel is reached.

`load_wem_profile`, `resolve_wem_profile`, and `ProfileRegistry` are
metadata-only paths and always run in pure Python; they never touch the
kernel.

## Deliberately outside the public contract

Do not depend on internal modules such as `analysis.dsp.transform`,
`analysis.psychoacoustics.pipeline`, `analysis.session`, `vorbis.floor`,
`vorbis.residue`, `vorbis.packet_encoder`, scheduler modules, internal
diagnostics, table loaders or parser diagnostics. Their names, signatures,
dataclasses, dictionary layouts and module locations may change during
architecture work.

Also outside the contract:

- package-global registry dictionaries;
- direct table/resource filesystem paths;
- internal verification functions;
- module `main()` functions other than the installed/package CLI;
- private float and bit-packing helpers;
- intermediate analysis buffers and state objects.

New public API must first be documented here and covered by a focused public
contract test.
