# AGENTS.md — wwise-wem

Standalone bit-exact implementation of Wwise 2013.2 Vorbis WAV→WEM encoding.
Correctness is defined by exact bytes, not by behavior similarity. Read this
file first; subtree rules live in the child `AGENTS.md` files listed below.

## Red lines (never change without explicit human approval)

| Invariant | Where enforced |
|---|---|
| Golden WEM `SHA-256 17851d26c6210b85e498ae0452d2562d7b9e2c3e9e795c459656b9c9d8d35247` | `make golden` |
| 205-frame per-frame hash contract | `make frame-contract` |
| Per-frame × per-stage pipeline hashes + representative raw dumps | `make stage-contract` (`tests/data/stage-golden/`) |
| Package-root public exports (see `docs/public-interface.md`) | distribution + wheel smoke |
| Profile data digest chain: payload → manifest SHA → index SHA | `bundle.verify_all`, wheel smoke |

Any refactor is valid only while every gate above still passes unchanged.

## Verification ladder (reuse before rerunning)

1. Smallest executable target first: one `unittest`/`cargo test` case or module.
2. Then the affected gate (`make frame-contract` / `stage-contract` / one crate test).
3. Full `make test` / `cargo test --workspace` only at phase boundaries or when
   the change touches shared state, configuration, lockfiles, or generated assets.
4. A passing gate is reused — do not re-earn it per task or per agent; rerun only
   on a named material invalidator (code/test/data/config the gate exercises).

## Determinism rules (project-wide)

- No runtime transcendental (`sin/cos/log/log10/pow/exp`) in any encoder path.
  All such values come from profile data (`FrozenMathTables`, static trig
  banks). New geometry requiring a new transcendental input must first extend
  `scripts/record_tmath.py` → `scripts/generate_frozen_tables.py` and ship it
  as a checksummed profile resource; encoder code may only table-read it.
- Float semantics: values are float32 at every assignment point (Python
  `_f32` locations are normative; Rust must round at the same statements).
  Summation and butterfly ordering are part of the contract — never reorder.
- Bit patterns travel as integers or little-endian bytes, never via decimal strings.
- Portability floor: the kernel must stay compilable for `wasm32-unknown-unknown`
  in a scalar configuration — parallel acceleration (e.g. rayon) lives behind a
  default-on, off-able feature, and profile bytes must be consumable via an
  I/O-free entry (`from_resources`), not only the filesystem loader.
- Generated contract assets must be byte-stable across regeneration (run twice,
  diff empty) with `sort_keys` JSON and explicit endianness in file names.

## Dependency direction

The import graph is one-directional and acyclic; the authoritative rule list is
`docs/architecture.md`. Summary: `scheduling` imports nothing downstream;
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

## Provenance hygiene

Repository surfaces are clean-room phrased. Inside the `src/wwise_wem` package,
string literals, identifiers, and paths must not contain the forbidden marker
substrings enforced by `tests/contract/distribution_allowlist.json` and the
cleanliness contract (`capture`, `fixture`, `the probe`, `research`, `experimental`
— case-insensitive, substring level in package text; docs/README are exempt).
Preferred vocabulary: `recording`/`record`, `representative`, `sample`, `site`.

## New-file registration checklist

Adding any file under `src/wwise_wem/` or packaged data requires:
1. entry in `tests/contract/distribution_allowlist.json` (modules/resources),
2. package-data glob coverage in `pyproject.toml` (for data),
3. if it is profile data: manifest `resources` entry with SHA-256 and index
   re-address, followed by `make wheel-smoke`.

## Subtree guides

| Area | Guide |
|---|---|
| Rust kernel | [`crates/AGENTS.md`](crates/AGENTS.md) |
| Distribution facade | [`src/wwise_wem/AGENTS.md`](src/wwise_wem/AGENTS.md) |
| Python reference implementation | [`reference/AGENTS.md`](reference/AGENTS.md) |
| Contracts & assets | [`tests/AGENTS.md`](tests/AGENTS.md) |
| gRPC IDL | [`proto/AGENTS.md`](proto/AGENTS.md) |
| Tooling scripts | [`scripts/AGENTS.md`](scripts/AGENTS.md) |

## Integration topology (normative)

- In-process consumers (Python today; Node/Go libraries later) bind the Rust
  kernel **directly** (PyO3-style FFI or native libraries). Never route a
  same-process call through gRPC or any serialization hop.
- `proto/wwise/v1` is the canonical structural contract: type names, streaming
  lifecycle, packet framing, and error codes in every binding mirror it, even
  where no RPC transport exists.
- gRPC (`wem-server`) serves only out-of-process boundaries: non-linking
  languages, remote services, and clients that cannot embed the kernel. Browsers
  use the wasm build of the same core, not RPC.

## Governing documents

`docs/architecture.md` (layers), `docs/domain-model.md` (vocabulary — use these
terms in code and messages), `docs/profiles.md` (profile & frozen-table
ownership), `docs/public-interface.md` (compatibility surface). Vocabulary from
the domain model is normative; do not invent parallel names for defined concepts.

## Knowledge gate

Before non-trivial work, search project knowledge (`maestro search …`). After
verified work, stage durable lessons (red-line discoveries, failure modes,
non-obvious decisions) rather than process narration.
