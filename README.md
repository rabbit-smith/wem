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

## Install

A wheel needs nothing but Python 3.10+. The kernel is compiled into the package,
and it is a version-stable abi3 build, so one wheel serves every Python 3.10+ on
its platform:

```bash
make build                                    # writes one into dist/
pip install dist/wwise_wem-*.whl
```

From a checkout, an editable install builds the kernel and needs Python 3.10+, a
Rust toolchain on `PATH`, and network access so pip can fetch the `maturin`
build backend:

```bash
python3 -m venv .venv && . .venv/bin/activate
pip install -e .                              # or: make native
```

Measured on an Apple M3 Max with a warm cargo cache: 2–5 s to create the virtual
environment, about 12 s for the install including the Rust build, and about a
second to encode the 1.7 MB fixture — 15–20 s end to end. A build against an
empty cargo cache also downloads and compiles the dependencies.

## Encode a file

```bash
wwise-wem tests/fixtures/input.wav --output output.wem
```

**That encode is the scalar configuration, and the timings below are the encode
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

## Every other language

Each example drives the production kernel; none carries a second encoder, profile
loader, or DSP implementation. C and Go link the C ABI, so build it once first:
`cd crates && cargo build -p wem-capi --release`.

| Language | Quick start | Detail |
|---|---|---|
| Python | `python3 examples/python/encode_wav.py tests/fixtures/input.wav out.wem` | [`examples/python/`](examples/python/README.md) |
| Rust | `cargo run --manifest-path examples/rust/Cargo.toml -- in.pcm out.wem 2013 44100 6` | [`examples/rust/`](examples/rust/README.md) |
| C | `cc -std=c11 -Iinclude examples/c/encode_pcm16.c -Lcrates/target/release -lwem_capi -Wl,-rpath,"$PWD/crates/target/release" -o wem-c && ./wem-c tests/fixtures/input.wav out.wem` | [`examples/c/`](examples/c/README.md) |
| Go | `cd examples/go-cgo && go run . -wav ../../tests/fixtures/input.wav -out out.wem` | [`examples/go-cgo/`](examples/go-cgo/README.md) |
| Browser, Node | `make wasm-build && node examples/wasm-demo/serve.mjs` (port 8090) | [`examples/wasm-demo/`](examples/wasm-demo/README.md), [`js/`](js/README.md) |

Every interface selects the configuration the same way — one structured selection
of a Wwise generation plus the PCM geometry, never a profile name, directory,
index or environment variable — and the C ABI is the surface the others mirror.
[`examples/README.md`](examples/README.md) holds the table of what each takes as
input.

## Performance

Both figures come from committed measurement scripts, printed with the machine
identity and the load average they were taken under. Neither carries a threshold:
a number compared against a recorded one hides a change behind a re-record step,
which is why a performance gate does not live in this repository. Reproduce
either with `python3 scripts/measure_encode_perf.py` or
`python3 scripts/measure_decode_perf.py`.

Encode concurrency — throughput, per-encode latency and CPU per encode against N
concurrent encodes, for both installed geometries with the parallel feature on
and off:

![Encoding throughput, per-encode latency and CPU per encode against
concurrency, for both installed geometries with the internal parallel feature
on and off](docs/figures/concurrency-curves.png)

Decode concurrency — the same three questions for decodes, over the push-chunk
sizes and both geometries:

![Decoding throughput, per-decode latency and CPU per decode against
concurrency, for both installed geometries at two push-chunk
sizes](docs/figures/decode-concurrency-curves.png)

The stage splits, the scaling in stream length and the memory curves are in
[`docs/findings/encode-performance.md`](docs/findings/encode-performance.md) and
[`docs/findings/decode-performance.md`](docs/findings/decode-performance.md); the
longer encode memory study is in
[`docs/findings/duration-curves.md`](docs/findings/duration-curves.md).

## Reference

| Document | Holds |
|---|---|
| [`docs/reference/standards.md`](docs/reference/standards.md) | The product norms, and the test that establishes each |
| [`docs/reference/public-interface.md`](docs/reference/public-interface.md) | Package exports, the `encode` and `decode` APIs, result types, the CLI |
| [`docs/reference/decoding.md`](docs/reference/decoding.md) | The decoder: lifecycle, refusal classes, output alignment, the streaming property |
| [`docs/reference/architecture.md`](docs/reference/architecture.md) | Layers, dependency direction, the encoding and decoding flows |
| [`docs/reference/domain-model.md`](docs/reference/domain-model.md) | The vocabulary, which is normative |
| [`docs/reference/profiles.md`](docs/reference/profiles.md) | Profile ownership, selection and provenance |
| [`docs/guides/usage.md`](docs/guides/usage.md) | Task instructions: install, encode, decode, every language |
| [`docs/guides/development.md`](docs/guides/development.md) | Build, test, the verification ladder, the rules work is held to |
| [`docs/roadmap.md`](docs/roadmap.md) | What is still open |
| [`docs/README.md`](docs/README.md) | The map of the whole set, findings and method included |

## Repository layout

- **C ABI.** [`include/wem.h`](include/wem.h), implemented by `crates/wem-capi` —
  the interface and the rules it follows are in
  [`docs/reference/standards.md`](docs/reference/standards.md#integration-topology).
- **Rust crates.** The `crates/` workspace; `wem-core` is what a Rust caller uses
  directly, as [`examples/rust/`](examples/rust/README.md) does.
- **Python package.** `src/wwise_wem/`, whose only execution path is the compiled
  extension `_core`.
- **Reference implementation and tests.** `reference/wwise_wem_reference/` and
  `tests/` — what each holds is in
  [`docs/guides/development.md`](docs/guides/development.md#repository-map).