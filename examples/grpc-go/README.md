# wwise.v1 reference gRPC client (Go)

A minimal Go client for the `wwise.v1` `WemEncoder` service implemented by
`crates/wem-server` (tonic). It streams a local WAV through `Encode`
(Init → chunk* → Finish) and writes the returned WEM container.

Expected result for the repository fixture (golden hash pinned by the Rust
kernel tests):

    sha256: 17851d26c6210b85e498ae0452d2562d7b9e2c3e9e795c459656b9c9d8d35247

CI coverage: the `server-interop` job in `.github/workflows/test.yml`
regenerates these stubs with the exact recipe below and re-runs this
end-to-end sha256 assertion on every push.

## Layout

- `main.go` — the client (hand-written; includes a minimal RIFF/WAV reader).
- `wwise/v1/*.pb.go` — **generated gRPC stubs, never committed**
  (`.gitignore`d; `proto/AGENTS.md`: generated code goes to temp/run-time
  directories). Regenerate them before building (step 1 below).

## Build (from the repository root)

1. Generate the Go stubs into this directory (`protoc` + `protoc-gen-go` +
   `protoc-gen-go-grpc` are required; install the plugins with
   `go install google.golang.org/protobuf/cmd/protoc-gen-go@latest` and
   `go install google.golang.org/grpc/cmd/protoc-gen-go-grpc@latest`):

       PATH="$HOME/go/bin:$PATH" protoc -I proto \
         --go_out=examples/grpc-go --go_opt=paths=source_relative \
         --go_opt=Mwwise/v1/common.proto=grpc-go/wwise/v1 \
         --go_opt=Mwwise/v1/profile.proto=grpc-go/wwise/v1 \
         --go_opt=Mwwise/v1/encode.proto=grpc-go/wwise/v1 \
         --go-grpc_out=examples/grpc-go --go-grpc_opt=paths=source_relative \
         --go-grpc_opt=Mwwise/v1/common.proto=grpc-go/wwise/v1 \
         --go-grpc_opt=Mwwise/v1/profile.proto=grpc-go/wwise/v1 \
         --go-grpc_opt=Mwwise/v1/encode.proto=grpc-go/wwise/v1 \
         proto/wwise/v1/common.proto proto/wwise/v1/profile.proto proto/wwise/v1/encode.proto

   The `M<file>=grpc-go/wwise/v1` mapping is required, not cosmetic:
   `proto/wwise/v1/*.proto` carry no `go_package` option (and must not be
   changed), so the import path `grpc-go/wwise/v1` (matching `go.mod`)
   comes from these flags. The shorter `--go_opt=module=` form is rejected
   by protoc-gen-go in this setup (see CI, which pins the recipe).

2. Resolve Go module dependencies and build:

       cd examples/grpc-go
       go mod tidy
       go build ./...

## Run

Start the Rust server (debug build):

    cd crates
    CARGO_TARGET_DIR=/tmp/wem-grpc cargo build -p wem-server
    /tmp/wem-grpc/debug/wem-server --addr 127.0.0.1:50991

(Or pass `--addr 127.0.0.1:0` for an ephemeral port: the server prints
`READY <addr>` on startup, and CI discovers it that way instead of using
a fixed port or sleep.)

Then, from `examples/grpc-go`:

    go run . -addr 127.0.0.1:50991 \
      -wav ../../tests/fixtures/input.wav \
      -out /tmp/out.wem

Flags: `-addr` (or `WEM_GRPC_ADDR`), `-wav` (input file, required),
`-out` (output path), `-chunk-bytes` (stream chunk size, auto frame-aligned).

The client prints the profile handshake (name / `setup_sha256` /
`manifest_sha256`), the packet/`WemComplete` summary, and the sha256 of the
written WEM bytes.

## Local verification evidence (this environment, 2026-09)

- `go 1.26.3`, `protoc 35.1` (`/opt/homebrew`),
  `protoc-gen-go v1.34.2`, `protoc-gen-go-grpc 1.5.1` (from `$HOME/go/bin`).
- Server: `wem-server` (release build) on an ephemeral loopback port,
  discovered via its `READY <addr>` line (bounded polling, no fixed sleep).
- Result: 206 packets (setup + audio), `total_len=108771`,
  `sha256=17851d26c6210b85e498ae0452d2562d7b9e2c3e9e795c459656b9c9d8d35247`,
  output byte-identical to `tests/fixtures/reference.wem` (`cmp` clean).

## Notes / TODOs

- The client writes the container from `WemComplete.inline_bytes` (set by
  the server for outputs ≤ 2 MiB). For larger outputs the client must
  reconstruct the container from the packet stream — not implemented here
  (the server leaves `inline_bytes` empty above 2 MiB).
- No TLS: loopback reference only; production wiring is a follow-up.
