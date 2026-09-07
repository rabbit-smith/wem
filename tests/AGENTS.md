# tests/ — contract layers and versioned assets

Tests are the executable definition of "bit-exact". They are more authoritative
than any description; when code and contract disagree, the contract wins.

## Suite map (run order = cheap → expensive)

| Layer | Command | Meaning |
|---|---|---|
| unit | `make test-fast` (discovery) | module behavior, validation, loaders |
| integration | in `make test-fast` | CLI/API/adapter end paths |
| contract | in `make test-fast` (test_* files) + `make frame-contract`, `make stage-contract` | pipeline invariants |
| golden | `make golden` | whole-file byte identity (the digest) |
| proto | `make proto-contract proto-smoke PY=…` | IDL structure + streaming byte identity |
| wheel | `make wheel-smoke` | single-wheel (facade + native extension) inventory, clean-venv byte-exact encode |

Modules without `test_` prefix (e.g. `frame_pipeline_contract.py`,
`stage_pipeline_contract.py`) are intentionally not discovered; they run only
via their Makefile targets and the CI steps that mirror them. Keep it that way:
new heavy per-frame suites get an explicit target, and if a workflow should
enforce it, add the step (matching existing explicit runs).

## Environment

All test targets run with `PYTHONPATH=src:reference` (the Makefile sets it):
`src` provides the distribution facade, `reference/` provides the pure-Python
reference implementation that the oracle pipeline, dual-engine parity, and
capture tests import. Native-kernel tests skip (with an install hint) when
`wwise_wem._native` is not importable; every other test must pass without it.

## Versioned asset rules (`tests/data/`, `tests/fixtures/`)

- `fixtures/input.wav` + `fixtures/reference.wem`: sacred inputs/outputs, never
  regenerate or touch `reference.wem`.
- `data/frame-contract/`: per-frame hashes of the accepted pipeline. Any change
  requires the golden digest conversation first.
- `data/stage-golden/stages/index.json`: SHA-256 per frame × stage for all 205
  frames; `frames/*.bin`: raw little-endian dumps for the 28 representative
  frames (edges + every short↔long transition and its predecessor). These are
  the cross-implementation (Rust) diffing baseline and must outlive the Python
  oracle — treat as source of truth, not cache.
- `data/stage-golden/transcendental/`: per-site (input bits, output bits)
  records of the live domain; regenerate via `scripts/record_tmath.py`.
- `data/perf-baseline.json`: the release-build fixture-encode performance
  baseline (median encode-stage ms, cap 150ms, regression factor 1.25)
  consumed by `scripts/perf_gate.py`; the values are a versioned contract —
  update only with an explicit decision (`python3 scripts/perf_gate.py
  --record`), never as incident response to a red gate.
- Asset budgets: stage-golden total ≤ 20 MB. Growth requires trimming the
  representative-frame rule deliberately (document the rule change here), not
  by silently adding full-stream dumps.

## Writing contract tests

- Failures must localize: report first differing frame index + stage name +
  digest prefix (follow `stage_pipeline_contract.py`'s differ pattern).
- Deterministic regeneration is part of the contract: run the emit script twice,
  diff must be empty; keep JSON `sort_keys=True`; encode endianness in file
  suffixes (`.f32le`, `.u16le`, `.u8`, `.u64le`).
- Byte-level dumps are always LE numeric or raw bytes; never text floats, never
  repr, never platform-dependent formatting.
- Deviations (deliberate skips when optional dev tooling is absent) must print
  an install hint, and CI must install the extras so the suite is truly enforced
  there, not silently skipped.
- Test names describe domain behavior (`test_windows_reproduce_reference_synthesis`),
  not implementation churn.

## Modifying tests

The cleanliness and distribution contracts also police `tests/contract/*.json`
allowlists; when you add package files or change profile resource counts, the
same commit must update: `distribution_allowlist.json`, `wheel_smoke.py`'s
expected resource set, and any hardcoded counts (e.g. manifest resource count
in `test_profile_bundle.py`).
