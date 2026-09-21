# Integration examples

Each example uses the production Rust kernel. None carries a second encoder,
profile loader, or DSP implementation.

| Language | Directory | Input | Interface |
|---|---|---|---|
| Python | [`python/`](python/) | WAV | `wwise_wem.encode` |
| Rust | [`rust/`](rust/) | interleaved signed-16 PCM | `wem-core` |
| C | [`c/`](c/) | WAV or interleaved signed-16 PCM | `include/wem.h` |
| Go | [`go-cgo/`](go-cgo/) | WAV | C ABI through cgo |
| JavaScript | [`wasm-demo/`](wasm-demo/) | WAV | WebAssembly streaming API |

Every interface selects the encoder configuration the same way: one structured
selection — a Wwise generation plus the PCM geometry (`WemProfile` in the C ABI,
`WwiseProfile` in Rust and Python). Examples whose input is a WAV read the
geometry out of its header; examples fed header-less PCM state it on the command
line. No profile name, profile directory, profile index/manifest bytes, or
environment variable appears anywhere in that scheme.

The configurations compiled into every native library are Wwise 2013 at
6 channels/44.1kHz (the repository fixture's geometry) and Wwise 2013 at
2 channels/48kHz. A selection no compiled configuration satisfies is rejected,
never silently replaced by a default.

All examples require at least 4096 frames. The Go, C, Rust, and WebAssembly
examples are deliberately thin shells over the canonical kernel interfaces
described in [`../include/wem.h`](../include/wem.h); `go-cgo/` is the reference
for how a new language integrates.