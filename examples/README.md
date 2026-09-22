# Integration examples

Each example uses the production Rust kernel. None carries a second encoder,
decoder, profile loader, or DSP implementation: an example that quietly did
something the library does not would be a defect in the documentation.

Every direction the library ships has an example in every language it ships:

| Language | Direction | Example | Input | Output | Interface |
|---|---|---|---|---|---|
| Python | encode | [`python/encode_wav.py`](python/encode_wav.py) | WAV | WEM | `wwise_wem.encode` |
| Python | decode | [`python/decode_wem.py`](python/decode_wem.py) | WEM | raw interleaved LE f32 | `wwise_wem.decode` |
| Rust | encode | [`rust/src/main.rs`](rust/src/main.rs) | interleaved signed-16 PCM | WEM | `wem_core::Encoder` |
| Rust | decode | [`rust/src/bin/decode_wem.rs`](rust/src/bin/decode_wem.rs) | WEM | raw interleaved LE f32 | `wem_core::decoder` |
| C | encode | [`c/encode_pcm16.c`](c/encode_pcm16.c) | WAV or interleaved signed-16 PCM | WEM | `include/wem.h` §1 |
| C | decode | [`c/decode_wem.c`](c/decode_wem.c) | WEM | raw interleaved LE f32 | `include/wem.h` §5 |
| Go | encode | [`go-cgo/main.go`](go-cgo/main.go) | WAV | WEM | C ABI through cgo |
| Go | decode | [`go-cgo/decode/main.go`](go-cgo/decode/main.go) | WEM | raw interleaved LE f32 | C ABI §5 through cgo |
| JavaScript | encode | [`wasm-demo/`](wasm-demo/) | WAV | WEM | wasm streaming API (`WemSession`) |
| JavaScript | decode | [`wasm-demo/`](wasm-demo/) | WEM | raw interleaved LE f32 | wasm `WemDecoder` |

The JavaScript wrapper's typed decode surface (`createDecoder` / `decodeWem` in
[`../js/src/index.ts`](../js/src/index.ts)) has its runtime check in
[`../js/test-node.mjs`](../js/test-node.mjs): it decodes the committed reference
WEM through the wrapper in Node and compares the samples against the WAV that
container was produced from. The browser demo cannot import that wrapper — it is
TypeScript and the page has no build step — so it drives the wasm module
directly for both directions.

Every encode interface selects the encoder configuration the same way: one
structured selection — a Wwise generation plus the PCM geometry (`WemProfile` in
the C ABI, `WwiseProfile` in Rust and Python). Examples whose input is a WAV
read the geometry out of its header; examples fed header-less PCM state it on
the command line. No profile name, profile directory, profile index/manifest
bytes, or environment variable appears anywhere in that scheme.

**The decode direction has no selection at all.** A WEM is self-describing: the
configuration is resolved from the container's own geometry, and the container
whose setup packet this build does not carry is refused
(`WEM_ERR_FORMAT_UNSUPPORTED`), never decoded with a substituted
configuration. The geometry and the declared frame count reach the caller
through the interface's own header announcement (`header_cb` in the C ABI,
`result.channels` / `result.sample_rate` / `result.total_frames` in Python, the
step's `header` in Rust and wasm), never by an example reading the container
itself.

The decode examples all write the same thing — **raw interleaved little-endian
f32 at ±1.0 full scale, with no header** — and each says so in its own README.
That is the decoder's own output in one stated byte order, not a conversion:
choosing a PCM container and a sample format would be a surface no example
owns, and a wrong-but-plausible WAV writer would hide a real defect. It also
makes the examples checkable against each other, which a digest cannot do:

```sh
python3 examples/python/decode_wem.py tests/fixtures/reference.wem py.f32
./wem-c-decode tests/fixtures/reference.wem c.f32
cmp py.f32 c.f32 && echo "byte-identical"
```

The configurations compiled into every native library are Wwise 2013 at
6 channels/44.1kHz (the repository fixture's geometry) and Wwise 2013 at
2 channels/48kHz. A selection no compiled configuration satisfies is rejected,
never silently replaced by a default.

All examples require at least 4096 frames to encode. The Go, C, Rust, and
WebAssembly examples are deliberately thin shells over the canonical kernel
interfaces described in [`../include/wem.h`](../include/wem.h); `go-cgo/` is the
reference for how a new language integrates.