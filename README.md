# Wwise WEM encoder

Standalone Python implementation of Wwise Vorbis WAV-to-WEM encoding. Runtime
code uses PCM plus immutable packaged profile tables. The first exact profile
targets Wwise 2013.2; further generations can be added as additional profiles.

## Supported exact profile

| Wwise | PCM | Channels | Rate | Blocks | Status |
|---|---|---:|---:|---:|---|
| 2013.2 | signed 16-bit | 6 (5.1) | 44100 Hz | 256/2048 | bit-exact |

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
package as `wwise_wem._native`, so the native engine is available directly.
That step needs a Rust toolchain on PATH. From a prebuilt wheel (or, when
published, from the index) no Rust is needed at all: the single wheel ships
facade and kernel together.

If you only need the pure-Python reference engine in the development tree,
skip the native build and keep the repository layout on the path (for
example `PYTHONPATH=src:reference`, as the Makefile uses), with
`WWISE_WEM_ENGINE=python` or the auto fallback.

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

### Engine availability

The facade is native-first: when the native kernel (`wwise_wem._native`,
built from `crates/wem-python`) is importable it encodes; otherwise the
facade delegates to the pure-Python reference implementation in the
development tree (`reference/wwise_wem_reference`). A plain `pip install`
of a wheel built from this repository already carries the native extension
(the maturin backend embeds it in the wheel), so byte-producing calls work
out of the box. The pure-Python reference engine ships only with the
development source tree (`reference/wwise_wem_reference`, importable when
`reference/` is on `PYTHONPATH`); an installed facade without the native
extension raises a clear `ImportError` from byte-producing calls instead of
running a partial pipeline. The engine that produced a result is reported on
`EncodeStats.engine`.

## Python API

```python
from pathlib import Path
from wwise_wem import encode_wav

result = encode_wav(Path("input.wav"))
Path("output.wem").write_bytes(result.data)
print(result.stats.audio_packets, result.stats.bytes)
```

For PCM already loaded in memory, construct `Encoder(profile)` and call
`encode_pcm(PcmBuffer)`. Inputs need at least 4096 PCM frames.
`read_pcm16_wav(path)` remains as a compatibility helper that reads a WAV
into the historical `(rate, frames, channel_rows)` float domain.

## Acceptance result

The included fixture produces 205 audio packets and a 108,771-byte WEM with
SHA-256:

```text
17851d26c6210b85e498ae0452d2562d7b9e2c3e9e795c459656b9c9d8d35247
```

It is byte-identical to the Wwise reference.

## Tests

```bash
python3 -m unittest discover -s tests -v
```

The suite checks packaged-table integrity, profile resolution, automatic and
explicit selection, CLI output, and whole-file golden identity.

## Project boundary

- `src/wwise_wem/`: distribution facade (root API, engine switch, CLI, adapters, profile metadata) and its DTOs.
- `reference/wwise_wem_reference/`: bit-exact pure-Python reference implementation (development tree only, not in the wheel).
- `src/wwise_wem/data/`: immutable setup, psychoacoustic and codebook tables.
- `tests/fixtures/`: one PCM input and its bit-exact acceptance WEM.
- `tests/data/frame-contract/`: checked per-frame hashes for the exact profile.

Domain terminology is defined in [`docs/domain-model.md`](docs/domain-model.md).
Development-agent conduct is governed by the layered [`AGENTS.md`](AGENTS.md)
guide set (root, `crates/`, `src/wwise_wem/`, `tests/`, `proto/`, `scripts/`).
