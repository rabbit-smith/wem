# wem-go-cgo — Go cgo reference binding

Reference binding for the WEM encoder kernel: a thin cgo shell over the
C ABI core surface (`include/wem.h`). It encodes a signed-16 PCM WAV
file one-shot into a WEM container and writes the bytes — the Rust
kernel does all the work.

This is the canonical example of the integration rule pinned in
`include/wem.h` and `crates/AGENTS.md`: a new language integrates by
writing a shim over the C ABI — never a second transport, never a
language-specific kernel path.

The encoder configuration is one structured selection: the sample reads
`channels` and `sample_rate` out of the WAV header and passes them, with
the Wwise generation `C.WEM_WWISE_2013`, as a `C.WemProfile` by pointer.
cgo mirrors the header's declarations 1:1, so the shell has no profile
name, no profile directory, and no environment variable.

## Build (three steps)

1. Build the C ABI core surface:

   ```bash
   cd crates
   cargo build -p wem-capi --release
   ```

   (produces `crates/target/release/libwem_capi.{so,dylib}`)

2. Build the Go samples:

   ```bash
   cd examples/go-cgo
   CGO_CFLAGS="-I<repo>/include" \
   CGO_LDFLAGS="-L<repo>/crates/target/release -lwem_capi -Wl,-rpath,<repo>/crates/target/release" \
   go build ./...
   ```

   The same flags are already declared in each command's `#cgo` directives
   (relative to the package via `${SRCDIR}`), so a plain `go build ./...` /
   `go run .` works from a clean checkout after step 1. `go vet ./...` covers
   both commands.

3. Run it:

   ```bash
   go run . -wav ../../tests/fixtures/input.wav -out out.wem
   ```

   Byte-exactness check (against the committed reference container):

   ```bash
   cmp out.wem ../../tests/fixtures/reference.wem && echo "byte-identical"
   ```

## Decoding (`./decode`)

`decode/main.go` is the same shell over the decode direction of the same
header (`wem_decoder_new` / `_push` / `_finish` / `_free`, section 5). There is
no selection to pass: a WEM is self-describing, and the geometry arrives
through the C ABI's own header callback.

```bash
go run ./decode -wem ../../tests/fixtures/reference.wem -out out.f32
go run ./decode -wem ../../tests/fixtures/input.wav -out /dev/null   # refused: WemError 7
```

The two C ABI callbacks are the interesting part of a cgo binding, and they are
where cgo's rules bite:

- Both are **Go functions exported to C** (`//export wemGoHeader` /
  `//export wemGoPcm`). A file using `//export` may only carry declarations in
  its cgo preamble, so the callback prototypes are declared there and the
  function-pointer conversion to `C.WemHeaderCb` / `C.WemPcmCb` is explicit.
- The sink the kernel hands back through `user_data` is **C memory**
  (`C.calloc`): a Go pointer to Go memory holding a slice may not cross that
  boundary. The delivered samples are read through `unsafe.Slice` over the
  kernel's `const float *` and written into the sink as little-endian bytes,
  which is also the output file's format.
- The PCM callback runs before `wem_decoder_push` returns; a refusal is the
  return code of that call, and the frames the earlier packets completed have
  already been written.

The output is raw interleaved little-endian f32 at ±1.0 full scale, with no
header: the decoder's own samples, not a conversion of them. The printed line
carries the delivered frames, the geometry, the frame count the container
declares, the setup packet's length and the block count. Every example in this
repository decodes the same WEM to the same bytes, so `cmp out.f32` against
another example's output is the check.

## Notes

- `go.mod` declares `go 1.26` (released); CI pins 1.26 via setup-go.
- The encode sample is single-shot by design; the streaming encode API
  (`wem_session_new` over the same `WemProfile *`, then `push` /
  `finish`) is exercised by the Rust integration tests
  (`crates/wem-capi/tests/capi_surface.rs`). The decode sample pushes bounded
  chunks through the streaming decode surface for the same reason the C and
  Rust samples do: one chunk of input, one chunk's PCM.