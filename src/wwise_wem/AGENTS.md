# src/wwise_wem/ — distribution facade

This package is the thin distribution facade: root API shell, CLI, WAV
adapters, DTOs, the decode driver, and the profile identity type. The bit-exact
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
  asserted by `tests/parity/test_public_surface.py` and the distribution tests.
  Internal module paths are not part of that surface, and the extension's module
  name is owned by the packaging surface, not restated here.
- `decode` is the one decoding entry point: a plain function (never a generator
  function — its body must run at call time) returning the decode result
  documented in
  [`../../docs/reference/public-interface.md`](../../docs/reference/public-interface.md#decode).
  No session type enters the package root. The design behind it, including what
  the call resolves eagerly and why, is in
  [`../../docs/reference/decoding.md`](../../docs/reference/decoding.md).
- DTO immutability and the `_f32` rounding sites follow
  [`../../docs/reference/standards.md`](../../docs/reference/standards.md)
  ([Source rules](../../docs/reference/standards.md#source-rules),
  [Determinism](../../docs/reference/standards.md#determinism)).

## Registration duties

Every new module or packaged data file requires, in the same commit:
1. `tests/parity/distribution_allowlist.json` (modules/resources lists),
2. `pyproject.toml` package-data glob when under `data/` — there is no `data/`:
   profile values are Rust constants compiled into the extension, so the
   resource list stays empty,
3. profile data: generated into the carrier by
   `scripts/generate_profile_code.py` from `corpus/profiles/` (untracked
   development material); `scripts/generate_frozen_tables.py` writes that
   material, it does not touch the package,
4. the wheel packaging test (`python3 scripts/wheel_smoke.py`) passing for the
   installed and the zip import path.

This applies to the facade only: reference-tree modules are development-tree
code and must not be added to the allowlist.

## Provenance hygiene (checked by tests)

Package text carries no development-process provenance: no disassembler symbol
names or image addresses, no binary module or section labels, no machine-local
absolute paths, no internal lane or report references. The patterns are defined
once, in
[`../../scripts/check_provenance.py`](../../scripts/check_provenance.py); read
them there rather than restating the list here, because a restated list is
itself a record of what was removed.

Use `recording`/`record`, `representative`, `sample`, `site`, `reference`, and
**"the paired build"** for the build this implementation is measured against.

The rule covers the recorded profile material under `corpus/profiles/` as well —
it is the source the carrier is generated from, even though it does not ship.

## Style checks

`ruff check src reference tests scripts` (ruff E4/E7/E9/F/W) and
`mypy --no-site-packages src reference` (the mypy baseline with
`check_untyped_defs`); both run inside `make test`.
New code: typed signatures, `from __future__ import annotations`, no unused
imports. `tests/` may relax only what the config already relaxes.
