# Wwise WEM encoder

Standalone implementation of Wwise Vorbis WAV→WEM encoding: a Rust kernel
behind a Python facade, with the C ABI as the canonical core surface. Runtime
code uses PCM plus immutable profile tables compiled into the kernel. The first exact profile
targets Wwise 2013.2 6ch/44.1kHz; a second profile covers the 2ch/48k paired-build
generation; further generations can be added as additional profiles.

## Supported exact profile

| Wwise | PCM | Channels | Rate | Blocks | Status |
|---|---|---:|---:|---:|---|
| 2013.2 | signed 16-bit | 6 (5.1) | 44100 Hz | 256/2048 | bit-exact |
| 2013.2 | signed 16-bit | 2 | 48000 Hz | 256/2048 | encodes; all psychoacoustic surfaces mechanism-materialized or read-verified (see below) |

The profile registry is keyed by `(channels, sample_rate)`. Further channel
layouts and rates can be added without changing the stream encoder API.

## Install and encode

```bash
python3 -m venv .venv
. .venv/bin/activate
pip install -e .

wwise-wem input.wav --output output.wem
```

The build backend is maturin: an editable install (or any `pip install .`)
builds the abi3 kernel from `crates/wem-python` and embeds it in the
package as `wwise_wem._core`, the facade's single execution path. That
step needs a Rust toolchain on PATH. From a prebuilt wheel (or, when
published, from the index) no Rust is needed at all: the single wheel
ships the facade and its embedded kernel together.

In the development tree, build the kernel with `make native` (a one-line
`maturin develop`) or `pip install -e .`; the pure-Python reference
implementation under `reference/` is a test-time oracle only — the test
suites import it directly to check the kernel byte-for-byte, and no
runtime code path selects between the two.

Automatic selection reads the WAV geometry. The same selection can be made
explicitly:

```bash
wwise-wem input.wav \
  --wwise-version 2013 \
  --channels 6 \
  --sample-rate 44100 \
  --profile wwise2013-6ch-44100 \
  --output output.wem
```

### Single execution path

The facade has exactly one execution path: the native kernel extension
(`wwise_wem._core`, built from `crates/wem-python`), embedded in the
package. A plain `pip install` of a wheel built from this repository
carries that extension (the maturin backend embeds it), so byte-producing
calls work out of the box; the facade never selects, probes, or falls
back. A missing extension surfaces as the ordinary `ImportError` of the
Python import machinery — there is no second way for it to fail.

The pure-Python reference implementation under
`reference/wwise_wem_reference` ships only with the development source
tree; it is a test-time oracle that the test suites import directly to
check the kernel byte-for-byte.

## Python API

```python
from pathlib import Path
from wwise_wem import encode

result = encode(Path("input.wav"))
Path("output.wem").write_bytes(result.data)
print(result.stats.audio_packets, result.stats.bytes)
```

The same function accepts typed in-memory input:

```python
from wwise_wem import PcmBuffer, RawPcm, encode

# Channel-major samples in the signed-16 / 32768 domain.
result = encode(PcmBuffer(48000, (left, right)))

# Unframed bytes require explicit geometry and format.
result = encode(RawPcm(raw_bytes, 48000, 2, "s16le"))
```

WAV format is detected from its RIFF header, independent of the filename
extension. Supported input formats are signed 16-bit PCM, signed 24-bit PCM,
and 32-bit IEEE float PCM. Inputs need at least 4096 frames.

## Acceptance result

The included fixture produces 205 audio packets and a 108,771-byte WEM with
SHA-256:

```text
17851d26c6210b85e498ae0452d2562d7b9e2c3e9e795c459656b9c9d8d35247
```

It is byte-identical to the Wwise reference.

## 2ch/48k profile (paired-build generation)

The 2ch/48k profile encodes stream-compatible WEMs. Its psychoacoustic
surfaces were proven to be init-time materialized geometry of the paired
build — not streamed-audio adaptive state — and a per-instruction port of
the build's geometry materializer reproduces every registered 6ch surface
byte-for-byte. Parity is locked on three surfaces: the ported builder
(`reference/wwise_wem_reference/geometry_materializer/`), the registered
profile bytes (`tests/contract/test_geometry_materializer_contract.py`), and
the kernel mirror (`crates/wem-analysis` `dsp::x87/crt90/psy_geom*`, generated
parity suites). The 2ch short surfaces are among those mechanism-generated
values: `geometry.first_octave` was read from the paired build running at
48000 Hz and all five short surfaces were re-registered from the ported
builder — including both per-profile row sets, each read from its own knot
bank — locked by the same contract. The twelve LONG surfaces were then promoted
the same way and are read-verified against the running build too. Roadmap:
[`docs/roadmap.md`](docs/roadmap.md). Calibration provenance per field:
[`docs/profiles.md`](docs/profiles.md).

## Integration surface

The canonical interface is the C ABI core surface (`include/wem.h`,
implemented by `crates/wem-capi`): the Init -> chunk* -> Finish lifecycle,
reply framing (seq 0 is the setup packet, then audio packets), and error
codes are pinned there. Every language binding is a thin parallel shell over
the kernel — PyO3 (`wwise_wem._core`), Node/wasm (`js/`), Go cgo
(`examples/go-cgo`) — and owns no numerics of its own.

Runnable Python, Rust, C, Go, and browser examples live in
[`examples/`](examples/README.md).

## Tests

```bash
python3 -m unittest discover -s tests -v
```

The suite checks packaged-table integrity, profile resolution, automatic and
explicit selection, CLI output, whole-file golden identity, and geometry-
materializer parity (ported builder == registered surfaces). The contract
layers run from the Makefile (`make golden`, `make frame-contract`,
`make stage-contract`); the Rust workspace gates run with `cargo test
--workspace` (kernel-side materializer parity suites included).

## Project boundary

- `src/wwise_wem/`: distribution facade (root API, CLI, adapters, profile metadata) and its DTOs; the single execution path is the in-package native extension `wwise_wem._core`.
- `crates/`: the Rust kernel workspace (scheduling, analysis, vorbis, container, profiles, core) plus the C ABI (`wem-capi`) and language shells (`wem-python`, `wem-wasm`); `include/wem.h` is the normative cross-language contract.
- `reference/wwise_wem_reference/`: bit-exact pure-Python reference implementation (development tree only, not in the wheel; a test-time oracle the suites import directly), including the vendored geometry materializer.
- `src/wwise_wem/data/`: immutable setup, psychoacoustic and codebook tables.
- `tests/fixtures/`: one PCM input and its bit-exact acceptance WEM.
- `tests/data/frame-contract/`: checked per-frame hashes for the exact profile.

Domain terminology is defined in [`docs/domain-model.md`](docs/domain-model.md).
Development-agent conduct is governed by the layered [`AGENTS.md`](AGENTS.md)
guide set (root, `crates/`, `src/wwise_wem/`, `tests/`, `scripts/`).
