# Public interface

The Python package has one encoding function, one decoding function, six
exported value types, and one public error type:

```python
from wwise_wem import (
    EncodeResult,
    EncodeStats,
    PcmBuffer,
    RawPcm,
    WwiseProfile,
    WwiseVersion,
    WwiseWemError,
    decode,
    encode,
)
```

`decode` also returns a value — the decode result documented under
[`decode`](#decode) — and that value's type is not itself a root export: the
caller never sees a session object, only the function and what it returns.
Internal module paths, profile loaders, registry objects, adapters, the decode
result's class, and the native binding are implementation details.

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

## `decode`

```python
result = decode(source)
```

`source` accepts four forms: `bytes`, `bytearray` and `memoryview` hold the WEM
itself, and `str` or `os.PathLike[str]` names a file, read whole at call time —
a file-access error surfaces there with its standard `OSError` subclass, such as
`FileNotFoundError`. Any other type raises `TypeError`.

```python
from wwise_wem import decode

result = decode(wem_bytes)   # a rejection of the container header is raised HERE
result.channels              # geometry is readable before iteration
result.sample_rate
result.total_frames          # the container's dw_total_pcm_frames
for block in result:         # interleaved f32, ±1.0 full scale
    ...
```

`decode` is a plain function returning an iterable result object, **not a
generator function**: a generator function's body does not run until the first
`next()`, which would defer every rejection to iteration time, against the
call-time error behaviour this facade documents. The call opens the native
decode session and consumes the container's header region, so the geometry is
resolved before the first block.

- `channels` and `sample_rate` are the kernel's one-time header announcement for
  the container; `total_frames` is the container's own `dwTotalPCMFrames`.
- Each iteration step yields one block — a list of interleaved f32 samples at
  ±1.0 full scale, at most 1024 frames long. A decoded sample outside ±1.0 is
  passed through rather than clipped: the library owns no clipping policy.
- A successful iteration delivers exactly `total_frames` frames.
- The result is a one-shot iterator, like a generator, and owns one native
  decode session. `result.close()` releases it deterministically, so
  `contextlib.closing(result)` works; otherwise collection releases it. The
  geometry already announced stays readable after `close()`.
- A **rejection** of the container framing, of the setup packet, or of the
  geometry it names is raised by `decode` itself, because nothing has been
  decoded yet. A rejection only the audio packets can produce is raised from the
  iteration step that reaches it, *after* the blocks the earlier packets
  completed have been handed over — the kernel delivers the prefix a refusal
  completed and then reports its code — and the result is exhausted afterwards.
- `decode` takes no `profile` and no `quality`: a WEM is self-describing and the
  decoder reads whatever the bitstream says.
- `decode` is also reachable from the command line: `--decode` writes the PCM it
  returns as an uncompressed signed-16 WAV. See [CLI](#cli).

## Errors

A rejection made by the kernel reaches the caller as `WwiseWemError`, a
`ValueError` subclass carrying the kernel's stable error class in `.code`, the
kernel's diagnostic text in `.message` (and in `str(error)`), and the original
kernel error as `__cause__`:

| `code` | Meaning |
|---|---|
| `PROFILE_NOT_FOUND` | no installed configuration satisfies the selection (or more than one does) |
| `GEOMETRY_MISMATCH` | PCM channels/sample rate differ from the selected profile |
| `INPUT_TOO_SHORT` | fewer PCM frames than the kernel's 4096-frame minimum |
| `FORMAT_UNSUPPORTED` | the selection names a generation this revision does not support |
| `STATE_ERROR` | a request outside the lifecycle, or input outside the accepted domain |
| `INPUT_MALFORMED` | the input's own bytes do not parse: the container framing, the setup packet, or an audio packet (decode only) |
| `INTERNAL` | a kernel configuration or assembly fault |

These are the same classes every cross-language shell maps from the kernel, and
the codes are stable. `INPUT_MALFORMED` is the decode surface's malformed-input
class and is never reported by an encode call (`decode`'s counterpart,
`FORMAT_UNSUPPORTED`, is the class a container that parses but names a
configuration this build does not carry is refused with). Because
`WwiseWemError` subclasses `ValueError`, a caller that only tests
`except ValueError` keeps working unchanged. The facade's own
input checks (a source of the wrong type, a buffer the selection does not
describe, fewer than 4096 frames) raise their own `ValueError` before the kernel
is reached.

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

`EncodeResult.data` contains the completed WEM bytes and
`EncodeResult.stats` contains an immutable `EncodeStats`; `len(result)` is the
container's byte length and `result.write_to(path)` persists it, so a caller
need not reach into `data` to save a result. `EncodeStats.to_dict()` returns
the stable JSON-ready statistics fields: `pcm_frames`, `channels`,
`audio_packets`, `short_packets`, `long_packets`.

Every one of those is an observation the caller cannot recompose. The
container's byte length is not among them — it is `len(result)` — and neither
is a label naming the selected profile, which is the `WwiseProfile` the caller
itself passed: selection is exactly one profile per geometry or a rejection,
so the selected profile is a pure function of the geometry the caller already
knows. No surface returns a digest of the container either; a caller that
wants one computes it from the bytes it received.

For `tests/fixtures/input.wav`, the output is 205 audio packets (77 short and
128 long) and 108,771 output bytes, byte-identical to the committed reference
container `tests/fixtures/reference.wem`.

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

Decoding:

```python
from contextlib import closing
from wwise_wem import decode

with closing(decode("output.wem")) as result:
    print(result.channels, result.sample_rate, result.total_frames)
    pcm = [sample for block in result for sample in block]
```

## CLI

```text
wwise-wem INPUT.wav --output OUTPUT.wem [OPTIONS]
wwise-wem INPUT.wem --decode --output OUTPUT.wav
python -m wwise_wem INPUT.wav --output OUTPUT.wem [OPTIONS]
python -m wwise_wem INPUT.wem --decode --output OUTPUT.wav
```

Encoding is the default direction. Supported options are `--quality`,
`--wwise-version`, `--channels`, `--sample-rate`, and required `--output`.
`--wwise-version` takes the short label `2013` or the full generation `2013.2`
and defaults to `2013`; it forms the whole selection together with the WAV
geometry, which the CLI reads from the input. An unrecognized value is rejected
as an argument error, without reading the WAV. The CLI parses the WAV once,
applies optional geometry assertions to that `PcmBuffer`, and passes the same
buffer and the selection to `encode`.

`--decode` reverses the direction: the positional argument is then a WEM and the
output is a PCM WAV. It is a flag rather than a subcommand because this command
line is positional — a `decode` subcommand would take the argv slot a file named
`decode` already occupies, and would change invocations that work today. Every
encode invocation is unchanged, including one naming such a file, and both front
ends take the same flag with the same meaning (the Rust binary
`crates/wem-core/src/bin/wwise-wem.rs` mirrors it). The encoder's options are
refused in decode mode rather than ignored: a WEM is self-describing, so there is
nothing for them to select or assert.

The WAV is uncompressed signed-16 PCM: signed-16 is the form the encode
direction reads, so a decoded file goes straight back into the same command line.
The samples are `decode`'s own output, mapped by the package's
float-to-signed-16 rule (`adapters/sample_conversion.float_to_int16`), which the
Rust command line applies too — the two write the same bytes for the same WEM.
The file is written only once the session has finished: a session the decoder
refuses part way through has delivered a *prefix* of the declared frame count, so
the command line reports the refusal, naming how much of the stream it left
behind, and creates no output at all.

## Execution path

Every byte-producing Python call uses the in-package native extension
`wwise_wem._core`, which carries the complete profile calibration set compiled
in as Rust constants. Callers never provide a profile name, a profile directory,
or an environment variable, and there is no packaged profile data to point at:
the reference oracle reads the same carrier through the extension's
`profile_tables()` hand-off. The pure-Python implementation under `reference/`
is a development-time oracle only.
