# C: encode signed-16 PCM

Build the canonical C ABI library, compile the example, then encode
interleaved little-endian signed-16 PCM. `CHANNELS` is needed only to derive
the frame count from the raw input; the kernel checks it against `PROFILE`.

```sh
cd crates && cargo build -p wem-capi --release && cd ..
cc -std=c11 -Wall -Wextra -Werror \
  -Iinclude examples/c/encode_pcm16.c \
  -Lcrates/target/release -lwem_capi \
  -Wl,-rpath,"$PWD/crates/target/release" -o wem-c-example
./wem-c-example input.pcm output.wem wwise2013-2ch-48000 2
```

The callback writes each output block directly to the target file. See
[`include/wem.h`](../../include/wem.h) for the streaming API when PCM arrives
incrementally.
