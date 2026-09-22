# AGENTS.md — wwise-wem

Standalone bit-exact implementation of Wwise 2013.2 Vorbis WAV→WEM encoding.
Read this file first; subtree rules live in the child `AGENTS.md` files listed
below.

## Product norms

What the system must be — bit-exactness and what establishes it, determinism and
float semantics, bit-pattern transport, layers and dependency direction, errors
and panics, caller streams and ambient state, profile data ownership, the
integration topology, the portability floor — is in
[`docs/reference/standards.md`](docs/reference/standards.md). This file holds the
working rules: what to read, the verification ladder, git discipline and the
registration duties.

## Verification ladder (reuse before rerunning)

1. Smallest executable target first: one `unittest`/`cargo test` case or module.
2. Then the affected suite (`tests/parity/test_stage_pipeline.py` / one crate test /
   `tests/parity/test_frame_pipeline_parity.py`).
3. Full `make test` / `cargo test --workspace` only at phase boundaries or when
   the change touches shared state, configuration, lockfiles, or generated assets.
4. A passing suite is reused — do not re-run it per task or per agent; rerun only
   on a named material invalidator (code/test/data/config the suite exercises).

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

Repository surfaces are clean-room phrased. The marker substrings the
distribution and cleanliness suites reject anywhere in package text, and the
vocabulary to use instead, are listed in
[`src/wwise_wem/AGENTS.md`](src/wwise_wem/AGENTS.md#provenance-vocabulary-checked-by-tests)
(docs and README are exempt from the marker rule).

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

## Governing documents

[`docs/reference/standards.md`](docs/reference/standards.md) (the product norms
and the test that establishes each), `docs/reference/architecture.md` (layers),
`docs/reference/domain-model.md` (vocabulary — use these terms in code and
messages), `docs/reference/profiles.md` (profile & frozen-table ownership),
`docs/reference/public-interface.md` (package exports, the `encode` API, result
types, and the CLI), `docs/guides/development.md` (how work is done here: the
verification ladder, the shared checkout, commit discipline and the code-writing
standards). The domain model's terms are the ones to use; do not invent parallel
names for defined concepts.

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
