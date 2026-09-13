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
| Geometry-materializer parity: ported builder == registered 6ch surfaces == kernel `psy_geom*` | `tests/contract/test_geometry_materializer_contract.py` + `cargo test -p wem-analysis` parity suites |
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
| C ABI core surface | [`include/wem.h`](include/wem.h) (contract) + `crates/wem-capi` (implementation) |
| Distribution facade | [`src/wwise_wem/AGENTS.md`](src/wwise_wem/AGENTS.md) |
| Python reference implementation | [`reference/AGENTS.md`](reference/AGENTS.md) |
| Contracts & assets | [`tests/AGENTS.md`](tests/AGENTS.md) |
| Tooling scripts | [`scripts/AGENTS.md`](scripts/AGENTS.md) |

## Integration topology (normative)

- The Rust kernel is the sole integration point; its **C ABI core
  surface** (`crates/wem-capi`, contracted by `include/wem.h`) is the
  canonical interface. The lifecycle (Init -> chunk* -> Finish), the
  reply framing (seq 0 = setup packet, then audio packets), and the
  error codes are pinned in `include/wem.h` and implemented 1:1 by
  `wem-capi`; every language binding conforms to that contract.
- Every binding is a **parallel shell** over the kernel, never a
  parallel implementation: PyO3 today (in-package `wwise_wem._core`),
  Go via cgo (`examples/go-cgo` is the reference shell), a wasm build
  next. Shells map the contract 1:1 and own no numerics; a new language
  integrates by writing a shim over the C ABI — never by changing the
  kernel for it.
- Same-process consumers bind the kernel **directly**: never route a
  call through RPC or a serialization hop, and never keep a second
  integration surface in parallel with the C ABI.
- Streaming chunk boundaries must not affect output bytes; any
  client-side framing rule added later needs a parity case proving that.
- Browsers use the wasm build of the same core, not a remote service.

## Governing documents

`docs/architecture.md` (layers), `docs/domain-model.md` (vocabulary — use these
terms in code and messages), `docs/profiles.md` (profile & frozen-table
ownership), `docs/public-interface.md` (compatibility surface). Vocabulary from
the domain model is normative; do not invent parallel names for defined concepts.

## Knowledge gate

Before non-trivial work, search project knowledge (`maestro search …`). After
verified work, stage durable lessons (red-line discoveries, failure modes,
non-obvious decisions) rather than process narration.
