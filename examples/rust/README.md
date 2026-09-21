# Rust: encode signed-16 PCM

This standalone package uses `wem-core` directly. Its input is interleaved,
little-endian signed-16 PCM with no header, so the command line states the whole
structured selection: the Wwise generation (`2013`, or its full generation
string) plus the PCM sample rate and channel count.

```sh
cargo run --manifest-path examples/rust/Cargo.toml -- \
  input.pcm output.wem 2013 48000 2
```

`WwiseVersion::parse` decodes the generation label and
`Encoder::new(WwiseProfile::new(version, channels, sample_rate)?)` resolves the
selection against the configurations compiled into the kernel. A selection no
compiled configuration satisfies is an error, never a substituted default;
nothing here needs a profile name, a profile directory, or an environment
variable. The streaming counterpart is
`StreamSession::for_selection(selection)`.

Byte-exactness check against the repository fixture (a 6ch @ 44.1kHz WAV, so
its data chunk is the header-less PCM input):

```sh
dd if=tests/fixtures/input.wav of=input.pcm bs=1 skip=44
cargo run --manifest-path examples/rust/Cargo.toml -- input.pcm out.wem 2013 44100 6
shasum -a 256 out.wem
# expect: 17851d26c6210b85e498ae0452d2562d7b9e2c3e9e795c459656b9c9d8d35247
```