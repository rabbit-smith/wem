# Rust: encode signed-16 PCM

This standalone package uses `wem-core` directly. Its input is interleaved,
little-endian signed-16 PCM with no header; supply its sample rate, channel
count, and matching profile explicitly.

```sh
cargo run --manifest-path examples/rust/Cargo.toml -- \
  input.pcm output.wem wwise2013-2ch-48000 48000 2
```

The example resolves the packaged profiles from this checkout explicitly, so it
does not depend on `WEM_DATA_DIR` or the current directory.
