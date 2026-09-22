# tests/ — suites and the assets they read

The suites are the executable form of the bit-exactness claim, and they are what
establishes each claim listed in
[`../docs/reference/standards.md`](../docs/reference/standards.md#what-is-established);
how a failing comparison is read is in the same document, under
[Bit-exactness](../docs/reference/standards.md#bit-exactness).

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
- There is **no performance baseline asset**. Encoding speed is measured by
  `scripts/measure_encode_perf.py` on the release build and reported with its
  full distribution; a recorded median compared against a run is a recorded
  expectation whose failure mode is re-recording
  ([`../docs/reference/standards.md`](../docs/reference/standards.md#bit-exactness)),
  so the suite holds no timing threshold and no memory ceiling.
- Asset budgets: stage-records total ≤ 20 MB. Growth requires trimming the
  representative-frame rule deliberately (document the rule change here), not
  by silently adding full-stream dumps.

## Writing pipeline tests

- A cross-implementation test runs both implementations and compares values:
  per frame, per stage, per channel, per bin.
- Failures must localize: name the first differing frame index, stage, channel,
  and bin, and print both words.
- Byte-level dumps encode endianness in the file suffix (`.f32le`, `.u16le`,
  `.u8`, `.u64le`); the values they carry follow
  [`../docs/reference/standards.md`](../docs/reference/standards.md#bit-patterns).
- Deviations (deliberate skips when optional dev tooling is absent) must print
  an install hint, and CI must install the extras so the suite is truly enforced
  there, not silently skipped.
- Test names describe domain behavior (`test_windows_reproduce_reference_synthesis`),
  not implementation churn.

## Modifying tests

The cleanliness and distribution suites also read the `tests/parity/*.json`
allowlists; when you add package files or change the profile inventory, the same
commit must update `distribution_allowlist.json` and anything that reads it
(`scripts/wheel_smoke.py`). Profile values are compiled into the kernel, so the
allowlist's `resources` list stays empty and no packaged data exists to count.
