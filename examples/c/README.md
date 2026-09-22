# C: encode signed-16 PCM, decode a WEM

Both examples drive [`include/wem.h`](../../include/wem.h) directly — the same
header every language shell mirrors.

## Encoding

Build the canonical C ABI library, compile the example, then encode a signed-16
PCM WAV. ABI revision 3 selects the encoder configuration with one structured
`WemProfile` value — a Wwise generation plus the PCM geometry — so the example
reads `channels` and `sample_rate` out of the WAV header and passes that
selection by pointer. There is no profile name, no profile directory, and no
environment variable anywhere in the profile selection.

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

## Decoding

`decode_wem.c` drives section 5 of the same header — `wem_decoder_new` /
`wem_decoder_push` / `wem_decoder_finish` / `wem_decoder_free` — with the two
callbacks that surface requires:

```sh
cc -std=c11 -Wall -Wextra -Werror \
  -Iinclude examples/c/decode_wem.c \
  -Lcrates/target/release -lwem_capi \
  -Wl,-rpath,"$PWD/crates/target/release" -o wem-c-decode
./wem-c-decode tests/fixtures/reference.wem out.f32
```

- `announce_header` is the one-time header announcement: the geometry, the
  frame count the container declares (`dw_total_pcm_frames`) and the setup
  packet this revision parsed. It is the only place the geometry appears, which
  is why the C ABI makes both callbacks required.
- `write_pcm` receives interleaved f32 at ±1.0 full scale in bounded blocks
  (1024 frames each, the C ABI's delivery size) and writes them as they arrive.

No selection is passed anywhere: a WEM is self-describing, and the
configuration is resolved from the container's own geometry. A container that
parses but is not one this build carries is refused with `WemError 5`
(`WEM_ERR_FORMAT_UNSUPPORTED`); bytes that do not parse are `WemError 7`
(`WEM_ERR_INPUT_MALFORMED`) — the decode surface's own class, the appended
value of the stable table in section 2.

The output is raw **interleaved little-endian f32 at ±1.0 full scale, with no
header** — the decoder's own samples. It is deliberately not a WAV writer:
choosing a sample format, a chunk layout and a header would be a surface this
example does not own, and one that could be wrong while still looking
plausible. Each sample is packed to four little-endian bytes explicitly rather
than `fwrite`-ing the `const float *` block, so the file does not depend on the
host's byte order and the other examples' output for the same WEM is
byte-for-byte comparable:

```sh
python3 examples/python/decode_wem.py tests/fixtures/reference.wem py.f32
cmp py.f32 out.f32 && echo "byte-identical"
```

The summary line reports the frames delivered, the geometry, the declared frame
count, the setup packet's length and the number of `pcm_cb` blocks. A refusal
is reported by the call that hit it, after the frames the earlier packets
completed have already been written — the C ABI's `pcm_cb`-then-code order —
and the process exits non-zero with the `WemError` code.