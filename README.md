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

The template metadata compatibility path remains available:

```bash
wwise-wem input.wav --template reference.wem --output output.wem
```

## Python API

```python
from pathlib import Path
from wwise_wem import encode_wav

result = encode_wav(Path("input.wav"))
Path("output.wem").write_bytes(result.data)
print(result.stats.audio_packets, result.stats.bytes)
```

For PCM already loaded in memory, construct `Encoder(profile)` and call
`encode_pcm(PcmBuffer)`. Inputs need at least 4096 PCM frames. The
`encode_wav_to_wem` tuple API remains as a compatibility adapter.

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

- `src/wwise_wem/`: encoder, packet/container implementation and profile registry.
- `src/wwise_wem/data/`: immutable setup, psychoacoustic and codebook tables.
- `tests/fixtures/`: one PCM input and its bit-exact acceptance WEM.
- `tests/data/frame-contract/`: checked per-frame hashes for the exact profile.

Domain terminology is defined in [`docs/domain-model.md`](docs/domain-model.md).
Development-agent conduct is governed by the layered [`AGENTS.md`](AGENTS.md)
guide set (root, `crates/`, `src/wwise_wem/`, `tests/`, `proto/`, `scripts/`).
