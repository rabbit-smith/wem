# Public interface contract

The Python package has one encoding function and four public value types:

```python
from wwise_wem import EncodeResult, EncodeStats, PcmBuffer, RawPcm, encode
```

Internal module paths, profile loaders, registry objects, adapters, and the
native binding are implementation details.

## `encode`

```python
result = encode(source, profile=None, quality=None)
```

`source` accepts exactly three forms:

- `str` or `os.PathLike[str]`: a RIFF/WAVE file. The reader inspects the file
  header, independent of the filename extension. It supports signed 16-bit PCM,
  signed 24-bit PCM, and 32-bit IEEE float PCM.
- `PcmBuffer`: channel-major samples already in the encoder's signed-16 domain.
- `RawPcm`: unframed bytes with explicit sample rate, channel count, and sample
  format.

Plain `bytes` are rejected because raw PCM geometry cannot be inferred safely.
With no `profile`, the encoder selects the installed profile matching the input
channel count and sample rate. `profile="wwise2013-6ch-44100"` or
`profile="wwise2013-2ch-48000"` selects one explicitly. `quality` is an optional
finite profile interpolation value.

All inputs require at least 4096 frames. Unsupported source or field types raise
`TypeError`; invalid values, unsupported geometry, profile mismatches, and kernel
configuration errors raise `ValueError`. File access errors retain their standard
`OSError` subclasses, such as `FileNotFoundError`.

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
lowercase SHA-256 digest. `EncodeStats.to_dict()` returns the stable JSON-ready
statistics fields.

For `tests/fixtures/input.wav`, the contract is 205 audio packets (77 short and
128 long), 108,771 output bytes, and SHA-256
`17851d26c6210b85e498ae0452d2562d7b9e2c3e9e795c459656b9c9d8d35247`.

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

Supported options are `--profile`, `--quality`, `--wwise-version 2013`,
`--channels`, `--sample-rate`, `--expect-sha256`, and required `--output`.
The CLI parses the WAV once, applies optional geometry assertions to that
`PcmBuffer`, and passes the same buffer to `encode`.

## Execution path

Every byte-producing Python call uses the in-package native extension
`wwise_wem._core`. Packaged profile manifests select one complete, checksummed
bundle; the runtime does not assemble a profile from fragments. The pure-Python
implementation under `reference/` is a development-time oracle only.
