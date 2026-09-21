# Integration examples

Each example uses the production Rust kernel. None carries a second encoder,
profile loader, or DSP implementation.

| Language | Directory | Input | Interface |
|---|---|---|---|
| Python | [`python/`](python/) | WAV | `wwise_wem.encode` |
| Rust | [`rust/`](rust/) | interleaved signed-16 PCM | `wem-core` |
| C | [`c/`](c/) | interleaved signed-16 PCM | `include/wem.h` |
| Go | [`go-cgo/`](go-cgo/) | WAV | C ABI through cgo |
| JavaScript | [`wasm-demo/`](wasm-demo/) | WAV | WebAssembly streaming API |

The Python facade selects a profile from a WAV's channel count and sample rate.
The lower-level examples take a profile name and signed-16 PCM because their
inputs have no self-describing header. Profiles are compiled into every native
library and no profile directory or environment variable is required. The exact profiles are
`wwise2013-6ch-44100` and `wwise2013-2ch-48000`.

All examples require at least 4096 frames. The Go, C, Rust, and WebAssembly
examples are deliberately thin shells over the canonical kernel interfaces
described in [`../include/wem.h`](../include/wem.h).
