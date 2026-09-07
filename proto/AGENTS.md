# proto/ — wwise.v1 gRPC contract

The proto tree is the machine-readable architecture: the Rust kernel
(`wem-server`) and any language client treat it as canonical. Historical
note: a Python mock server once pinned its semantics before the real one
existed; v1 semantics are now pinned by the wem-server integration tests
and `scripts/interop_grpc_smoke.py`.

## Evolution rules

- `wwise.v1` is frozen additive-only. Never reuse, renumber, or retag existing
  fields; every message reserves headroom (`reserved N to M;`) — consume
  reserved numbers only with a contract-tested migration note.
- Lifecycle semantics are part of the contract and mirror the Analysis session:
  exactly one `Init` (ProfileRef: `setup_sha256` is a hard identity assertion,
  `name` is soft; `TemplateRef` is rejected in v1) → `chunk`s → `Finish`;
  responses are `Packet`×N (`seq 0` = setup packet, then audio packets) followed
  by one `WemComplete` (`total_len`, lowercase hex `sha256`, optional
  `inline_bytes`), or a terminal `EncoderError` with an enumerated code.
  Semantics live in field/enum comments — keep them true to the test suite.
- Streaming chunk boundaries must not affect output bytes; any client-side
  framing rule added later needs a smoke case proving that.

## Gates

- `buf lint` (v2, STANDARD; the single documented exception is
  `SERVICE_SUFFIX` for `WemEncoder`) and `buf breaking` (WIRE_JSON) are the
  CI-facing checks; `tests/contract/test_proto_contract.py` enforces structure
  without external tooling (skips with an install hint when grpcio-tools is
  absent; CI installs it).
- `scripts/interop_grpc_smoke.py` (and the wem-server integration tests) must
  keep proving byte identity between the streamed path and the local encode;
  a proto change that cannot preserve this is a protocol redesign (new
  version), not an edit.
- Generated code (`*_pb2*.py`, Rust/Go stubs) is never committed; tooling
  generates into temp directories at run time.

## Style

proto3, `package wwise.v1`, snake_case fields, UPPER_SNAKE enum values with an
`*_UNSPECIFIED = 0` first value, one RPC domain per file
(`common`/`profile`/`encode`), imports only inside the tree (no Google
well-knowns unless genuinely needed). Comments must not use
reverse-engineering-sensitive wording.
