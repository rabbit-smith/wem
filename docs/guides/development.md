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
make wasm-build      # wasm-pack: both shell packages (js/pkg, js/pkg-node)
make clean           # remove build, test and bytecode artifacts
```

`pip install -e .` is equivalent to `make native`. These four produce or remove
artifacts and judge nothing: `make native` writes the extension every
byte-producing call needs, and `make wasm-build` writes the packages the Node
check and the browser demo read. `make test` depends on both, so a fresh
checkout needs no build step before it.

The tools these commands call — `ruff`, `mypy`, `maturin` — resolve from
`.venv/bin` when the venv exists and from `PATH` otherwise, so a checkout whose
venv is not activated runs the same commands as an activated one. They are not
all part of an editable install: `pip install -e ".[dev]"` brings `ruff` and
`mypy`, and `pip install maturin` is what `make native` needs — pip fetches the
maturin build backend into an isolated environment for the install itself and
does not leave the command in the venv.

### Toolchain

`rustdoc` and `clippy-driver` can resolve from a different toolchain than
`cargo` and `rustc` — on the machine this was written on, Homebrew's — and the
mix fails with a spurious E0514 (*"found crate … compiled by an incompatible
version of rustc"*, with both sides printing the same version number), because
the foreign rustdoc and clippy-driver read metadata written by a different build
of that rustc. `cargo doc` and `cargo clippy` therefore need the rustup
toolchain's `bin` ahead of everything else.

The Makefile exports that once, for every command it runs, and derives it from
`rustup show active-toolchain` rather than hard-coding a triple: it prepends
that toolchain's `bin` to `PATH` and points `RUSTDOC` at the same toolchain's
`rustdoc`. It changes no compiler — those are the binaries the rustup shim in
`~/.cargo/bin` already dispatches to — it is skipped entirely on a machine
without that directory (CI, a container whose rustup lives elsewhere), and it
applies only to the commands the makefile runs, never to the developer's shell.

## Verification ladder

Climb from the cheapest executable target; do not start from the full suite.

1. One `unittest`/`cargo test` case or module.
2. The affected suite: `make test-fast` (the unit, integration and
   cross-implementation suites), `make 2ch-long`,
   `tests/parity/test_frame_pipeline_parity.py`, or one crate's tests.
3. `make test` before handing work back, and at phase boundaries or whenever a
   change touches shared state, configuration, lockfiles, or generated assets.
   It is the whole verdict — the static checks, every suite, the wheel and the
   wasm shell's bytes — so there is no second command to remember.

Before any of that, `git config core.hooksPath .githooks` arms the local
pre-commit layer: the same provenance patterns and asset guard the CI job runs,
plus `ruff` on the staged files, inside the couple of seconds a hook can afford.
It is early feedback and never the verdict — [`hooks.md`](hooks.md) says what is
deliberately absent from it and why that matters more than coverage here.

A passing suite is reused. Do not re-run a green suite per task or per agent;
rerun it only on a named invalidator — code, test, data, or configuration that
the suite actually exercises. `python3 scripts/fuzz_diff_parity.py --pr` is the
one to reach for when a change touches streaming, scheduling, profile assembly,
or packet packing; the nightly job runs the same tool with `--full`.

## What `make test` runs

One target, every check, cheapest first. Each line below is a command in the
`test` recipe, in the order it runs; the first failure stops the run and names
itself.

| Step | Checks |
| --- | --- |
| `ruff check src reference tests scripts`, `mypy --no-site-packages src reference` | Python lint and the typed baseline |
| `cd crates && cargo fmt --all --check` | Rust formatting |
| `cd crates && cargo clippy --workspace --all-targets -- -D warnings` | The Rust workspace lints with warnings denied |
| `cd crates && cargo clippy -p wem-analysis -p wem-core --all-targets --features parallel -- -D warnings` | The same lints over the opt-in configuration: `--workspace --all-targets` does not lint the bin the `parallel` feature gates |
| `cd crates && RUSTDOCFLAGS="-D warnings" cargo doc --workspace --no-deps` | Rustdoc over the workspace with warnings denied. It resolves every intra-doc link, which clippy does not read: a public item whose documentation links to a private one compiles, lints and tests clean, and only this step reports it |
| `make test-fast` | The Python suites by discovery: `tests/unit`, `tests/integration`, `tests/parity` (module behavior, CLI/API/adapter end paths, pipeline invariants and cross-implementation parity) |
| `python3 scripts/native_smoke.py` | The facade's streaming path (five uneven chunks), its one-shot path, and the error-code mapping, each byte-exact against `tests/fixtures/reference.wem` |
| `make wem-bytes` | Whole-file reference WEM parity: the encoder's bytes equal the committed `tests/fixtures/reference.wem`, byte for byte (205 audio packets) |
| `make 2ch-long` | The long-run cross-implementation comparison, one 2ch/48 kHz case across the native, reference and streaming paths |
| `cd crates && cargo test --workspace --all-targets` | The Rust kernel, C ABI and shell suites in the default configuration — scalar, the `parallel` feature being opt-in — including `frame_pipeline_parity` (per-frame values against the oracle, all 205 frames), `container_codec` (the native container builder against the oracle's, byte for byte, plus the degenerate payloads it must refuse) and the geometry-materializer parity suites |
| `cd crates && cargo test -p wem-analysis -p wem-core --all-targets --features parallel` | The same suites in the opt-in configuration our CLI requires, so that the parallel path is built and tested and not only the scalar one |
| `python3 scripts/fuzz_diff_parity.py --pr` | Native vs oracle parity under randomized chunking and quality, on the fixed seed set |
| `python3 scripts/wheel_smoke.py` | Installed-wheel inventory (facade + native engine, no profile data), the zip-import metadata path, and one real encode in a clean venv. Run with an empty `PYTHONPATH`, so the reference tree cannot be imported inside the "installed wheel" venv |
| `make wasm-build`, `node js/test-node.mjs` | wasm-pack builds both shell packages, then the shell's bytes against `tests/fixtures/reference.wem` (one-shot, three chunkings) plus the selection and error-code mapping |

The same ladder from the test tree's point of view — layer, command and run
order — is in [`../../tests/AGENTS.md`](../../tests/AGENTS.md).

Two layer targets stay named, because the build names the path they run and
`unittest discover` reaches neither: `make wem-bytes`
(`tests/whole_file/test_whole_file.py`) and `make 2ch-long`
(`tests/parity/two_channel_long_run.py`). `make test-fast` is the third, and the
one to use while iterating: the same three discovery runs `make test` starts
with, and nothing else.

Everything else a developer may want alone is the underlying tool's own
command, shown in the table above: one suite, one tool, no name for it. The
performance instruments are not in this ladder at all
([Watching performance](#watching-performance)), and two checks stay in CI
because they need toolchains the ladder does not: the C client and the Go cgo
binding, which stream the fixture through `include/wem.h` and compare the bytes
(`.github/workflows/test.yml`, the `capi` job).

## Code-writing standards

What a change is expected to look like, beyond passing the tests. Where a check
exists it is named; where it does not, the rule is read in review. Formatting is
not a review topic: the formatting and lint steps of `make test` decide it.

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
  rustc's `dead_code` through the clippy step of `make test`; a commented-out
  block is read in review.
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

Performance is watched, not held to a number. A reading — a leaf or stage, its
share of an encode, and the machine it came from — becomes an entry in the
[encode performance finding](../findings/encode-performance.md) or the
[decode performance finding](../findings/decode-performance.md); it never
becomes a threshold in a test or a budget in CI. The instruments are
`scripts/measure_encode_perf.py` and `scripts/measure_decode_perf.py` (stage
timers, min/median/p95/spread, the machine identity the numbers belong to, and
the load average before and after each series),
`crates/wem-core/tests/stage_timings.rs` and its decode counterpart (the
per-stage split of one run, `#[ignore]`d, run with `--ignored --nocapture`), and
the RSS reporters in `crates/wem-core/tests/streaming.rs`. `make benchmark` runs
the two scripts over a release build it makes first; it is the one name this tree
gives a tool invocation, because the two are always read together — encode and
decode, one machine, one load. The measurements read a release build, which is
not the build the suites use:

```bash
make benchmark                          # both scripts, release build first
python3 scripts/measure_encode_perf.py  # or each one on its own
python3 scripts/measure_decode_perf.py
```

The CLI's own timer is a separate one-shot, worth it when one fixture encode is
all you want to see:

```bash
cd crates && cargo build --release -p wem-core --features parallel   # the CLI needs the feature
cd crates && target/release/wwise-wem ../tests/fixtures/input.wav --output /dev/null --time
```

The timing instruments **report**: an instrument that can fail a build has
become a limit, and the ceiling it enforces belongs in the finding as a reading
rather than in the suite as a threshold. Memory is the one exception, and only
because it is a different kind of claim: how much input a session may accumulate
is a property of the product, so `streaming.rs` keeps its RSS ceilings as
assertions and prints each reading beside the ceiling it is held to.

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
The same rule covers the commands a document names: a command is written down
only while it runs today and selects what the text claims about it, so a target
that goes away takes every mention of it with it.

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