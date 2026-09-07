# scripts/ — deterministic tooling

Scripts here are contract infrastructure: their outputs feed versioned assets
or CI gates. They must be boring, idempotent, and offline.

## Rules

- **Byte-stable outputs by construction**: deterministic ordering
  (`sort_keys`, sorted file walks), explicit little-endian packing, and no
  timestamps/paths/absolute locations embedded in artifacts.
  Every emit/record script must be safe to run twice with an empty diff.
- **No network, no ambient state**: operate on repo paths resolved from
  `Path(__file__)`; env overrides must be declared in `--help`.
- **Write only to declared destinations**: e.g. `record_tmath.py` →
  `tests/data/stage-golden/transcendental/`, `generate_frozen_tables.py` →
  the payload + manifest + index trio (and it refuses when the index already
  matches), `emit_stage_golden.py` → `tests/data/stage-golden/stages/`.
  Never write outside these trees as a side effect.
- **Cross-check before emit**: generators assert their output against existing
  contract assets (frozen table generation proves domain agreement with the
  recorded site pairs; the golden digest is verified after, not assumed).
- Scripts must pass `make lint` (ruff covers `scripts/`), use `#!/usr/bin/env
  python3`, type hints, and `SystemExit` codes; failures print the offending
  artifact path, never a stack-trace shrug.
- `wheel_smoke.py` verifies the single-wheel distribution end-to-end:
  the facade + native extension inventory against the distribution
  allowlist, the zip-import and installed metadata paths, and a clean-venv
  byte-exact encode through the embedded kernel (builds the wheel, so it
  needs a Rust toolchain); when packaging changes (new data, native
  artifacts), update it in the same commit — its expected sets are
  contracts, not constants.
- `fuzz_diff_parity.py` is the deterministic oracle-vs-native differential
  gate: a fixed seed set of random PCM streams (random lengths, random
  frame-aligned chunk splits) must encode byte-identically through the
  pure-Python oracle and the native streaming kernel; `--pr` is the small
  PR-budget run, `--full` the nightly-sized run. Seeds are part of the
  contract; never randomize them at run time.
- `perf_gate.py` is the fixture performance gate on the release build:
  the encode-stage median must stay under the 150ms cap and within 25% of
  the recorded baseline (`tests/data/perf-baseline.json`); re-recording the
  baseline is a deliberate decision, not an incident response.
- Naming: verb-first (`record_`, `generate_`, `emit_`, `verify_`); one script,
  one artifact family; shared helpers belong in `tests/*_support.py` when the
  consumer is a contract.

## Forbidden

Ad-hoc one-off scripts committed here; interactive prompts; anything that
mutates `tests/fixtures/` or `reference.wem`; writes into `src/wwise_wem/`
other than the documented profile-data trio update.
