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
- External dependencies are locked at workspace level (`serde`, `serde_json`).
  Adding any dependency requires an approved task, not a lane fix.
- `make rust-fmt` (`cargo fmt --all --check`), `make rust-lint`
  (`cargo clippy --workspace --all-targets -- -D warnings`) and `make rust-doc`
  (`RUSTDOCFLAGS="-D warnings" cargo doc --workspace --no-deps`) must all come
  back clean. The doc gate exists because clippy does not read doc comments: a
  public item whose documentation links to a private one compiles, lints and
  tests clean, and only rustdoc reports it.

## Verification workflow per crate

- `cargo test -p <crate>` scoped first; `--workspace` only at integration points.
- **Measurements, not benchmarks with thresholds.** `wem-core` keeps the
  fixture-encode measurement in `crates/wem-core/tests/stage_timings.rs`
  (`#[ignore]`d; driven by `scripts/measure_encode_perf.py` with
  `--ignored --nocapture`). It reports the per-stage split and asserts no
  elapsed time. A threshold on a shared machine reports the machine, and a
  recorded median compared against a run hides the change behind a re-record
  step, so neither lives in a test here
  ([`../docs/reference/standards.md`](../docs/reference/standards.md#determinism)).
  Meeting a number never means changing math order.
- The harness may only read the public kernel API; instrumentation inside
  shipping code is not how a stage gets timed (`std::time::Instant` around
  calls that already exist, or a `#[cfg(test)]` seam).

## Git

Lane agents do not commit; the orchestrator commits verified work with
`feat(rust): …` / `fix(rust): …`. `Cargo.lock` changes must be explained by a
declared dependency event.
