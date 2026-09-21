# reference/ — pure-Python reference implementation (test-time oracle)

This tree is the bit-exact oracle. The native kernel is checked against its
output bytes, never the reverse.
It is a **test asset, not an engine**: the distributed facade has a single
execution path (the native extension `wwise_wem._core`) and never imports
this package at runtime.

## Status

- **Not distributed.** It is not part of the wheel and not listed in
  `tests/contract/distribution_allowlist.json`. The development tree drives
  every test target with `PYTHONPATH=src:reference` (see Makefile).
- **Imported directly by tests and capture tooling only.** Parity suites,
  the frame/stage comparisons, and the container capture call
  `wwise_wem_reference.python_engine` directly and compare its bytes against
  the kernel and the golden fixtures; parity is asserted by those tests, not a
  runtime switch.
- **Two-way naming.** Implementation domains here import facade DTOs and
  profile-metadata types (`wwise_wem.model`, `wwise_wem.profiles.*`) by
  absolute name; everything else resolves by in-package relative imports.

## Hard rules

1. **Byte-for-byte output.** `make stage-contract`, `make golden` and the
   core-oracle parity suite (`tests/integration/test_core_oracle_golden.py`)
   compare this tree's bytes against the kernel and the committed reference. A
   failure names the value or the byte range that differs; the fix is either in
   the code or in the expected bytes, and which one is a decision about the
   work, not a re-record.
2. **No direct transcendentals.** `math.sin/cos/log/log10/pow/exp` outside
   `wwise_wem_reference._tmath` is prohibited in encoder paths; all such
   calls go through the named site entries and, for exact-profile paths,
   through `FrozenMathTables` injection (see `docs/reference/profiles.md`).
3. **Layer boundaries.** Follow `docs/reference/architecture.md` import rules inside
   this tree: `analysis`, `vorbis`, `container` never read package
   resources; the reference `profiles` loaders are the only resource
   readers besides the facade's installed-profile bundle loader.
4. **Fail loudly.** Input/boundary errors are explicit `ValueError`s at
   the point of validation. No silent defaults, no `try/except: pass`, no
   `assert` for invariants that `-O` would erase.

## Style checks

`make lint` (ruff E4/E7/E9/F/W + mypy baseline with `check_untyped_defs`)
covers this tree alongside `src/`, `tests/`, and `scripts/`.
