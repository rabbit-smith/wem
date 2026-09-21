# Wwise WEM encoder

Standalone, bit-exact implementation of Wwise Vorbis WAV→WEM encoding: a Rust
kernel behind a Python facade, with the C ABI as the canonical core surface.
Correctness is defined by exact bytes against a paired Wwise build, never by
behavioural similarity.

## Status

| Wwise | PCM | Channels | Rate | Blocks | Status |
| --- | --- | ---: | ---: | ---: | --- |
| 2013.2 | signed 16-bit | 6 (5.1) | 44100 Hz | 256/2048 | bit-exact: whole-file golden, per-frame and per-stage contracts |
| 2013.2 | signed 16-bit | 2 | 48000 Hz | 256/2048 | bit-exact: paired input plus eight real-build corpus cases |

The registry is keyed by `(channels, sample_rate)`; further layouts and rates are
added as new profiles without changing the stream encoder API. Profile
provenance, the evidence behind both profiles, and the limits of that evidence
are in [`docs/findings/2ch-byte-exactness.md`](docs/findings/2ch-byte-exactness.md)
and [`docs/roadmap.md`](docs/roadmap.md).

## Install and encode

A wheel needs nothing but Python 3.10+ — the native kernel is embedded in the
package:

```bash
pip install wwise_wem-<version>-<platform>.whl
wwise-wem input.wav --output output.wem
```

From a source checkout, an editable install builds the kernel and needs a Rust
toolchain on `PATH`:

```bash
python3 -m venv .venv && . .venv/bin/activate
pip install -e .          # or: make native
wwise-wem input.wav --output output.wem --expect-sha256 <hex>
```

```python
from pathlib import Path
from wwise_wem import encode

result = encode("input.wav")          # or PcmBuffer / RawPcm
Path("output.wem").write_bytes(result.data)
print(result.stats.audio_packets, result.stats.bytes, result.sha256)
```

The profile is chosen from the input geometry and can be asserted explicitly;
auto-selection, the CLI flags, the accepted input forms and the error contract
are in [`docs/guides/usage.md`](docs/guides/usage.md).

## Integration surface

The canonical interface is the C ABI core surface ([`include/wem.h`](include/wem.h),
implemented by `crates/wem-capi`): the `Init` → `chunk*` → `Finish` lifecycle,
reply framing (`seq 0` carries the setup packet, then audio packets), and the
error codes are pinned there. Every language binding is a thin parallel shell
over the kernel — PyO3 (`wwise_wem._core`), Node and browser via wasm (`js/`),
Go via cgo, C — and owns no numerics of its own. Runnable examples for Python,
Rust, C, Go and the browser live in [`examples/`](examples/README.md).

## Acceptance

The bundled sample encodes to 205 audio packets (77 short, 128 long) and
108,771 bytes:

```text
SHA-256 17851d26c6210b85e498ae0452d2562d7b9e2c3e9e795c459656b9c9d8d35247
```

It is byte-identical to the Wwise reference, as are the paired 2ch/48 kHz input
(37,658 B, 142/142 packets, SHA-256 `41fe43e2…ef629`) and its representative and
stress corpora. The full gate list is in
[`docs/guides/development.md`](docs/guides/development.md#verification-ladder).

## Documentation

| Document | Covers |
| --- | --- |
| [`docs/README.md`](docs/README.md) | Index of the whole doc set |
| [`docs/guides/usage.md`](docs/guides/usage.md) | Install, CLI, Python API, every language binding |
| [`docs/guides/development.md`](docs/guides/development.md) | Layout, build, verification ladder, red lines, conventions |
| [`docs/reference/architecture.md`](docs/reference/architecture.md) | Layers, dependency direction, encoding flow |
| [`docs/reference/domain-model.md`](docs/reference/domain-model.md) | Normative vocabulary |
| [`docs/reference/profiles.md`](docs/reference/profiles.md) | Profile ownership, provenance, frozen tables |
| [`docs/reference/public-interface.md`](docs/reference/public-interface.md) | Compatibility surface |
| [`docs/findings/`](docs/findings/) | Evidence records for completed results |
| [`docs/methodology/`](docs/methodology/) | How a byte-exactness diagnosis is run |
| [`AGENTS.md`](AGENTS.md) | Binding agent-conduct rules (root plus per-subtree guides) |

## Project boundary

- `src/wwise_wem/`: distribution facade — root API, CLI, adapters, profile
  registry — plus `_core.abi3.so`, the single execution path. There is no
  pure-Python fallback at runtime.
- `crates/`: the Rust kernel workspace (`scheduling`, `analysis`, `vorbis`,
  `container`, `profiles`, `core`) with the C ABI (`wem-capi`) and the language
  shells (`wem-python`, `wem-wasm`).
- `reference/wwise_wem_reference/`: bit-exact pure-Python oracle, development
  tree only, imported by the test suites to check the kernel byte for byte.
- `src/wwise_wem/data/`: immutable profile bundles — setup, codebooks and
  calibration tables.
- `tests/`: unit, integration, contract and golden suites plus their data
  assets.