# C: encode signed-16 PCM

Build the canonical C ABI library, compile the example, then encode a
signed-16 PCM WAV. ABI revision 3 selects the encoder configuration with one
structured `WemProfile` value — a Wwise generation plus the PCM geometry — so
the example reads `channels` and `sample_rate` out of the WAV header and passes
that selection by pointer. There is no profile name, no profile directory, and
no environment variable anywhere in the profile selection.

```sh
cd crates && cargo build -p wem-capi --release && cd ..
cc -std=c11 -Wall -Wextra -Werror \
  -Iinclude examples/c/encode_pcm16.c \
  -Lcrates/target/release -lwem_capi \
  -Wl,-rpath,"$PWD/crates/target/release" -o wem-c-example
./wem-c-example tests/fixtures/input.wav out.wem
shasum -a 256 out.wem
# expect: 17851d26c6210b85e498ae0452d2562d7b9e2c3e9e795c459656b9c9d8d35247
```

Header-less interleaved little-endian signed-16 PCM describes no geometry, so
it falls back to an explicit selection:

```sh
./wem-c-example input.pcm out.wem --channels 6 --sample-rate 44100
```

With a RIFF/WAVE input those options are accepted only when they agree with the
header — the file is the authority on its own geometry.

The callback writes each output block directly to the target file. See
[`include/wem.h`](../../include/wem.h) for the streaming API when PCM arrives
incrementally.