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
active virtual environment. Every byte-producing call goes through the
in-package native extension `wwise_wem._core`; there is no pure-Python
fallback and no profile directory to point at.

## Encode

```bash
wwise-wem input.wav --output output.wem
```

```python
from pathlib import Path
from wwise_wem import encode

result = encode("input.wav")
Path("output.wem").write_bytes(result.data)
print(result.stats.audio_packets, result.stats.bytes, result.sha256)
```

The configuration is selected from the WAV geometry for the installed Wwise
generation. To assert that geometry instead of trusting it, and to pin the
expected result:

```bash
wwise-wem tests/fixtures/input.wav --output output.wem \
  --channels 6 --sample-rate 44100 \
  --expect-sha256 17851d26c6210b85e498ae0452d2562d7b9e2c3e9e795c459656b9c9d8d35247
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

Both installed configurations are byte-exact against their paired builds; see
[`../findings/2ch-byte-exactness.md`](../findings/2ch-byte-exactness.md) and
[`../roadmap.md`](../roadmap.md) for the evidence and its limits.

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
| `--expect-sha256 HEX` | Fail unless the encoded WEM matches this digest |
| `--output PATH` | Required; parent directories are created |

## Results and errors

`EncodeResult.data` is the completed WEM, `EncodeResult.stats` is an immutable
`EncodeStats` (`to_dict()` returns the stable JSON-ready fields), and
`EncodeResult.sha256` is the lowercase digest.

`TypeError` for unsupported source or field types; `ValueError` for invalid
values, unsupported geometry, profile mismatch, or kernel configuration errors;
standard `OSError` subclasses such as `FileNotFoundError` for file access.

## Other languages

The Rust kernel is the single integration point. Its C ABI core surface
(`include/wem.h`) is the interface every shell mirrors — `Init` → `chunk*` →
`Finish`, reply frame `seq 0` carrying the setup packet, then audio packets — and
every binding is a thin parallel shell over it that owns no numerics.

| Language | Entry point | Guide |
| --- | --- | --- |
| Python | `wwise_wem.encode` | this page |
| Rust | `wem-core` (`Encoder`) | [`../../examples/rust/`](../../examples/rust/) |
| C | `include/wem.h` | [`../../examples/c/`](../../examples/c/) |
| Go | cgo over the C ABI | [`../../examples/go-cgo/`](../../examples/go-cgo/) |
| Node / browser | wasm-bindgen shell | [`../../js/README.md`](../../js/README.md), [`../../examples/wasm-demo/`](../../examples/wasm-demo/) |

Profiles are compiled into every native library, so no binding takes a profile
name, a profile directory, or an environment variable. The lower-level
bindings take one structured selection (Wwise generation plus PCM channel count
and sample rate) alongside signed-16 PCM, because their inputs carry no
self-describing header.
See [`../../examples/README.md`](../../examples/README.md) for the runnable set.

## Check an install

The bundled sample is a complete acceptance case: 205 audio packets (77 short,
128 long) and 108,771 bytes.

```bash
wwise-wem tests/fixtures/input.wav --output out.wem \
  --expect-sha256 17851d26c6210b85e498ae0452d2562d7b9e2c3e9e795c459656b9c9d8d35247
```

More cases — the six real-build 2ch reference inputs, the two stress inputs, and
the differential fuzz test — are described in
[`../findings/2ch-byte-exactness.md`](../findings/2ch-byte-exactness.md); the
commands that run them are in [`development.md`](development.md).