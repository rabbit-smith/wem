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

2. Build the Go sample:

   ```bash
   cd examples/go-cgo
   CGO_CFLAGS="-I<repo>/include" \
   CGO_LDFLAGS="-L<repo>/crates/target/release -lwem_capi -Wl,-rpath,<repo>/crates/target/release" \
   go build .
   ```

   The same flags are already declared in `main.go`'s `#cgo` directives
   (relative to the package via `${SRCDIR}`), so a plain `go build .` /
   `go run .` works from a clean checkout after step 1.

3. Run it:

   ```bash
   go run . -wav ../../tests/fixtures/input.wav -out out.wem
   ```

   Byte-exactness check (against the golden digest):

   ```bash
   shasum -a 256 out.wem
   # expect: 17851d26c6210b85e498ae0452d2562d7b9e2c3e9e795c459656b9c9d8d35247
   ```

## Notes

- `go.mod` declares `go 1.26` (released); CI pins 1.26 via setup-go.
- The sample is single-shot by design; the streaming API
  (`wem_session_new` over the same `WemProfile *`, then `push` /
  `finish`) is exercised by the Rust integration tests
  (`crates/wem-capi/tests/capi_e2e.rs`).
