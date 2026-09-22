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

## CLI reference

```text
wwise-wem INPUT.wav --output OUTPUT.wem [OPTIONS]
python -m wwise_wem INPUT.wav --output OUTPUT.wem [OPTIONS]
```

| Option | Meaning |
| --- | --- |
| `--quality FLOAT` | Profile quality interpolation |
| `--wwise-version GENERATION` | Wwise generation (`2013` or `2013.2`, default `2013`); with the WAV geometry this is the whole selection |
| `--channels N` | Assert the WAV channel count |
| `--sample-rate HZ` | Assert the WAV sample rate |
| `--output PATH` | Required; parent directories are created |

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
`FORMAT_UNSUPPORTED`, `STATE_ERROR`, `INTERNAL`) and whose `.message` — also
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
See [`../../examples/README.md`](../../examples/README.md) for the runnable set.

## Check an install

The bundled sample is a complete acceptance case: 205 audio packets (77 short,
128 long) and 108,771 bytes, byte-identical to the committed reference
container.

```bash
wwise-wem tests/fixtures/input.wav --output out.wem
cmp out.wem tests/fixtures/reference.wem && echo "byte-identical"
```

More cases — the six real-build 2ch reference inputs, the two stress inputs, and
the differential fuzz test — are described in
[`../findings/2ch-byte-exactness.md`](../findings/2ch-byte-exactness.md); the
commands that run them are in [`development.md`](development.md).