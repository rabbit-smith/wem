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
  `docs/figures/concurrency-curves.png`, `measure_decode_concurrency.py` →
  `docs/figures/decode-concurrency-samples.json`,
  `plot_decode_concurrency_curves.py` → `docs/figures/decode-concurrency-curves.png`.
  Never write outside these trees as a side effect.
- The decode measurement writes only generated measurement inputs, and only to
  gitignored scratch: `measure_decode_perf.py` →
  `crates/target/decode-perf/inputs/*.wav` and
  `crates/target/decode-perf/wems/*.wem` (its longer streams, built from the
  tracked fixture by the release encoder). It prints its numbers, records no
  baseline, and reads nothing under `corpus/`.
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
- Scripts must pass the lint step of `make test` — `ruff check src reference
  tests scripts`; the mypy baseline (`mypy --no-site-packages src reference`)
  does not cover `scripts/`. Use `#!/usr/bin/env python3`, type hints, and
  `SystemExit` codes; failures print the offending artifact path, never a
  stack-trace shrug.
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
- `measure_encode_perf.py` **measures** the release build and judges nothing:
  the CLI's stage timers for the fixture and for every 2-channel corpus WAV,
  plus the kernel's per-stage split read from the `stage_timings` harness, each
  reported as min / median / p95 / max / spread with the machine identity and
  the machine's load average (before and after each series, because two
  readings taken at different loads cannot be compared). It has no threshold, no
  recorded baseline and no re-record escape hatch, and its exit status reports
  only whether the measurement ran. A number that is compared against a recorded
  one hides a change behind a re-record step
  ([`../docs/reference/standards.md`](../docs/reference/standards.md#bit-exactness));
  that is why a performance threshold does not belong in this tree. It selects
  the harness's reporting test **by name** (`--ignored --exact
  stage_timings_report`), because the other ignored test in that file belongs to
  the duration-curve driver and needs an environment this one does not set;
  `--no-stages` skips the harness when no Rust toolchain is available. Its
  evidence is written up in
  [`../docs/findings/encode-performance.md`](../docs/findings/encode-performance.md).
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
- Naming: verb-first (`record_`, `generate_`, `emit_`, `verify_`, `fuzz_`,
  `measure_`); one script, one artifact family; shared helpers belong in
  `tests/*_support.py` when the consumer is a test suite.

## Forbidden

Ad-hoc one-off scripts committed here; interactive prompts; anything that
mutates `tests/fixtures/` or `reference.wem`; writes into `src/wwise_wem/`
other than the documented profile-data trio update.
