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
- Mock gRPC tooling (`mock_grpc_server.py`, `mock_grpc_smoke.py`) pins proto
  semantics with the Python oracle: byte equality with `encode_wav` output is
  the pass condition; ports are ephemeral, generated stubs live only in temp
  directories, and error paths (unknown digest, lifecycle violation) must
  return enumerated errors, not exceptions.
- `wheel_smoke.py` verifies installed and zip-import resource paths; when
  packaging changes (new data, native artifacts), update it in the same commit
  — its expected sets are contracts, not constants.
- Naming: verb-first (`record_`, `generate_`, `emit_`, `verify_`); one script,
  one artifact family; shared helpers belong in `tests/*_support.py` when the
  consumer is a contract.

## Forbidden

Ad-hoc one-off scripts committed here; interactive prompts; anything that
mutates `tests/fixtures/` or `reference.wem`; writes into `src/wwise_wem/`
other than the documented profile-data trio update.
