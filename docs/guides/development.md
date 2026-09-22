# Development

How the repository is laid out, how to build and verify it, and the rules the
work is held to. The product norms are in
[`../reference/standards.md`](../reference/standards.md); the map of the doc set
is [`../README.md`](../README.md). The `AGENTS.md` set carries the same working
rules for an agent, together with where to start and what to register.

## Repository map

| Path | Role |
| --- | --- |
| `src/wwise_wem/` | Distribution facade: root API, CLI, adapters, profile identity, and the embedded native extension |
| `crates/` | Rust kernel workspace: `scheduling`, `analysis`, `vorbis`, `container`, `profiles`, `core`, plus the C ABI (`wem-capi`) and the language shells (`wem-python`, `wem-wasm`) |
| `include/wem.h` | C ABI interface the shells mirror 1:1, implemented by `crates/wem-capi` |
| `reference/wwise_wem_reference/` | Bit-exact pure-Python oracle, development tree only, never shipped |
| `crates/wem-profiles/src/generated/` | The compiled profile carrier: setup, codebooks, transforms, analysis and psychoacoustic tables as Rust constants (`corpus/profiles/` is the untracked material they are generated from) |
| `tests/` | `unit/`, `integration/`, `parity/`, `whole_file/`, and the `data/` regression assets |
| `scripts/` | Regeneration, decoding, fuzz, and packaging tools |
| `examples/`, `js/` | Runnable bindings: Python, Rust, C, Go via cgo, Node and browser via wasm |
| `docs/` | This guide set; see the [map](../README.md) |

Dependency direction is one-directional and acyclic: `scheduling` imports
nothing downstream, `analysis`/`vorbis`/`container` never open package
resources, `profiles` is the sole carrier owner, and one application layer
assembles the use case. The rules are in
[`../reference/standards.md`](../reference/standards.md#layers-and-dependency-direction)
and [`../reference/architecture.md`](../reference/architecture.md).

## Build

```bash
make native          # maturin develop: build the in-package kernel extension
make build           # wheel into dist/
make wheel-smoke     # install the wheel in a clean venv and encode once
make wasm-build      # wasm-pack: both shell packages (js/pkg, js/pkg-node)
```

`pip install -e .` is equivalent to `make native`. Python-only tooling:
`make lint` (ruff + mypy), `make rust-lint` (clippy, warnings denied),
`make rust-doc` (rustdoc, warnings denied).

The wasm packages are build output and are not committed, so `make wasm-build`
runs before the Node test and before the browser demo
([`../../examples/wasm-demo/`](../../examples/wasm-demo/)).

## Verification ladder

Climb from the cheapest executable target; do not start from the full suite.

1. One `unittest`/`cargo test` case or module.
2. The affected suite: `make test-fast`, `make 2ch-stress`,
   `make 2ch-long`, `tests/parity/test_frame_pipeline_parity.py`, or one
   crate's tests.
3. The full ladders (`make test`, `cargo test --workspace`) at phase
   boundaries, or whenever the change touches shared state, configuration,
   lockfiles, or generated assets.
4. `make check` before handing work back: lint, rust-lint, rust-doc, the full
   test ladder, and the wheel smoke.

A passing suite is reused. Do not re-run a green suite per task or per agent;
rerun it only on a named invalidator — code, test, data, or configuration that
the suite actually exercises. `make fuzz-parity` is the one to reach for when a
change touches streaming, scheduling, profile assembly, or packet packing.

### What each target runs

The table lists the local targets and the quantities they check. Every one of
them is a test run; when one goes red, it names what changed, and the answer is
to fix the code or the test rather than to re-record the expectation.

| Target | Checks |
| --- | --- |
| `make wem-bytes` | Whole-file reference WEM parity: the encoder's bytes equal the committed `tests/fixtures/reference.wem`, byte for byte (205 audio packets) |
| `tests/parity/test_frame_pipeline_parity.py`, `cargo test -p wem-core --test frame_pipeline_parity` | Per-frame values (scheduling fields, eight analysis stages, floor posts, residue rows, audio packet) against the pure-Python oracle, all 205 frames |
| `cargo test -p wem-container --test container_codec` | The native container builder against the oracle's, byte for byte, from one live packet stream, plus the degenerate payloads it must refuse with a typed error |
| `make 2ch-stress`, `make 2ch-long` | The 2ch stress corpus and the long-run cross-implementation comparison |
| `make fuzz-parity` | Native vs oracle parity under randomized chunking and quality; Python unit, integration and cross-implementation suites run in `make test-fast` |
| `cargo test --workspace` | Rust kernel, C ABI and shell suites, including the geometry-materializer parity suites — in the default (scalar) configuration |
| `make rust-test` | The same suites, then a second leg with `--features parallel`: the kernel's internal parallelism is opt-in, so the default run alone would leave that configuration unexercised |
| `make wheel-smoke` | Installed-wheel inventory (facade + native engine, no profile data) and one real encode |
| `make wasm-build` | wasm-pack builds both shell packages: `js/pkg` (web) and `js/pkg-node` (nodejs) |
| `make wasm-test` | Builds both packages, then runs `js/test-node.mjs`: the shell's bytes against `tests/fixtures/reference.wem` (one-shot, three chunkings) plus the selection and error-code mapping; with no package built it fails and prints the build command |
| `make check` | `lint`, `rust-fmt`, `rust-lint`, `rust-doc`, the full Python ladder and the wheel smoke |
| `make rust-doc` | Rustdoc over the workspace with warnings denied. It resolves every intra-doc link, which clippy does not read: a public item whose documentation links to a private one compiles, lints and tests clean, and only this target reports it |

One target is **not** a check: `python3 scripts/measure_encode_perf.py` measures
the release build — the CLI stage timers and the kernel's per-stage split, each
as min / median / p95 / spread with the machine identity and the load it ran
under — and never goes red ([Watching performance](#watching-performance)). What
it reported, and what the numbers show, is in
[`../findings/encode-performance.md`](../findings/encode-performance.md).

The same targets from the test tree's point of view — layer, command and run
order — are in [`../../tests/AGENTS.md`](../../tests/AGENTS.md).

## Code-writing standards

What a change is expected to look like, beyond passing the tests. Where a check
exists it is named; where it does not, the rule is read in review. Formatting is
not a review topic: `make rust-fmt` and `make lint` decide it.

- **A name says what the thing is or does.** A function named for the work it
  performs or the value it returns, a variable for the value it holds, a test
  for the behaviour it pins (`test_windows_reproduce_reference_synthesis`). No
  abbreviation that needs a comment to decode. No check carries a naming rule;
  this is read in review.
- **A unit does one thing.** One reason to change per function, one owner per
  module; split a function before adding a second mode to it. Not machine-checked
  — clippy's complexity lints are not enabled.
- **No dead code and no commented-out code.** Delete it; the git history has it.
  Unused imports and locals are caught by `ruff` (`F`), unused Rust items by
  rustc's `dead_code` through `make rust-lint`; a commented-out block is read in
  review.
- **No duplicated logic that has to be kept in step by hand.** One
  implementation, one home. Where two surfaces must agree byte for byte the
  parity suites are the check, but they report the divergence, they do not
  prevent it; a copy-paste pair that must move together is a design error.
  Nothing in the pipeline detects duplication.
- **Errors are handled at the boundary, not swallowed in the middle.** A failure
  a caller cannot see is a defect
  ([standards](../reference/standards.md#errors)). The boundary is the one place
  allowed to decide what a failure means. No lint in the set flags a blind
  `except` or a discarded result, so this is read in review.
- **A test states behaviour, not the implementation.** It asserts an observable
  — bytes, values, an error class — rather than restating the code path that
  produced it, so that a rewrite of the path keeps the test meaningful. Read in
  review.

## Watching performance

Performance is watched, not gated. A reading — a leaf or stage, its share of an
encode, and the machine it came from — becomes an entry in the
[encode performance finding](../findings/encode-performance.md); it never
becomes a threshold in a test or a budget in CI. The instruments are
`scripts/measure_encode_perf.py` (CLI stage timers, min/median/p95/spread, the
machine identity the numbers belong to, and the load average before and after
each series), `crates/wem-core/tests/stage_timings.rs` (the per-stage split of
one encode, `#[ignore]`d, run with `--ignored --nocapture`), and the RSS
reporters in `crates/wem-core/tests/streaming.rs`. The timing instruments
**report**: an instrument that can fail a build has become a gate, and the
ceiling it enforces belongs in the finding as a reading rather than in the suite
as a limit. Memory is the one exception, and only because it is a different kind
of claim: how much input a session may accumulate is a property of the product,
so `streaming.rs` keeps its RSS ceilings as assertions and prints each reading
beside the ceiling it is held to.

A change to a hot path earns its place before it is written: the redundancy or
invariance established from the code rather than from a percentage, then a
bit-exactness argument for the specific change, and only then the edit. The
measurement that accompanies it is a paired before/after with the run order
alternated and **the machine's load stated** — a naive pair on a busy machine
once reported a 152 ms "after" against a 116 ms "before" for a stage nobody had
touched. No optimization that could move a byte lands until the crate's targets
are green under both feature configurations: `--all-targets` as it builds by
default — scalar, the `parallel` feature being opt-in — and `--all-targets
--features parallel`.

The finding is the ledger, and it is append-only in spirit: a closed item stays
there, marked closed with the number that closed it, because an entry deleted
on completion destroys the only record that the work was ever measured.

## Adding and removing files

Adding a file under `src/wwise_wem/` or packaged data, and the wheel check that
follows it, are in the root
[`AGENTS.md`](../../AGENTS.md#new-file-registration-checklist) and
[`../../src/wwise_wem/AGENTS.md`](../../src/wwise_wem/AGENTS.md#registration-duties).

Removing a mechanism, target, asset or public item is the same kind of change
from the other side, and the documents that describe it are part of it: the same
change updates or deletes those documents and corrects the [map](../README.md).
A document describing something that no longer exists is worse than no document.

Provenance rules — what may be recorded as profile data and what evidence a
value needs — are in [`../reference/profiles.md`](../reference/profiles.md).

## Shared checkout and concurrent lanes

Work happens in several lanes at once — a worktree per task, and often more than
one agent inside the same repository. A checkout is not private, and these are
the rules the failure mode taught us.

- **A branch can change under you.** Between two of your commands another lane
  can create or switch the branch of the worktree you are standing in. Every
  state-changing git command (`merge`, `commit`, `reset`, `checkout`,
  `branch -f`) acts on whatever `HEAD` points at *at that moment*.
- **Read the branch in the same command that writes it.** Chain the check and
  the write (`git status -sb && git merge --ff-only <branch>`); the branch you
  saw in an earlier turn is not evidence. Then verify the result
  (`git log --oneline -1 <branch>`), because a fast-forward that lands on the
  wrong branch still exits 0.
- **Land on `main` as an explicit, verified step.** When `main` is not checked
  out in any worktree, `git branch -f main <sha>` fast-forwards it atomically
  from your own worktree; git refuses when `main` *is* checked out somewhere,
  which is the safety net you want. Confirm with `git log --oneline -1 main`
  before removing the worktree you landed from.
- **Leave other lanes' state alone.** Do not clean, stash, reset or revert files
  you did not create, and do not move a branch that another worktree has checked
  out. Untracked scratch files in a shared tree belong to whoever made them.
- **Finish the loop.** Once the commit is on `main`, remove your worktree and its
  branch (`git worktree remove <path>`, `git branch -d <branch>`). `git branch -d`
  compares against *the branch you are standing on*, not against `main`, so in a
  shared checkout its "not fully merged" refusal is not proof that the commit is
  missing. Decide with `git merge-base --is-ancestor <sha> main`, and only then
  delete with `-D`.

## Conventions

The binding git and provenance rules are the Git discipline and provenance
sections of the root [`AGENTS.md`](../../AGENTS.md). In short: stage explicit
paths only, one logical change per conventional commit (`feat(rust):`,
`fix(profiles):`, `test:`, `docs:`, `ci:`, `chore:`), keep `crates/Cargo.lock`
committed, push only when the human asks, and keep repository surfaces clean-room
phrased — inside `src/wwise_wem/` the marker substrings listed in
[`../../src/wwise_wem/AGENTS.md`](../../src/wwise_wem/AGENTS.md#provenance-vocabulary-checked-by-tests)
must not appear in literals, identifiers, or paths.

## Documentation

The map is [`../README.md`](../README.md), and it is the place to correct when a
document is added or removed. Two writing rules: a result goes into
`findings/` only with reproducible evidence attached, and a method goes into
`methodology/` only after it has actually decided a case. Do not restate
reference material in a finding — link to it.
