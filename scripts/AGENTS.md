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
- Real-server interop smoke (`interop_grpc_smoke.py`) pins the wwise.v1
  contract end-to-end: a Python gRPC client streams
  `tests/fixtures/input.wav` through the real Rust `wem-server` on an
  ephemeral port (READY-line handshake, no fixed delays) and must rebuild
  the container byte-identically to `tests/fixtures/reference.wem`, with
  the unknown-digest path returning the enumerated PROFILE_NOT_FOUND error;
  v1 semantics are otherwise pinned by the wem-server integration tests.
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
