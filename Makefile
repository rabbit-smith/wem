.PHONY: test test-fast fuzz-parity 2ch-stress 2ch-long wem-bytes lint rust-fmt rust-lint rust-doc rust-test rust-bench benchmark build wheel-smoke wasm-build wasm-test check clean native

PY ?= python3
NODE ?= node

test: test-fast fuzz-parity 2ch-stress 2ch-long wem-bytes

test-fast:
	PYTHONDONTWRITEBYTECODE=1 PYTHONPATH=src:reference $(PY) -m unittest discover -s tests/unit -t . -v
	PYTHONDONTWRITEBYTECODE=1 PYTHONPATH=src:reference $(PY) -m unittest discover -s tests/integration -t . -v
	PYTHONDONTWRITEBYTECODE=1 PYTHONPATH=src:reference $(PY) -m unittest discover -s tests/parity -t . -v

fuzz-parity:
	PYTHONDONTWRITEBYTECODE=1 PYTHONPATH=src:reference $(PY) scripts/fuzz_diff_parity.py --pr

2ch-stress:
	PYTHONDONTWRITEBYTECODE=1 PYTHONPATH=src:reference $(PY) -m unittest tests.parity.test_2ch_corpus.TwoChannelStressCorpusTests -v

2ch-long:
	PYTHONDONTWRITEBYTECODE=1 PYTHONPATH=src:reference $(PY) -m unittest tests.parity.two_channel_long_run -v

wem-bytes:
	PYTHONDONTWRITEBYTECODE=1 PYTHONPATH=src:reference $(PY) -m unittest tests.whole_file.test_whole_file -v

# Lint tools come from the project venv when it exists. Without this, `make
# lint` only works for a developer whose shell happens to have ruff/mypy on
# PATH, which is how "the Python lint could not be run locally" happened.
VENV_BIN := $(if $(wildcard .venv/bin),.venv/bin/,)
RUFF ?= $(VENV_BIN)ruff
MYPY ?= $(VENV_BIN)mypy
lint:
	$(RUFF) check src reference tests scripts
	$(MYPY) --no-site-packages src reference

rust-fmt:
	cd crates && cargo fmt --all --check

rust-lint:
	cd crates && cargo clippy --workspace --all-targets -- -D warnings
	cd crates && cargo clippy -p wem-analysis -p wem-core --all-targets --features parallel -- -D warnings

# Rustdoc resolves every intra-doc link and reports the ones it cannot follow.
# clippy does not read doc comments, so a public item whose documentation links
# to something private reaches main under a green `rust-lint`; this target is
# the check that catches it. Deterministic, so unlike a timing it is a fair gate.
rust-doc:
	cd crates && RUSTDOCFLAGS="-D warnings" cargo doc --workspace --no-deps

# Development-tree native kernel: builds the in-package extension
# (src/wwise_wem/_core.abi3.so) into the active venv via maturin.
# Required for every byte-producing call — there is no other path.
native:
	maturin develop -F extension-module

build:
	$(PY) -m pip wheel . --no-deps -w dist

wheel-smoke:
	$(PY) scripts/wheel_smoke.py

# Both configurations, because the kernel ships one and is verified in the
# other: a default build is scalar (the library imposes no threads — the
# `parallel` feature is opt-in), and the second leg is the opt-in configuration
# our CLI requires. Testing the default alone would hide the parallel path the
# flip makes non-default.
rust-test:
	cd crates && cargo test --workspace
	cd crates && cargo test -p wem-analysis -p wem-core --features parallel

# The CLI is built only with the feature it requires: `wwise-wem` encodes one
# file at a time, where the internal parallelism pays, and the nightly encode
# measurement reads the binary this target builds.
rust-bench:
	cd crates && cargo build --release -p wem-core --features parallel && target/release/wwise-wem ../tests/fixtures/input.wav --output /dev/null --time

# The measurements, and deliberately not a gate. `scripts/measure_*_perf.py`
# report numbers over several runs with the machine identity and the load
# average; they take minutes, and their exit status says only whether the run
# completed. No threshold and no recorded baseline live here, because a number
# compared against a recorded one hides a change behind a re-record step
# (docs/reference/standards.md). This target is therefore not part of
# `make check`; `rust-bench` above is the one-shot encode smoke against it.
benchmark:
	cd crates && cargo build --release -p wem-core
	$(PY) scripts/measure_encode_perf.py
	$(PY) scripts/measure_decode_perf.py

# Web/Node shell packages: wasm-pack builds both targets — js/pkg (web) and
# js/pkg-node (nodejs). They are wasm-pack output and are not committed, so
# build them before the Node test or the browser demo. The npm scripts carry
# the absolute --out-dir, because wasm-pack resolves a relative one against the
# crate directory rather than the current one.
wasm-build:
	cd js && npm run build

# The shell's byte comparison: build both packages, then run the Node test that
# checks the wasm encoder's bytes against tests/fixtures/reference.wem, one-shot
# and across chunkings. Without a built package the test fails with the build
# command; it never skips.
wasm-test: wasm-build
	$(NODE) js/test-node.mjs

check: lint rust-fmt rust-lint rust-doc test wheel-smoke

clean:
	$(PY) scripts/clean.py
