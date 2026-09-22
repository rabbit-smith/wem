# tests/ — suites and the assets they read

Tests are the executable definition of "bit-exact". When code and a test
disagree, the difference is the report: fix one of them, do not re-record
the other.

## Suite map (run order = cheap → expensive)

| Layer | Command | Meaning |
|---|---|---|
| unit | `make test-fast` (discovery) | module behavior, validation, loaders |
| integration | in `make test-fast` | CLI/API/adapter end paths |
| cross-implementation | in `make test-fast` (discovery) | pipeline invariants, cross-implementation parity |
| whole-file | `make wem-bytes` | whole-file byte identity (the digest) |
| capi | `cargo test -p wem-capi` | C ABI surface: reference byte identity via the FFI, error-code mapping, lifecycle violations |
| wheel | `make wheel-smoke` | single-wheel (facade + native extension) inventory, clean-venv byte-exact encode |

`tests/parity/` holds the cross-implementation suites. Each module sets two
implementations side by side — the Python oracle against the native kernel, the
C ABI against the kernel, the built wheel against its allowlist — and compares
their values or their bytes; several read a `*.json` allowlist from this
directory as an input.

`unittest discover` runs every `test_*.py` module under `tests/unit`,
`tests/integration`, and `tests/parity`; `cargo test --workspace --all-targets`
runs the crate suites. Helper modules without the `test_` prefix (e.g.
`oracle_frame_values.py`, `stage_records_support.py`, `wem_byte_compare.py`) are
imported by those modules and are never run on their own. `test_stage_pipeline.py`
is discovered through its `test_` prefix like every other comparison module.

## Environment

All test targets run with `PYTHONPATH=src:reference` (the Makefile sets it):
`src` provides the distribution facade, `reference/` provides the pure-Python
reference implementation that the per-frame parity suites, the oracle pipeline
and the adapter tests import directly (a test asset, not a runtime path).

The native kernel (`wwise_wem._core`) is a **hard requirement** of every
byte-producing test: build it with `make native` (or `pip install -e .`)
before running the suite, and no test designs skip cases for
native-absent environments — a missing kernel fails the suite, on purpose.

## Versioned assets (`tests/data/`, `tests/fixtures/`)

- `fixtures/input.wav` is the input the encoding suites read; a suite that
  needs different PCM builds it in process. `fixtures/reference.wem` is the
  expected whole-file output of that input, compared byte for byte by
  `tests/whole_file`.
- `data/stage-records/stages/index.json`: SHA-256 per frame × stage for all 205
  frames; `frames/*.bin`: raw little-endian dumps for the 28 representative
  frames (edges + every short↔long transition and its predecessor). The stage
  parity suites in `wem-analysis`/`wem-core` read them; the live per-frame
  comparison (`crates/wem-core/tests/frame_pipeline_parity.rs` and
  `tests/parity/test_frame_pipeline_parity.py`) compares the oracle against
  the kernel directly and reads none of them.
- `data/stage-records/transcendental/`: per-site (input bits, output bits)
  records of the live domain; regenerate via `scripts/record_tmath.py`.
- `data/perf-baseline.json`: the release-build fixture-encode performance
  baseline (median encode-stage ms, cap 150ms, regression factor 1.25)
  consumed by `scripts/perf_check.py`; update it only with an explicit decision
  (`python3 scripts/perf_check.py --record`), never as incident response to a
  failing run.
- Asset budgets: stage-records total ≤ 20 MB. Growth requires trimming the
  representative-frame rule deliberately (document the rule change here), not
  by silently adding full-stream dumps.

## Writing pipeline tests

- A cross-implementation test runs both implementations and compares values:
  per frame, per stage, per channel, per bin. Recorded expectations are not a
  comparison — when they disagree with a run, they hide the change behind a
  re-record step instead of reporting it. Values travel as 8-hex-digit words
  (see `tests/parity/oracle_frame_values.py`), never as decimal reprs.
- Failures must localize: name the first differing frame index, stage, channel,
  and bin, and print both words.
- Byte-level dumps are always LE numeric or raw bytes; never text floats, never
  repr, never platform-dependent formatting. Encode endianness in file suffixes
  (`.f32le`, `.u16le`, `.u8`, `.u64le`).
- Deviations (deliberate skips when optional dev tooling is absent) must print
  an install hint, and CI must install the extras so the suite is truly enforced
  there, not silently skipped.
- Test names describe domain behavior (`test_windows_reproduce_reference_synthesis`),
  not implementation churn.

## Modifying tests

The cleanliness and distribution suites also read the `tests/parity/*.json`
allowlists; when you add package files or change profile resource counts, the
same commit must update: `distribution_allowlist.json`, `wheel_smoke.py`'s
expected resource set, and any hardcoded counts (e.g. manifest resource count
in `test_profile_bundle.py`).
