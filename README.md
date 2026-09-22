# Wwise WEM encoder

Standalone, bit-exact implementation of Wwise Vorbis WAV→WEM encoding: a Rust
kernel behind a Python facade, with the C ABI as the canonical core surface.
Correctness is defined by exact bytes against a paired Wwise build, never by
behavioural similarity.

## Status

| Wwise | PCM | Channels | Rate | Blocks | Status |
| --- | --- | ---: | ---: | ---: | --- |
| 2013.2 | signed 16-bit | 6 (5.1) | 44100 Hz | 256/2048 | bit-exact: whole-file, per-frame and per-stage comparisons |
| 2013.2 | signed 16-bit | 2 | 48000 Hz | 256/2048 | bit-exact: paired input plus eight real-build corpus cases |

The configuration is selected with one structured value — the Wwise generation
plus the PCM channel count and sample rate; further layouts and rates are added
as new profiles without changing the encoder API. Profile provenance, the
evidence behind both profiles, and the limits of that evidence
are in [`docs/findings/2ch-byte-exactness.md`](docs/findings/2ch-byte-exactness.md)
and [`docs/reference/profiles.md`](docs/reference/profiles.md).

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
wwise-wem input.wav --output output.wem
```

```python
from pathlib import Path
from wwise_wem import encode

result = encode("input.wav")          # or PcmBuffer / RawPcm
Path("output.wem").write_bytes(result.data)
print(len(result), result.stats.audio_packets)
```

The profile is chosen from the input geometry and can be asserted explicitly;
auto-selection, the CLI flags, the accepted input forms and the error behaviour
are in [`docs/guides/usage.md`](docs/guides/usage.md).

## Integration surface

The canonical interface is the C ABI core surface ([`include/wem.h`](include/wem.h),
implemented by `crates/wem-capi`): the `Init` → `push*` → `Finish` lifecycle,
reply framing (`seq 0` carries the setup packet, then audio packets), and the
error codes are pinned there. Every language binding is a thin parallel shell
over the kernel — PyO3 (`wwise_wem._core`), Node and browser via wasm (`js/`),
Go via cgo, C — and owns no numerics of its own. Runnable examples for Python,
Rust, C, Go and the browser live in [`examples/`](examples/README.md).

## Acceptance

The bundled sample encodes to 205 audio packets (77 short, 128 long) and
108,771 bytes. It is byte-identical to the Wwise reference, as are the paired
2ch/48 kHz input (37,658 B, 142/142 packets) and its representative and stress
corpora. Every claim of that kind is made against the bytes themselves — the
committed reference container or the paired build's output — never against a
recorded digest of them. The full list of test targets is in
[`docs/guides/development.md`](docs/guides/development.md#verification-ladder).

## Documentation

| Document | Covers |
| --- | --- |
| [`docs/README.md`](docs/README.md) | Index of the whole doc set |
| [`docs/reference/standards.md`](docs/reference/standards.md) | The product norms, and the test that establishes each |
| [`docs/guides/usage.md`](docs/guides/usage.md) | Install, CLI, Python API, every language binding |
| [`docs/guides/development.md`](docs/guides/development.md) | Layout, build, verification ladder, code-writing standards |
| [`docs/reference/architecture.md`](docs/reference/architecture.md) | Layers, dependency direction, encoding flow |
| [`docs/reference/domain-model.md`](docs/reference/domain-model.md) | The vocabulary to use in code and messages |
| [`docs/reference/profiles.md`](docs/reference/profiles.md) | Profile ownership, selection, provenance, frozen tables |
| [`docs/reference/public-interface.md`](docs/reference/public-interface.md) | Package exports, the `encode` API and the CLI |
| [`docs/findings/`](docs/findings/) | Evidence records for completed results |
| [`docs/methodology/`](docs/methodology/) | How a byte-exactness diagnosis is run |
| [`docs/roadmap.md`](docs/roadmap.md) | What is still open |
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
- `tests/`: unit, integration, cross-implementation and whole-file suites plus their
  data assets.