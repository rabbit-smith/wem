# reference/ — pure-Python reference implementation

This tree is the bit-exact oracle. Its output bytes are the project's source
of truth; the native kernel is verified against it, never the reverse.

## Status

- **Not distributed.** It is not part of the wheel and not listed in
  `tests/contract/distribution_allowlist.json`. The development tree drives
  every test target with `PYTHONPATH=src:reference` (see Makefile).
- **Imported lazily by the facade.** `wwise_wem._reference` resolves modules
  from here only when a byte-producing call needs the pure-Python engine
  (or template container parsing); a facade without this tree reports a
  clear `ImportError` instead of running a half-built path.
- **Two-way naming.** Implementation domains here import facade DTOs and
  profile-metadata types (`wwise_wem.model`, `wwise_wem.profiles.*`) by
  absolute name; everything else resolves by in-package relative imports.

## Hard rules

1. **Output bytes are frozen.** Any edit must keep `make frame-contract`,
   `make stage-contract`, and `make golden` green, and the dual-engine
   suite (`tests/integration/test_dual_engine_golden.py`) byte-identical.
   If a change alters any digest, it is a project decision, not a code
   change — stop and escalate.
2. **No direct transcendentals.** `math.sin/cos/log/log10/pow/exp` outside
   `wwise_wem_reference._tmath` is prohibited in encoder paths; all such
   calls go through the named site entries and, for exact-profile paths,
   through `FrozenMathTables` injection (see `docs/profiles.md`).
3. **Layer boundaries.** Follow `docs/architecture.md` import rules inside
   this tree: `analysis`, `vorbis`, `container` never read package
   resources; the reference `profiles` loaders are the only resource
   readers besides the facade's installed-profile bundle loader.
4. **Fail loudly.** Input/boundary errors are explicit `ValueError`s at
   the point of validation. No silent defaults, no `try/except: pass`, no
   `assert` for invariants that `-O` would erase.

## Style gates

`make lint` (ruff E4/E7/E9/F/W + mypy baseline with `check_untyped_defs`)
covers this tree alongside `src/`, `tests/`, and `scripts/`.
