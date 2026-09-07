# src/wwise_wem/ — distribution facade

This package is the thin distribution facade: root API shell, engine
switch, CLI, WAV adapters, DTOs, and the installed-profile metadata path.
The bit-exact implementation domains live in the development-tree reference
package (`reference/wwise_wem_reference`, see `reference/AGENTS.md`); its
byte-for-byte outputs remain the project's source of truth, and the Rust
kernel is verified against it, never the reverse.

## Hard rules

1. **Output bytes are frozen.** Any edit must keep `make frame-contract`,
   `make stage-contract`, and `make golden` green. If a change alters any
   digest, it is a project decision, not a code change — stop and escalate.
2. **No direct transcendentals.** `math.sin/cos/log/log10/pow/exp` outside
   `wwise_wem_reference._tmath` is prohibited in encoder paths; all such
   calls go through the named site entries and, for exact-profile paths,
   through `FrozenMathTables` injection. New runtime transcendental inputs
   require the record → freeze workflow first (see `docs/profiles.md`).
3. **Reference imports stay lazy.** Facade modules import
   `wwise_wem_reference` only through `wwise_wem._reference.reference_module`
   inside the call paths that need it; importing the package root must never
   pull in reference modules (the wheel does not ship them).
3. **Layer boundaries.** Follow `docs/architecture.md` import rules; `analysis`,
   `vorbis`, `container` never read package resources; `profiles` is the only
   loader. Receiving typed tables is the only configuration mechanism.
4. **Fail loudly.** Input/boundary errors are explicit `ValueError`s at the
   point of validation (mirror existing style). No silent defaults, no
   `try/except: pass`, no `assert` for invariants that `-O` would erase.

## Compatibility surfaces

- Package root exports (`docs/public-interface.md`): additive only; internal
  module paths are explicitly not contract.
- `_f32` statement placement is normative for the Rust port — when touching a
  numeric function, keep every rounding site where it is unless a contract
  run proves the output bytes identical.
- Immutable DTOs: frozen dataclasses + `MappingProxyType` per existing pattern;
  no mutable state escaping one `AnalysisSession`.

## Registration duties

Every new module or packaged data file requires, in the same commit:
1. `tests/contract/distribution_allowlist.json` (modules/resources lists),
2. `pyproject.toml` package-data glob when under `data/`,
3. profile data: manifest `resources` entry + SHA-256 + `index.json` re-hash
   (`scripts/generate_frozen_tables.py` shows the canonical update path),
4. `make wheel-smoke` green (installed + zip import paths).

This applies to the facade only: reference-tree modules are development-tree
code and must not be added to the allowlist.

## Provenance vocabulary (contract-enforced)

The distribution/cleanliness contracts reject these **case-insensitive
substrings** anywhere in package `.py`/`.json` text — including identifiers,
docstrings, comments, env var names, and file paths:
`capture`, `fixture`, `the probe`, `research`, `experimental` — plus regex hits
like bare `hook`, `DLL`, `RVA`, hex-backtick addresses, and `/tmp` paths.
Use: `recording/record`, `representative`, `sample`, `site`, `reference`.
This applies to strings inside profile JSON payloads as well.

## Style gates

`make lint` (ruff E4/E7/E9/F/W + mypy baseline with `check_untyped_defs`).
New code: typed signatures, `from __future__ import annotations`, no unused
imports. `tests/` may relax only what the config already relaxes.
