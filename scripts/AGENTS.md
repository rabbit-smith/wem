# scripts/ — deterministic tooling

Scripts here produce the versioned assets and run the CI checks: their outputs
are read by the test suites and by the packaging steps. They must be boring,
idempotent, and offline. What their outputs must satisfy is in
[`../docs/reference/standards.md`](../docs/reference/standards.md#determinism)
(byte-stable regeneration) and
[`../docs/reference/standards.md`](../docs/reference/standards.md#bit-patterns)
(little-endian packing).

## Rules

- **Byte-stable outputs by construction**: every generator script here must be
  safe to run twice with an empty diff.
- **No network, no ambient state**: operate on repo paths resolved from
  `Path(__file__)`; env overrides must be declared in `--help`.
- **Write only to declared destinations**: e.g. `generate_frozen_tables.py` →
  `corpus/profiles/*/analysis/frozen-tables.json` (the untracked recorded
  material), `generate_profile_code.py` →
  `crates/wem-profiles/src/generated/`, `generate_2ch_{reference,stress}_inputs.py`
  → `tests/data/2ch-{reference,stress}/*.wav`, `measure_concurrency.py` →
  `docs/figures/concurrency-samples.json` and, for a comparison run, a second
  `docs/figures/concurrency-samples-*.json`, `plot_concurrency_curves.py` →
  `docs/figures/concurrency-curves.png`. Never write outside these trees
  as a side effect.
- The duration-curve pair writes to gitignored measurement scratch and nowhere
  else: `generate_duration_curve_inputs.py` → `crates/target/curves/inputs/*.wav`,
  and `measure_duration_curves.py` → `crates/target/curves/*.json` (its raw
  samples). The driver also builds and runs the ignored harnesses in
  `crates/wem-core/tests/` and spawns one child process per measurement point;
  it reads no committed asset and records no baseline, so a loaded machine
  changes the numbers and never the exit status. Its evidence is written up in
  [`../docs/findings/duration-curves.md`](../docs/findings/duration-curves.md).
- **Cross-check before emit**: generators assert their output against what is
  already committed where a fixture is involved (the 2ch corpus generators
  compare their rendered input against the committed WAV byte for byte before
  writing, and with `--check` also report a missing file); the reference
  container is compared byte for byte after, not assumed. Nothing writes a
  value for a later run to compare against — that comparison belongs in a test.
- Scripts must pass `make lint` (ruff covers `scripts/`), use `#!/usr/bin/env
  python3`, type hints, and `SystemExit` codes; failures print the offending
  artifact path, never a stack-trace shrug.
- `wheel_smoke.py` verifies the single-wheel distribution end-to-end:
  the facade + native extension inventory against the distribution
  allowlist, the zip-import and installed metadata paths, and a clean-venv
  byte-exact encode through the embedded kernel (builds the wheel, so it
  needs a Rust toolchain); when packaging changes (new data, native
  artifacts), update it in the same commit — its expected sets are shared with
  the test suites, not local constants.
- `fuzz_diff_parity.py` is the deterministic oracle-vs-native differential
  test: a fixed seed set of random PCM streams (random lengths, random
  frame-aligned chunk splits) must encode byte-identically through the
  pure-Python oracle and the native streaming kernel; `--pr` is the small
  PR-budget run, `--full` the nightly-sized run. The seeds are fixed; never
  randomize them at run time.
- `perf_check.py` checks fixture performance on the release build:
  the encode-stage median must stay under the 150ms cap and within 25% of
  the recorded baseline (`tests/data/perf-baseline.json`); re-recording the
  baseline is a deliberate decision, not an incident response.
- `measure_concurrency.py` records how N concurrent encodes behave, over
  `N x {parallel on, off} x {installed geometry}`, by starting child processes
  together and reading each one's own `wait4` rusage. It has no cap and no
  baseline — a loaded machine changes the numbers, never the exit status — and
  the load average is recorded with every cell because that is part of the
  result. `plot_concurrency_curves.py` renders the committed figure from that
  JSON and must stay byte-stable: it is the one script here whose output is a
  committed image, so it pins figure size and DPI, sorts every series and
  writes with `metadata={}`. Matplotlib is a documentation tool for that one
  script and is deliberately absent from `pyproject.toml` and from every crate;
  run it from a venv of your own, as its docstring says.
- Naming: verb-first (`generate_`, `verify_`, `fuzz_`); one script,
  one artifact family; shared helpers belong in `tests/*_support.py` when the
  consumer is a test suite.

## Forbidden

Ad-hoc one-off scripts committed here; interactive prompts; anything that
mutates `tests/fixtures/` or `reference.wem`; writes into `src/wwise_wem/`
other than the documented profile-data trio update.
