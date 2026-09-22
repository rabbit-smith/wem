# src/wwise_wem/ — distribution facade

This package is the thin distribution facade: root API shell, CLI, WAV
adapters, DTOs, and the installed-profile metadata path. The bit-exact
implementation domains live in the development-tree reference package
(`reference/wwise_wem_reference`, see `reference/AGENTS.md`); the kernel is
checked against that tree's byte-for-byte output, never the reverse.

The norms this package is held to — byte-for-byte output, the single execution
path `wwise_wem._core`, transcendentals and float rounding sites, the layer
boundaries, and what "fail loudly" means — are in
[`../../docs/reference/standards.md`](../../docs/reference/standards.md).

## Public surfaces

- Package root exports: listed in
  [`../../docs/reference/public-interface.md`](../../docs/reference/public-interface.md),
  asserted by `tests/parity/test_public_api.py` and the distribution tests.
  Internal module paths are not part of that surface, and the extension's module
  name is owned by the packaging surface, not restated here.
- DTO immutability and the `_f32` rounding sites follow
  [`../../docs/reference/standards.md`](../../docs/reference/standards.md)
  ([Source rules](../../docs/reference/standards.md#source-rules),
  [Determinism](../../docs/reference/standards.md#determinism)).

## Registration duties

Every new module or packaged data file requires, in the same commit:
1. `tests/parity/distribution_allowlist.json` (modules/resources lists),
2. `pyproject.toml` package-data glob when under `data/`,
3. profile data: manifest `resources` entry + SHA-256 + `index.json` re-hash
   (`scripts/generate_frozen_tables.py` shows the canonical update path),
4. the wheel packaging test (`make wheel-smoke`) passing for the installed and
   the zip import path.

This applies to the facade only: reference-tree modules are development-tree
code and must not be added to the allowlist.

## Provenance vocabulary (checked by tests)

The distribution and cleanliness tests reject these **case-insensitive
substrings** anywhere in package `.py`/`.json` text — including identifiers,
docstrings, comments, env var names, and file paths:
`capture`, `fixture`, `the probe`, `research`, `experimental` — plus regex hits
like bare `hook`, `DLL`, `RVA`, hex-backtick addresses, and `/tmp` paths.
Use: `recording/record`, `representative`, `sample`, `site`, `reference`.
This applies to strings inside profile JSON payloads as well.

## Style checks

`make lint` (ruff E4/E7/E9/F/W + mypy baseline with `check_untyped_defs`).
New code: typed signatures, `from __future__ import annotations`, no unused
imports. `tests/` may relax only what the config already relaxes.
