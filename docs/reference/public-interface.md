# Public interface

The Python package has one encoding function and six public value types:

```python
from wwise_wem import (
    EncodeResult,
    EncodeStats,
    PcmBuffer,
    RawPcm,
    WwiseProfile,
    WwiseVersion,
    encode,
)
```

Internal module paths, profile loaders, registry objects, adapters, and the
native binding are implementation details.

## `encode`

```python
result = encode(source, *, profile=None, quality=None)
```

`source` accepts exactly three forms:

- `str` or `os.PathLike[str]`: a RIFF/WAVE file. The reader inspects the file
  header, independent of the filename extension. It supports signed 16-bit PCM,
  signed 24-bit PCM, and 32-bit IEEE float PCM.
- `PcmBuffer`: channel-major samples already in the encoder's signed-16 domain.
- `RawPcm`: unframed bytes with explicit sample rate, channel count, and sample
  format.

Plain `bytes` are rejected because raw PCM geometry cannot be inferred safely.
`profile` is a `WwiseProfile(version, channels, sample_rate)` selection
(see [Profile selection](#profile-selection)); with `profile=None` the encoder
selects the installed generation's configuration for the input channel count
and sample rate. Profile names, profile directories, and geometry-only lookups
are not part of the interface. `quality` is an optional finite profile
interpolation value.

All inputs require at least 4096 frames. Unsupported source or field types raise
`TypeError`; invalid values, unsupported geometry, profile mismatches, and kernel
configuration errors raise `ValueError`. File access errors retain their standard
`OSError` subclasses, such as `FileNotFoundError`.

## Profile selection

`profile` is a `WwiseProfile(version, channels, sample_rate)` selection: one
Wwise generation plus the PCM geometry to encode. It is handed straight to the
kernel, which resolves it against the configurations compiled into the
extension, so an unsatisfiable selection is an error, never a substituted
default — and there is exactly one authority on which configuration a
selection denotes.

```python
from wwise_wem import WwiseProfile, WwiseVersion, encode

selection = WwiseProfile(WwiseVersion.WWISE2013, 6, 44100)
result = encode("input.wav", profile=selection)
```

`WwiseVersion.WWISE2013` is the only registered generation. `code` is its
stable cross-language code, `label` the short spelling (`"2013"`) and
`generation` the full one (`"2013.2"`); `WwiseVersion.ALL` lists every
registered generation. `WwiseVersion.parse("2013")` accepts the short label or
the full generation, while `from_code` and `from_generation` decode the other
two spellings. `WwiseProfile` is immutable, hashable, and compares by value;
`.version`, `.channels` and `.sample_rate` are read-only.

A selection no installed profile satisfies raises `ValueError`, as do a
non-positive channel count or sample rate and an unrecognized version spelling.
A selection that more than one installed configuration satisfies is rejected as
ambiguous rather than satisfied by a first match.

## Input types

`PcmBuffer(sample_rate, channels)` stores immutable, equal-length channel rows.
Every sample must equal an integer signed-16 value divided by 32768. This keeps
the boundary exact while allowing callers to hold samples as Python floats.

`RawPcm(data, sample_rate, channels, sample_format)` accepts bytes-like data and
one of these explicit format strings:

| `sample_format` | Payload |
|---|---|
| `"s16le"` | signed 16-bit little-endian PCM |
| `"s24le"` | signed 24-bit little-endian PCM |
| `"f32le"` | 32-bit IEEE float little-endian PCM |

24-bit and float input is deterministically converted into the signed-16 domain.
The bit-exact Wwise guarantee applies to signed-16 input; converted input is
deterministic and matches the repository's reference encoder after conversion.

## Result types

`EncodeResult.data` contains the completed WEM bytes, `EncodeResult.stats`
contains an immutable `EncodeStats`, and `EncodeResult.sha256` returns the
lowercase SHA-256 digest. `len(result)` is the container's byte length and
`result.write_to(path)` persists it, so a caller need not reach into `data` to
save a result. `EncodeStats.to_dict()` returns the stable JSON-ready
statistics fields.

For `tests/fixtures/input.wav`, the output is 205 audio packets (77 short and
128 long), 108,771 output bytes, and SHA-256
`17851d26c6210b85e498ae0452d2562d7b9e2c3e9e795c459656b9c9d8d35247`.

Both installed profiles are byte-exact against their paired builds. The 2ch
result, its corpora, and the limits of that evidence are recorded in
[`../findings/2ch-byte-exactness.md`](../findings/2ch-byte-exactness.md).

## Examples

File input:

```python
from pathlib import Path
from wwise_wem import encode

result = encode("input.wav")
Path("output.wem").write_bytes(result.data)
```

In-memory samples:

```python
from wwise_wem import PcmBuffer, encode

pcm = PcmBuffer(sample_rate=48000, channels=(left, right))
result = encode(pcm)
```

Raw PCM:

```python
from wwise_wem import RawPcm, encode

source = RawPcm(raw_bytes, 48000, 2, "s16le")
result = encode(source)
```

## CLI

```text
wwise-wem INPUT.wav --output OUTPUT.wem [OPTIONS]
python -m wwise_wem INPUT.wav --output OUTPUT.wem [OPTIONS]
```

Supported options are `--quality`, `--wwise-version`, `--channels`,
`--sample-rate`, `--expect-sha256`, and required `--output`.
`--wwise-version` takes the short label `2013` or the full generation `2013.2`
and defaults to `2013`; it forms the whole selection together with the WAV
geometry, which the CLI reads from the input. An unrecognized value is rejected
as an argument error, without reading the WAV. The CLI parses the WAV once,
applies optional geometry assertions to that `PcmBuffer`, and passes the same
buffer and the selection to `encode`.

## Execution path

Every byte-producing Python call uses the in-package native extension
`wwise_wem._core`, which carries the complete checksummed profile bundle at
compile time. Callers never provide a profile name, a profile directory, or an
environment variable. The package manifests remain metadata for inspection and
for the development-tree resolution used by the oracle and the tooling; the
native runtime does not assemble a profile from fragments. The pure-Python
implementation under `reference/` is a development-time oracle only.
