# crates/ — Rust kernel

The Rust workspace is the performance and portability core: every high-throughput
stage (scheduling, analysis, Vorbis coding, container assembly) runs here; Python
(PyO3) and the C ABI are thin shells over `wem-core`, and nothing numerically hot
stays on the wrapper side.

The norms this subtree is held to — the crate edges and allowed dependency
directions, the C ABI surface, the porting and float rules, the panic and error
rules, `unsafe`, hot-loop and naming rules — are in
[`../docs/reference/standards.md`](../docs/reference/standards.md).

## C ABI surface

[`../docs/reference/standards.md`](../docs/reference/standards.md#integration-topology)
holds it: the lifecycle, the reply framing, the error codes, memory ownership
and the shell-mapping rules. `crates/wem-capi` implements the header 1:1.

## Workspace rules

- A crate may not `use` a sibling crate outside the direction declared there;
  the edges are held by the crate manifests and by review, not by runtime checks.
- External dependencies are locked at workspace level (`sha2`, `serde`,
  `serde_json`). Adding any dependency requires an approved task, not a lane fix.
- `make rust-fmt` (`cargo fmt --all --check`) and `make rust-lint`
  (`cargo clippy --workspace --all-targets -- -D warnings`) must both come back
  clean.

## Verification workflow per crate

- `cargo test -p <crate>` scoped first; `--workspace` only at integration points.
- Benchmarks: `wem-core` keeps a fixture-encode timer; the regression threshold
  is documented next to the bench. Meeting it never means changing math order
  ([`../docs/reference/standards.md`](../docs/reference/standards.md#determinism)).

## Git

Lane agents do not commit; the orchestrator commits verified work with
`feat(rust): …` / `fix(rust): …`. `Cargo.lock` changes must be explained by a
declared dependency event.
