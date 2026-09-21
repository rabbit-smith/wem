# Development

How the repository is laid out, how to build and verify it, and which rules are
load-bearing. The agent-conduct rules live in the layered
[`AGENTS.md`](../../AGENTS.md) set; this page is the human-facing summary of the
same ground.

## Repository map

| Path | Role |
| --- | --- |
| `src/wwise_wem/` | Distribution facade: root API, CLI, adapters, profile registry, and the embedded native extension |
| `crates/` | Rust kernel workspace: `scheduling`, `analysis`, `vorbis`, `container`, `profiles`, `core`, plus the C ABI (`wem-capi`) and the language shells (`wem-python`, `wem-wasm`) |
| `include/wem.h` | C ABI interface the shells mirror 1:1, implemented by `crates/wem-capi` |
| `reference/wwise_wem_reference/` | Bit-exact pure-Python oracle, development tree only, never shipped |
| `src/wwise_wem/data/profiles/` | Immutable packaged profile bundles (setup, codebooks, transforms, analysis, psychoacoustics) |
| `tests/` | `unit/`, `integration/`, `contract/`, `golden/`, and the `data/` regression assets |
| `scripts/` | Regeneration, decoding, fuzz, and packaging tools |
| `examples/`, `js/` | Runnable bindings: Python, Rust, C, Go via cgo, Node and browser via wasm |
| `docs/` | This guide set; see [Documentation map](#documentation-map) |

Dependency direction is one-directional and acyclic: `scheduling` imports
nothing downstream, `analysis`/`vorbis`/`container` never open package
resources, `profiles` is the sole resource owner, and one application layer
assembles the use case. The authoritative rules are in
[`../reference/architecture.md`](../reference/architecture.md), and the same
split applies to both implementations.

## Build

```bash
make native          # maturin develop: build the in-package kernel extension
make build           # wheel into dist/
make wheel-smoke     # install the wheel in a clean venv and encode once
```

`pip install -e .` is equivalent to `make native`. Python-only tooling:
`make lint` (ruff + mypy), `make rust-lint` (clippy, warnings denied).

## Verification ladder

Climb from the cheapest executable target; do not start from the full suite.

1. One `unittest`/`cargo test` case or module.
2. The affected suite: `make stage-contract`, `make 2ch-stress`,
   `make 2ch-long`, `tests/contract/test_frame_pipeline_parity.py`, or one
   crate's tests.
3. The full ladders (`make test`, `cargo test --workspace`) at phase
   boundaries, or whenever the change touches shared state, configuration,
   lockfiles, or generated assets.
4. `make check` before handing work back: lint, rust-lint, the full test
   ladder, and the wheel smoke.

A passing suite is reused. Do not re-run a green suite per task or per agent;
rerun it only on a named invalidator — code, test, data, or configuration that
the suite actually exercises.

### What each target runs

The table lists the local targets and the quantities they check. Every one of
them is a test run; when one goes red, it names what changed, and the answer is
to fix the code or the test rather than to re-record the expectation.

| Target | Checks |
| --- | --- |
| `make golden` | Whole-file golden WEM identity: `SHA-256 17851d26…d35247`, 205 audio packets |
| `tests/contract/test_frame_pipeline_parity.py`, `cargo test -p wem-core --test frame_pipeline_parity` | Per-frame values (scheduling fields, eight analysis stages, floor posts, residue rows, audio packet) against the pure-Python oracle, all 205 frames |
| `make stage-contract` | Per-frame × per-stage pipeline hashes and raw dumps |
| `make 2ch-stress`, `make 2ch-long` | The 2ch stress corpus and the long-run cross-implementation comparison |
| `make fuzz-parity` | Native vs oracle parity under randomized chunking and quality; Python unit, integration and cross-implementation suites run in `make test-fast` |
| `cargo test --workspace` | Rust kernel, C ABI and shell suites, including the geometry-materializer parity suites |
| `make wheel-smoke` | Installed-wheel profile digest chain (payload → manifest → index) and one real encode |
| `make check` | All of the above plus `ruff`, `mypy` and `clippy` |

## Determinism rules

The normative rules are in [`AGENTS.md`](../../AGENTS.md); the four that decide
most reviews:

- No runtime transcendental in an encoder path — values come from profile data or
  frozen tables, and a new geometry needs `scripts/record_tmath.py` →
  `scripts/generate_frozen_tables.py` first.
- Float32 at every assignment point: the Python `_f32` locations mark those
  points, Rust rounds at the same statements, and the parity suites compare the
  resulting values, so summation and butterfly order may not be rearranged.
- Bit patterns travel as integers or little-endian bytes, never via decimal
  strings.
- Generated assets must regenerate byte-stably (run twice, diff empty),
  with `sort_keys` JSON; the kernel stays `wasm32`-scalar-compilable.

## Profile data

A profile owns its complete calibration set — setup, codebooks, transforms,
transient tables, psychoacoustic tables, geometry, and container defaults — under
`src/wwise_wem/data/profiles/<name>/`. `manifest.json` is the single source for
identity, geometry, logical resource names, paths, schemas, and SHA-256 values;
`src/wwise_wem/data/profiles/index.json` selects the default and
checksum-addresses the manifest. Loaders resolve both through package
traversables, so the same inventory works from a source tree, an installed wheel,
or a ZIP import.

Adding any file under `src/wwise_wem/`, or any packaged data, requires:

1. an entry in `tests/contract/distribution_allowlist.json`;
2. package-data glob coverage in `pyproject.toml` for data;
3. for profile data, a manifest `resources` entry with its SHA-256 and a
   re-addressed index, followed by `make wheel-smoke`.

Provenance rules — what may be recorded as profile data and what evidence a
value needs — are in [`../reference/profiles.md`](../reference/profiles.md).

## Documentation map

| Surface | Holds |
| --- | --- |
| [`usage.md`](usage.md) | Install, CLI, per-language entry points |
| this page | Layout, build, test targets, determinism, conventions |
| [`../reference/`](../reference/) | Normative surfaces: architecture, domain model, profiles, public interface |
| [`../findings/`](../findings/) | Evidence records for a completed result, with its retractions and trust boundary |
| [`../methodology/`](../methodology/) | How to run a diagnosis: instruments, oracles, evidence discipline |
| [`../roadmap.md`](../roadmap.md) | What is proven today, what is left |
| `AGENTS.md` (root and per subtree) | Binding agent-conduct rules |

Write a result into `findings/` only with reproducible evidence attached; write
a method into `methodology/` only after it has actually decided a case. Do not
restate reference material in a finding — link to it.

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

The binding rules are the Git discipline and provenance-hygiene sections of
[`AGENTS.md`](../../AGENTS.md). In short: stage explicit paths only, one logical
change per conventional commit (`feat(rust):`, `fix(profiles):`, `test:`,
`docs:`, `ci:`, `chore:`), keep `crates/Cargo.lock` committed, push only when the
human asks, and keep repository surfaces clean-room phrased — inside
`src/wwise_wem/` the marker substrings enforced by
`tests/contract/distribution_allowlist.json` must not appear in literals,
identifiers, or paths.