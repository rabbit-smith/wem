# Rust: encode signed-16 PCM, decode a WEM

This standalone package uses `wem-core` directly, in both directions.

## Encoding

Its input is interleaved, little-endian signed-16 PCM with no header, so the
command line states the whole structured selection: the Wwise generation
(`2013`, or its full generation string) plus the PCM sample rate and channel
count.

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

## Decoding

`src/bin/decode_wem.rs` is the other direction, over
`wem_core::decoder::{DecodeSession, DecodeStep, DecodedHeader}`. There is no
selection argument: a WEM is self-describing, and the geometry arrives on the
step that parses the container's setup packet — `DecodedHeader` carries the
channel count, the sample rate, the frame count the container declares
(`dw_total_pcm_frames`), and the setup packet this build parsed.

```sh
cargo run --release --manifest-path examples/rust/Cargo.toml --bin decode_wem -- \
  tests/fixtures/reference.wem out.f32
```

The session is fed bounded 8 KiB chunks: a step's reply carries every frame that
push completed, so the example holds one chunk of input and one chunk's PCM
rather than the whole stream's. Chunk boundaries never change the samples, and
`Finish` is terminal whatever it returns.

`out.f32` is the decoder's own output, not a conversion of it: raw interleaved
little-endian f32 at ±1.0 full scale, with no header. The printed line carries
what makes the file interpretable — the frame count delivered, the geometry, the
frame count the container declares, the setup packet's length, and how many
steps produced PCM. (`Finish` returning `WEM_OK` is itself the check that the
delivered count equals the declared one; the example also fails loudly if the
two ever disagree.)

Every example in this repository decodes the same WEM to the same bytes, so the
check is a live comparison rather than a recorded digest:

```sh
python3 examples/python/decode_wem.py tests/fixtures/reference.wem py.f32
cmp py.f32 out.f32 && echo "byte-identical"
# and against the C, Go and Node examples the same way
```

A refusal, not a defect: the samples a refused step completed are written
*before* the refusal is reported (`DecodeStep` carries both), which is the C
ABI's `pcm_cb`-then-code order.