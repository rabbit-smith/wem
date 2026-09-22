# Usage

How to install the encoder, encode a file, and call it from each supported
language. For the exact API see
[`../reference/public-interface.md`](../reference/public-interface.md).

## Install

From a wheel, nothing but Python 3.10+ is required — the native kernel is
embedded in the package:

```bash
pip install wwise_wem-<version>-<platform>.whl
```

From this source tree, an editable install builds the kernel with maturin and
needs a Rust toolchain on `PATH`:

```bash
python3 -m venv .venv
. .venv/bin/activate
pip install -e .
```

`make native` (a one-line `maturin develop`) does the same thing inside an
active virtual environment. The install surface is the facade plus the embedded
kernel; the single execution path it follows is in
[`../reference/public-interface.md`](../reference/public-interface.md#execution-path).

## Encode

```bash
wwise-wem input.wav --output output.wem
```

```python
from pathlib import Path
from wwise_wem import encode

result = encode("input.wav")
Path("output.wem").write_bytes(result.data)
print(len(result), result.stats.audio_packets)
```

The configuration is selected from the WAV geometry for the installed Wwise
generation. To assert that geometry instead of trusting it:

```bash
wwise-wem tests/fixtures/input.wav --output output.wem \
  --channels 6 --sample-rate 44100
```

To check the result against the reference container, compare the bytes —
`cmp` says exactly where two files first differ, which a digest of each cannot:

```bash
cmp output.wem tests/fixtures/reference.wem && echo "byte-identical"
```

## Inputs

| Form | Accepted | Notes |
| --- | --- | --- |
| `str` / `os.PathLike[str]` | RIFF/WAVE file | Format is read from the header, not the extension: signed 16-bit, signed 24-bit, or 32-bit IEEE float PCM |
| `PcmBuffer(sample_rate, channels)` | channel-major rows already in the signed-16 domain | Every sample must equal an integer signed-16 value divided by 32768 |
| `RawPcm(data, sample_rate, channels, sample_format)` | unframed bytes | `sample_format` is `"s16le"`, `"s24le"`, or `"f32le"` |

24-bit and float input is converted deterministically at the adapter boundary.
The bit-exact guarantee applies to signed-16 input; converted input is
deterministic and matches the reference implementation after conversion.

Plain `bytes` are rejected: raw PCM geometry cannot be inferred safely. Every
input needs at least 4096 frames.

## Preparing input

The byte-exact claim is stated for a RIFF/WAVE container holding one interleaved
signed-16 stream at the geometry you are encoding for; the reader also converts
signed 24-bit and float samples itself, by the rules in [Inputs](#inputs).
Anything else — another wrapper, another sample representation, a compressed
source, a source at another geometry — is converted before the library sees it,
and that conversion is the caller's step: this library reads a file, it does not
decode one.

### What the reader accepts

The sample representation is read from the `fmt ` chunk, never from the filename
extension:

| `fmt ` chunk | Read as |
| --- | --- |
| format tag 1, 16 bits per sample | signed 16-bit PCM, the form the byte-exact claim is stated for |
| format tag 1, 24 bits per sample | signed 24-bit PCM, rounded to nearest with ties away from zero, then saturated (`adapters/sample_conversion.sample24_to_int16`) |
| format tag 3, 32 bits per sample | 32-bit IEEE float PCM, scaled by 32768, rounded the same way, then saturated (`adapters/sample_conversion.float_to_int16`) |

Those three and nothing else. 8-bit PCM, 32-bit integer PCM, 64-bit float,
A-law, µ-law, ADPCM, and MPEG or Vorbis audio inside a WAV are each refused
with a `ValueError` whose message names the three forms above. The `fmt ` chunk
must also be the canonical 16-byte one: a `WAVE_FORMAT_EXTENSIBLE` header
(format tag 0xFFFE) is refused even when its sub-format is signed 16-bit PCM,
and that is the header `ffmpeg` writes for a source with more than two channels
or integer samples wider than 16 bits — which is why the conversion below takes
a different route for the six-channel profile.

### Converting a source

The two numbers in every command below are the selection's own channel count
and sample rate: the pair a `WwiseProfile(version, channels, sample_rate)`
carries, and the pair the [profile table](#profile-selection) lists (6 and
44100, or 2 and 48000). A file whose geometry matches no installed
configuration is refused, so take the numbers from the selection you are
encoding for.

`ffmpeg`, for a target it can write directly (run and verified with `ffmpeg`
9.0.1 against the 2-channel 48 kHz profile):

```bash
ffmpeg -i input.mp3 -ac 2 -ar 48000 -c:a pcm_s16le output.wav
```

For the six-channel target `ffmpeg` writes the extensible header, so take the
samples as raw signed-16 and wrap them — one conversion in two steps, the
second being the standard library (run and verified for the 6-channel 44.1 kHz
profile):

```bash
channels=6 rate=44100
ffmpeg -i input.mp3 -f s16le -ac "$channels" -ar "$rate" - > output.pcm
python3 - "$channels" "$rate" <<'PY'
import sys, wave
channels, rate = int(sys.argv[1]), int(sys.argv[2])
with wave.open("output.wav", "wb") as target:
    target.setnchannels(channels)
    target.setsampwidth(2)
    target.setframerate(rate)
    target.writeframes(open("output.pcm", "rb").read())
PY
```

On macOS, `afconvert` writes the canonical header itself for either geometry
(run and verified with `afconvert` 2.0 against both profiles):

```bash
afconvert -f WAVE -d LEI16@44100 -c 6 input.mp3 output.wav
```

`-d LEI16@<rate>` is signed-16 little-endian at the selection's rate, `-c
<channels>` is its channel count, and swapping the pair encodes the other
profile. `afconvert` reproduces an extensible header when its own input carries
one, so hand it the original source rather than a file `ffmpeg` wrote for the
six-channel target.

The `sox` form, `sox input.mp3 -c 6 -r 44100 -b 16 output.wav`, is
**unverified**: `sox` was not installed on the machine these commands were run
on, and nothing here is a tested statement about it.

### The conversion is outside the claim

**The conversion is the caller's step and is not part of what this repository
claims; the claim begins at the signed-16 PCM.** A sample-rate conversion is
implementation-defined rather than fixed — the one specification surveyed that
owns a resampler declares it non-normative and allows any method
([`../findings/input-format-practice.md`](../findings/input-format-practice.md),
N5) — so a library that decoded or resampled on the caller's behalf would be
handing over a claim it cannot keep.

## Profile selection

| Selection | Wwise | PCM | Channels | Rate | Blocks |
| --- | --- | --- | ---: | ---: | ---: |
| `Wwise2013, 6, 44100` | 2013.2 | signed 16-bit | 6 (5.1) | 44100 | 256/2048 |
| `Wwise2013, 2, 48000` | 2013.2 | signed 16-bit | 2 | 48000 | 256/2048 |

Both installed configurations are byte-exact against their paired builds; the
evidence and its limits are in
[`../findings/2ch-byte-exactness.md`](../findings/2ch-byte-exactness.md), and the
profile table with its status is in [`../reference/profiles.md`](../reference/profiles.md).

A selection is one Wwise generation plus the PCM geometry, and it is the only
explicit form — profile names, profile directories, and geometry-only lookups
are not part of the interface:

```python
from wwise_wem import WwiseProfile, WwiseVersion, encode

result = encode(
    "input.wav",
    profile=WwiseProfile(WwiseVersion.WWISE2013, 6, 44100),
)
```

```bash
wwise-wem input.wav --output output.wem --wwise-version 2013.2
```

`profile=None` (the default) selects the installed generation's configuration
for the input geometry; `--wwise-version` defaults to `2013` and does the same
on the command line. `quality` / `--quality` applies an optional finite
interpolation over the selected configuration's quality curves. A selection
that matches no installed configuration — or more than one — is a
`ValueError`, never an approximation and never a first match.

## Decode

```python
from contextlib import closing
from wwise_wem import decode

with closing(decode("output.wem")) as result:
    print(result.channels, result.sample_rate, result.total_frames)
    for block in result:          # list of interleaved f32 samples, ±1.0
        ...
```

`decode` takes the WEM itself (`bytes`, `bytearray` or `memoryview`) or a path,
read whole at call time. It is a plain function returning an iterable result
object, so the geometry is readable before iteration and a container the decoder
refuses is refused by the call: a rejection the audio packets produce is raised
while iterating, after the blocks the packets before it completed. The result
owns one native decode session; `result.close()` (or `contextlib.closing`, as
above) releases it deterministically.

`samples = [sample for block in result for sample in block]` is the whole
stream, interleaved at `result.channels`. Writing it to a WAV is the caller's
step in Python — the package decodes to f32 samples and nothing else. The
command line does that step for you, and writes the same samples as an
uncompressed signed-16 PCM WAV:

```bash
wwise-wem output.wem --decode --output output.wav
```

`--decode` is a flag rather than a subcommand, because the command line is
positional: a `decode` subcommand would take the argument slot a file named
`decode` already occupies. Every encode invocation is unchanged, and the Rust
binary (`crates/wem-core/src/bin/wwise-wem.rs`) takes the same flag with the
same meaning.

The output is signed-16 because that is the form the encode direction reads, so
a decoded file goes straight back into `wwise-wem`. The samples are `decode`'s
own output mapped by the package's float-to-signed-16 rule
(`adapters/sample_conversion.py`), saturated rather than wrapped, which is also
what the Rust command line applies: the two write the same bytes for the same
WEM. The WAV is written only after the decode completes — a session refused part
way through has delivered a prefix of the declared frame count, so the command
line reports the refusal and creates no output, rather than presenting that
prefix as a whole file.

## CLI reference

```text
wwise-wem INPUT.wav --output OUTPUT.wem [OPTIONS]   # encode (the default)
wwise-wem INPUT.wem --decode --output OUTPUT.wav    # decode
python -m wwise_wem ...                             # either direction, the same way
```

| Option | Meaning |
| --- | --- |
| `--quality FLOAT` | Profile quality interpolation (encode) |
| `--wwise-version GENERATION` | Wwise generation (`2013` or `2013.2`, default `2013`); with the WAV geometry this is the whole selection (encode) |
| `--channels N` | Assert the WAV channel count (encode) |
| `--sample-rate HZ` | Assert the WAV sample rate (encode) |
| `--output PATH` | Required; parent directories are created |
| `--decode` | Decode INPUT, a WEM, to a signed-16 PCM WAV instead of encoding |

`--decode` is a flag rather than a subcommand: the command line is positional,
so a `decode` subcommand would take the argument slot a file named `decode`
already occupies. Every encode invocation — including one naming such a file —
is unchanged, and both front ends take the same flag with the same meaning. The
encoder's options are refused in decode mode rather than ignored: a WEM is
self-describing, so there is nothing for them to select or assert.

Decoded output is an uncompressed signed-16 PCM WAV (format tag 1, interleaved,
little-endian), with the channel count and sample rate the container's header
announces. A refusal — a container the decoder will not read, or a packet stream
that stops short — exits non-zero, writes nothing at all, and names how much of
the declared stream had been decoded before it.

## Results and errors

`EncodeResult.data` is the completed WEM and `EncodeResult.stats` is an
immutable `EncodeStats` (`to_dict()` returns the stable JSON-ready fields).
The statistics are what the encoder observed and the caller cannot recompose —
`pcm_frames` (a streaming caller may never have counted the frames it pushed)
and the packet counts (they require parsing the container). The container's
byte length is `len(result)`, and a digest of it is the caller's to compute
from `result.data` with whatever tool and encoding it prefers; the library
picks neither and returns neither.

`TypeError` for unsupported source or field types; `ValueError` for invalid
values, unsupported geometry, profile mismatch, or kernel configuration errors;
standard `OSError` subclasses such as `FileNotFoundError` for file access.

A rejection made by the kernel arrives as `WwiseWemError`, a `ValueError`
subclass whose `.code` is the kernel's stable error class
(`PROFILE_NOT_FOUND`, `GEOMETRY_MISMATCH`, `INPUT_TOO_SHORT`,
`FORMAT_UNSUPPORTED`, `STATE_ERROR`, `INPUT_MALFORMED` — the decode direction's
malformed-input class — and `INTERNAL`) and whose `.message` — also
`str(error)` — is the kernel's diagnostic text. `except ValueError` callers are
unaffected; the original kernel error stays reachable as `__cause__`.

## Other languages

Every binding is a thin shell over the same kernel through the C ABI surface
(`include/wem.h`); what that relationship requires is in
[`../reference/standards.md`](../reference/standards.md#integration-topology).

| Language | Entry point | Guide |
| --- | --- | --- |
| Python | `wwise_wem.encode` | this page |
| Rust | `wem-core` (`Encoder`) | [`../../examples/rust/`](../../examples/rust/) |
| C | `include/wem.h` | [`../../examples/c/`](../../examples/c/) |
| Go | cgo over the C ABI | [`../../examples/go-cgo/`](../../examples/go-cgo/) |
| Node / browser | wasm-bindgen shell | [`../../js/README.md`](../../js/README.md), [`../../examples/wasm-demo/`](../../examples/wasm-demo/) |

Profiles are compiled into every native library: no binding takes a profile
name, a profile directory, or an environment variable. The lower-level bindings
take the structured selection (Wwise generation plus PCM channel count and
sample rate) alongside signed-16 PCM, because their inputs carry no
self-describing header. The selection rules are in
[`../reference/profiles.md`](../reference/profiles.md).

The decode direction exists in the same shells and takes no selection — a WEM is
self-describing: `wwise_wem.decode` (Python), `wem_core::decoder::DecodeSession`
(Rust), `wem_decoder_*` (`include/wem.h` section 5, C) and `WemDecoder` (wasm).
The runnable examples under `examples/` cover both directions in every language
those shells ship; see [`../../examples/README.md`](../../examples/README.md) for
the set.

## Check an install

The bundled sample is a complete acceptance case: 205 audio packets (77 short,
128 long) and 108,771 bytes, byte-identical to the committed reference
container.

```bash
wwise-wem tests/fixtures/input.wav --output out.wem
cmp out.wem tests/fixtures/reference.wem && echo "byte-identical"
```

The same container is the decode direction's case: 139,398 frames of 6-channel
44.1 kHz audio, written as 1,672,820 bytes of WAV — a 44-byte header and
1,672,776 bytes of signed-16 samples.

```bash
wwise-wem tests/fixtures/reference.wem --decode --output out.wav
```

That WAV is a reconstruction of `tests/fixtures/input.wav`, not a copy of it:
Vorbis is lossy, so the two are compared as audio rather than with `cmp`. What
the command line promises is that every sample of `decode`'s output reached the
file unchanged, and that a container it refuses leaves no file behind.

More cases — the six real-build 2ch reference inputs, the two stress inputs, and
the differential fuzz test — are described in
[`../findings/2ch-byte-exactness.md`](../findings/2ch-byte-exactness.md); the
commands that run them are in [`development.md`](development.md).