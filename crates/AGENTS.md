# crates/ — Rust kernel

The Rust workspace is the performance and portability core: every high-throughput
stage (scheduling, analysis, Vorbis coding, container assembly) runs here.
Python and gRPC are thin shells over `wem-core`; nothing numerically hot stays
on the wrapper side.

## Crate map and allowed edges (acyclic; mirrors docs/architecture.md)

```
wem-profiles  → (types of wem-vorbis/wem-analysis/wem-container; sole resource owner)
wem-scheduling→ (none below it)
wem-analysis  → wem-scheduling (+ own dsp)
wem-vorbis    → (pure codec primitives; never profiles/container/application)
wem-container → (pure container codecs)
wem-core      → all of the above (sole orchestration layer)
```

- A crate may not `use` a sibling crate outside its declared direction; enforce
  by visibility and review, not runtime checks.
- External dependencies are locked at workspace level (`sha2`, `serde`,
  `serde_json`). Adding any dependency requires an approved task, not a lane fix.

## Bit-exact porting contract

1. The Python oracle is the specification. Port statement-by-statement:
   each `_f32(...)` in Python equals one `f32` operation boundary in Rust.
   Intermediate values that Python keeps as float64-wrapped-float32 must not be
   promoted or fused; do not algebraically rearrange sums, products, or the
   MDCT butterfly/bitreverse order.
2. No `f32::sin/cos/ln/log/exp/powf` anywhere in runtime paths. All transcendental
   inputs come from `wem-profiles` (`FrozenMathTables`, mdct trig bank). A ported
   function that would need a transcendental is a design error, not a TODO.
3. Float bit patterns cross boundaries via `to_bits/from_bits` or LE bytes only.
4. `fma` and `fast` math intrinsics are prohibited (they change rounding).
   SIMD may vectorize element-wise operations only, never change summation order,
   and must re-pass the stage contract before shipping.
5. Cross-frame mutable state lives in exactly one analysis session struct
   (domain model: *Analysis session*); everything else stays immutable
   (`Block plan`, `EncoderProfileResources` equivalents).

## Coding standards

- `cargo fmt` and `cargo clippy --workspace -- -D warnings` are gates.
- No `unwrap()/expect()/panic!()` on input-derived paths in `src/`; tests only.
  Public errors are per-crate enums (`ProfileError` style) with explicit variants
  matching Python's rejection conditions (missing geometry, checksum mismatch,
  unknown profile, out-of-domain lookup).
- `unsafe` is prohibited without a comment proving why bit-exactness or FFI
  requires it; assume "not needed" until proven otherwise.
- Naming follows `docs/domain-model.md` terms (`FramePlan`, `AnalysisSession`,
  `AudioPacket`, `WemContainer`, `FrozenMathTables`); no parallel vocabularies.
- Hot loops: no per-sample allocation, no `Vec` returns from inner stages where a
  scratch buffer exists; keep scratch ownership at the session boundary.

## Verification workflow per crate

- Every stage lands with its parity test against `tests/data/stage-golden/`
  (hashes for all 205 frames, raw byte equality for the 28 representative
  frames). A stage is not done until the byte diff names the first mismatching
  frame and field and comes back zero.
- `cargo test -p <crate>` scoped first; `--workspace` only at integration points.
- Benchmarks: `wem-core` keeps a fixture-encode timer; the regression threshold
  is documented next to the bench. Never "optimize" by changing math order.

## Git

Lane agents do not commit; the orchestrator commits verified work with
`feat(rust): …` / `fix(rust): …`. `Cargo.lock` changes must be explained by a
declared dependency event.
