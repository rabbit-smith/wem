# AGENTS.md — wwise-wem

Standalone bit-exact implementation of Wwise 2013.2 Vorbis WAV→WEM encoding.
Correctness is defined by exact bytes, not by behavior similarity. Read this
file first; subtree rules live in the child `AGENTS.md` files listed below.

## What the tests cover

| Claim | Where it is checked |
|---|---|
| Reference WEM `SHA-256 17851d26c6210b85e498ae0452d2562d7b9e2c3e9e795c459656b9c9d8d35247` | `make wem-bytes` |
| Per-frame × per-stage pipeline hashes + representative raw dumps | `tests/parity/test_stage_pipeline.py` over `tests/data/stage-records/` |
| Package-root public exports (see `docs/reference/public-interface.md`) | `tests/parity/test_public_api.py`; the export list in `tests/parity/test_distribution.py`; wheel smoke |
| Geometry-materializer parity: ported builder == registered 6ch surfaces == kernel `psy_geom*` | `tests/parity/test_geometry_materializer_parity.py` + `cargo test -p wem-analysis` parity suites |
| Profile data digest chain: payload → manifest SHA → index SHA | `bundle.verify_all`, wheel smoke |

Running those suites is what establishes each claim. A refactor is checked
against them: when one reports a difference, the difference says what moved —
either the code is wrong, or the expected bytes have changed as part of the
work, and those are two different changes.

## Verification ladder (reuse before rerunning)

1. Smallest executable target first: one `unittest`/`cargo test` case or module.
2. Then the affected suite (`tests/parity/test_stage_pipeline.py` / one crate test /
   `tests/parity/test_frame_pipeline_parity.py`).
3. Full `make test` / `cargo test --workspace` only at phase boundaries or when
   the change touches shared state, configuration, lockfiles, or generated assets.
4. A passing suite is reused — do not re-run it per task or per agent; rerun only
   on a named material invalidator (code/test/data/config the suite exercises).

## Determinism rules (project-wide)

- No runtime transcendental (`sin/cos/log/log10/pow/exp`) in any encoder path.
  All such values come from profile data (`FrozenMathTables`, static trig
  banks). New geometry requiring a new transcendental input must first extend
  `scripts/record_tmath.py` → `scripts/generate_frozen_tables.py` and ship it
  as a checksummed profile resource; encoder code may only table-read it.
- Float semantics: values are float32 at every assignment point (the Python
  `_f32` call sites mark those points; Rust must round at the same statements).
  Summation and butterfly ordering are fixed — the per-frame parity tests compare
  the resulting values, and reordering them changes what they read.
- Bit patterns travel as integers or little-endian bytes, never via decimal strings.
- Portability floor: the kernel must stay compilable for `wasm32-unknown-unknown`
  in a scalar configuration — parallel acceleration (e.g. rayon) lives behind a
  default-on, off-able feature, and profile bytes must be consumable via an
  I/O-free entry (`from_resources`), not only the filesystem loader.
- Generated assets (`tests/data/stage-records/`, the frozen tables) must come out
  byte-identical when regenerated: run the generator twice and the diff is empty,
  JSON is written with `sort_keys`, and file names carry explicit endianness.

## Dependency direction

The import graph is one-directional and acyclic; the authoritative rule list is
`docs/reference/architecture.md`. Summary: `scheduling` imports nothing downstream;
`analysis`/`vorbis`/`container` never open package resources; `profiles` is the
sole resource owner; one application/core layer alone assembles the use case.
This applies identically to Python (oracle) and Rust (kernel) crates.

## Git discipline

- Stage explicit paths only (`git add <file…>`); never blanket `-A`.
- Conventional commits with scopes: `feat(rust):`, `test:`, `fix(profiles):`,
  `docs:`, `ci:`, `chore:`. One logical change per commit.
- `crates/Cargo.lock` is committed (reproducible bit-exact builds outrank the
  library convention). Build outputs, native artifacts, harness directories
  (`.workflow/`, `target/`, `*.so`, wheels) stay ignored — check `.gitignore`
  before adding new file kinds; anything generated under `src/`, `tests/data/`,
  or repo root must be intentional.
- Push only when the human asks. History is never rewritten retroactively.
- Background work lanes do not commit; the orchestrator commits after
  independent verification.
- A checkout is shared. Another lane can create or switch the branch of the
  worktree you are standing in between two of your commands, and `merge`,
  `commit`, `reset` and `branch -f` act on whatever `HEAD` points at right then.
  Read the branch in the same command that writes (`git status -sb && git merge …`),
  confirm afterwards that the ref you meant to move actually moved — a
  fast-forward of the wrong branch still exits 0 — and never clean, stash, reset
  or revert files another lane left in the tree. Procedure:
  [`docs/guides/development.md`](docs/guides/development.md#shared-checkout-and-concurrent-lanes).

## Provenance hygiene

Repository surfaces are clean-room phrased. Inside the `src/wwise_wem` package,
string literals, identifiers, and paths must not contain the forbidden marker
substrings enforced by `tests/parity/distribution_allowlist.json` and the
cleanliness tests (`capture`, `fixture`, `the probe`, `research`, `experimental`
— case-insensitive, substring level in package text; docs/README are exempt).
Preferred vocabulary: `recording`/`record`, `representative`, `sample`, `site`.

## New-file registration checklist

Adding any file under `src/wwise_wem/` or packaged data requires:
1. entry in `tests/parity/distribution_allowlist.json` (modules/resources),
2. package-data glob coverage in `pyproject.toml` (for data),
3. if it is profile data: manifest `resources` entry with SHA-256 and index
   re-address, then the wheel packaging test (`make wheel-smoke`), which installs
   the built wheel in a clean venv and encodes once through it.

## Subtree guides

| Area | Guide |
|---|---|
| Rust kernel | [`crates/AGENTS.md`](crates/AGENTS.md) |
| C ABI core surface | [`include/wem.h`](include/wem.h) (interface the shells mirror) + `crates/wem-capi` (implementation) |
| Distribution facade | [`src/wwise_wem/AGENTS.md`](src/wwise_wem/AGENTS.md) |
| Python reference implementation | [`reference/AGENTS.md`](reference/AGENTS.md) |
| Test suites & assets | [`tests/AGENTS.md`](tests/AGENTS.md) |
| Tooling scripts | [`scripts/AGENTS.md`](scripts/AGENTS.md) |
| Documentation set | [`docs/README.md`](docs/README.md) |

## Integration topology

- The Rust kernel is the sole integration point; its **C ABI core
  surface** is `crates/wem-capi`, and [`include/wem.h`](include/wem.h) is the
  interface it implements. The lifecycle (Init -> chunk* -> Finish), the
  reply framing (seq 0 = setup packet, then audio packets), and the
  error codes are declared in `include/wem.h` and implemented 1:1 by
  `wem-capi`; every language binding mirrors that interface.
- Every binding is a **parallel shell** over the kernel, never a
  parallel implementation: PyO3 today (in-package `wwise_wem._core`),
  Go via cgo (`examples/go-cgo` is the reference shell), a wasm build
  next. Shells mirror the interface 1:1 and own no numerics; a new language
  integrates by writing a shim over the C ABI — never by changing the
  kernel for it.
- Same-process consumers bind the kernel **directly**: never route a
  call through RPC or a serialization hop, and never keep a second
  integration surface in parallel with the C ABI.
- Streaming chunk boundaries must not affect output bytes; any
  client-side framing rule added later needs a parity case proving that.
- Browsers use the wasm build of the same core, not a remote service.

## Governing documents

`docs/reference/architecture.md` (layers), `docs/reference/domain-model.md` (vocabulary — use these
terms in code and messages), `docs/reference/profiles.md` (profile & frozen-table
ownership), `docs/reference/public-interface.md` (package exports, the `encode`
API, result types, and the CLI). The domain model's terms are the ones to use; do
not invent parallel names for defined concepts.

The rest of the set is split by purpose: `docs/guides/` holds task instructions,
`docs/findings/` holds the evidence record for a completed result (root causes,
accounting, retractions, trust boundary), and `docs/methodology/` holds reusable
method. Before chasing any divergence from the paired build, read
`docs/methodology/byte-exact-diagnosis.md`; when such a chase concludes, write
the result into `docs/findings/` rather than into the roadmap.

## Project knowledge

Before non-trivial work, search project knowledge (`maestro search …`). After
verified work, stage durable lessons (failure modes, non-obvious decisions,
surprising constraints) rather than process narration.