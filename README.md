# Wwise WEM encoder and decoder

Standalone, bit-exact implementation of Wwise 2013.2 Vorbis WAV→WEM encoding,
and a deterministic decoder for the containers it reads: a Rust kernel behind a
Python package, with the C ABI declared in [`include/wem.h`](include/wem.h).

The two directions are held to different claims, deliberately. On the encode
side correctness means the exact bytes of a paired Wwise build, never behavioural
similarity. The decoder claims no such thing and could not: no external decoder
is reproducible across environments, so its contract is determinism plus a round
trip against this repository's own encoder. What each side establishes, and the
test that establishes it, is in
[`docs/reference/standards.md`](docs/reference/standards.md).

Two configurations are installed — Wwise 2013.2 at 6 channels/44.1 kHz and
2 channels/48 kHz — and both are byte-exact against their paired builds. How a
configuration is selected is in
[`docs/reference/profiles.md`](docs/reference/profiles.md); the evidence behind
each, with the limits of it, is in
[`docs/findings/2ch-byte-exactness.md`](docs/findings/2ch-byte-exactness.md).

## Encode a file

From a checkout, this needs Python 3.10+, a Rust toolchain on `PATH`, and a
network connection so pip can fetch the `maturin` build backend:

```bash
python3 -m venv .venv && . .venv/bin/activate
pip install -e .                                  # builds the kernel
wwise-wem tests/fixtures/input.wav --output output.wem
```

Measured on an Apple M3 Max with a warm cargo cache: 2–5 s to create the virtual
environment, about 12 s for the install including the Rust build, and about a
second to encode the 1.7 MB fixture — 15–20 s end to end. A build against an
empty cargo cache also downloads and compiles the dependencies.

**That encode is the scalar configuration, and the numbers below are the encode
stage only.** The package and the wheel impose no threads — the kernel's
`parallel` feature is opt-in, because a wheel's user cannot change a compile-time
feature — so the fixture's 3.2 s of audio take 0.13–0.14 s of encode time on this
machine against 0.11 s with the feature on (both measured under a load average
between 60 and 85, where CPU carries the comparison: the threaded build spends
about 1.5× the CPU on this fixture to finish sooner). Our own Rust CLI is the
binary that enables the feature explicitly, because it encodes one file at a
time — `cargo build --release -p wem-core --features parallel`. The
configuration, the cap on the workers and the deviation from the ecosystem's
environment-variable lever are in
[`docs/reference/standards.md`](docs/reference/standards.md#portability-floor).

A built wheel is the fast route and needs no toolchain at all: the kernel is
compiled into the package, so installing the wheel in a fresh virtual
environment and running the same command takes about three seconds in total
(`make build` writes one into `dist/`). The wheel is a version-stable abi3
build, so one wheel serves every Python 3.10+ on its platform.

## Decode a file

The decoder is in the Python API (the CLI encodes only, for now). It opens on no
selection at all — the container describes itself — and hands back f32 samples at
±1.0 full scale, in bounded blocks:

```python
import wwise_wem

result = wwise_wem.decode("output.wem")   # a path, bytes, bytearray or memoryview
print(result.channels, result.sample_rate, result.total_frames)
for block in result:                      # interleaved f32, up to 1024 frames each
    ...                                   # consume incrementally, or let it go
```

The geometry is readable before the first block, so a decode that expands to
gigabytes streams through bounded memory rather than buffering. What the surface
refuses and with which class, and the streaming property itself, are in
[`docs/reference/decoding.md`](docs/reference/decoding.md).

## What else is here

- **CLI and Python API.** Installing, encoding, decoding, the flags, the accepted
  input forms and the error behaviour are in
  [`docs/guides/usage.md`](docs/guides/usage.md); the exact interface is in
  [`docs/reference/public-interface.md`](docs/reference/public-interface.md).
- **C ABI.** [`include/wem.h`](include/wem.h), implemented by `crates/wem-capi` —
  the interface and the rules it follows are in
  [`docs/reference/standards.md`](docs/reference/standards.md#integration-topology).
- **Language shells and runnable examples.** Python, Rust, C, Go via cgo and the
  browser via wasm — see [`examples/README.md`](examples/README.md) and
  [`js/README.md`](js/README.md).
- **Rust crate.** `wem-core` in the `crates/` workspace, used directly by a Rust
  caller as [`examples/rust/`](examples/rust/README.md) does.
- **Reference implementation and tests.** `reference/wwise_wem_reference/` and
  `tests/` — what each holds is in
  [`docs/guides/development.md`](docs/guides/development.md#repository-map).

## Documentation

[`docs/README.md`](docs/README.md) is the map of the documentation set: product
norms, design of each area, task guides, evidence records and method, each
fact with the one document that holds it.
