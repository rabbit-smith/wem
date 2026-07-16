# Public interface contract

This page defines the supported user-facing contract for the 0.x encoder.
Implementation modules may be reorganized without preserving their current
functions or classes. Only the API and CLI described here are compatibility
targets.

## Supported Python API

Import supported names from the package root:

```python
from wwise2013_wem import (
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
    encode_wav,
    load_wem_profile,
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
`EncodeResult`; the tuple-returning functions below remain compatibility
adapters.

### Encode a WAV

```python
result = encode_wav(
    wav_path,
    template=None,
    profile=None,
)
```

- `wav_path` is a `pathlib.Path` to an uncompressed signed-16 PCM WAV.
- With neither `template` nor `profile`, the encoder resolves a profile from
  the WAV channel count and sample rate.
- `profile="wwise2013-6ch-44100"` selects the current profile explicitly.
- `template=path` enables the compatibility metadata adapter described below.
- `template` and `profile` are mutually exclusive.
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

### Typed value models

The package root also exports immutable adapter-boundary models:

- `PcmBuffer(sample_rate, channels)` validates equal channel lengths and
  exposes `channel_count` and `frame_count`;
- `ContainerMetadata` represents the fixed Wwise Vorbis fmt66 fields and
  converts with `from_fmt_dict()` / `to_fmt_dict()`;
- `SetupConfig` holds setup bytes, channel count, book IDs and a recursively
  read-only parsed view;
- `PacketResult` holds immutable packet bytes and the short/long mode triple;
- `EncodeStats` converts with `from_legacy_dict()` / `to_legacy_dict()`;
- `EncodeResult` converts with `from_legacy_tuple()` / `to_legacy_tuple()`.

These types are stable public adapters. Internal analysis buffers and
transform state remain outside the public contract.

### Legacy Python compatibility

The previous tuple API remains available:

```python
from wwise2013_wem import encode_wav_to_wem, read_pcm16_wav

encoded, stats = encode_wav_to_wem(wav_path, template=None, profile=None)
```

`stats` is the legacy dictionary form of `EncodeStats`. `Encoder` and these
compatibility functions are loaded lazily. Importing `wwise2013_wem` alone
does not import `application.encoder`, transform, floor, residue or
analysis-session implementation modules.

### Resolve profiles

```python
profile = resolve_wem_profile(channels=6, sample_rate=44100)
same = load_wem_profile("wwise2013-6ch-44100")
```

Both calls return an `EncoderProfile`; `WwiseVorbisProfile` remains its
compatibility alias. `PROFILE_REGISTRY` is a read-only `ProfileRegistry` with
`get()`, `resolve()` and `list()` operations. `ProfileKey(6, 44100)` preserves
the legacy geometry constructor while carrying the complete generation,
channel-layout and quality/setup identity. The supported profile exposes:

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

## Template compatibility contract

A template is a container-metadata adapter, not an encoder profile. It may
provide the RIFF byte order, Wwise Vorbis `fmt` values, seek-table bytes and
extra chunks that precede `data`. Per-file size, offset, packet-maximum and PCM
frame fields are recomputed for the newly encoded stream.

The template setup packet is used to identify and validate an installed exact
profile. The encoder first resolves the template's channel-count/sample-rate
geometry, then requires the setup packet to match that profile byte-for-byte
and by SHA-256. Supplying a WEM with the right geometry but a changed or unknown
setup packet does not create a new profile and does not select approximate
tables. Once validated, analysis, mode selection, floor and residue generation
use the installed profile implementation; existing audio packets in the
template are not encoder input.

Consequently, the compatibility path accepts the current reference WEM because
its 6-channel/44.1-kHz setup is the installed
`wwise2013-6ch-44100` setup. `result.stats.metadata_source` (or
`stats["metadata_source"]` through the legacy API) retains the `template:PATH`
label so callers can distinguish which metadata path they selected.

Automatic and named built-in profile selection are template-free. They read
the packaged profile setup resource and constants, and do not open or inspect a
reference WEM.

## Supported CLI

The command remains a single encode command in 0.x:

```text
python -m wwise2013_wem INPUT.wav --output OUTPUT.wem [OPTIONS]
wwise2013-wem INPUT.wav --output OUTPUT.wem [OPTIONS]
```

Supported options are:

| Option | Contract |
|---|---|
| `--output PATH` | Required destination WEM. |
| `--profile wwise2013-6ch-44100` | Explicit built-in profile. |
| `--template PATH` | Compatibility metadata/setup source. |
| `--wwise-version 2013` | Assert the supported Wwise generation. |
| `--channels N` | Assert the WAV channel count before encoding. |
| `--sample-rate HZ` | Assert the WAV sample rate before encoding. |
| `--expect-sha256 HASH` | Fail if the completed WEM digest differs. |
| `--help` | Print usage and the supported options. |

`--template` and `--profile` are mutually exclusive. If both are omitted,
profile selection is automatic. A failed header assertion does not create
the output file.

## Errors

Python calls raise `ValueError` for user input or configuration errors,
including:

- unknown profile name;
- unsupported channel-count/sample-rate geometry;
- a WAV that is not uncompressed signed-16 PCM;
- simultaneous template and profile selection;
- a template that is not a Wwise Vorbis WEM with a setup packet;
- WAV geometry that differs from selected template/profile metadata;
- template geometry for which no exact profile is installed;
- a template setup packet whose bytes or SHA-256 differ from the installed
  profile setup for that geometry;
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
