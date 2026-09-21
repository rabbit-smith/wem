.PHONY: test test-fast fuzz-parity 2ch-stress 2ch-long frame-contract stage-contract golden lint rust-lint rust-test rust-bench build wheel-smoke check clean native

PY ?= python3

test: test-fast fuzz-parity 2ch-stress 2ch-long frame-contract stage-contract golden

test-fast:
	PYTHONDONTWRITEBYTECODE=1 PYTHONPATH=src:reference $(PY) -m unittest discover -s tests/unit -t . -v
	PYTHONDONTWRITEBYTECODE=1 PYTHONPATH=src:reference $(PY) -m unittest discover -s tests/integration -t . -v
	PYTHONDONTWRITEBYTECODE=1 PYTHONPATH=src:reference $(PY) -m unittest discover -s tests/contract -t . -v

fuzz-parity:
	PYTHONDONTWRITEBYTECODE=1 PYTHONPATH=src:reference $(PY) scripts/fuzz_diff_parity.py --pr

2ch-stress:
	PYTHONDONTWRITEBYTECODE=1 PYTHONPATH=src:reference $(PY) -m unittest tests.contract.test_2ch_stress_corpus -v

2ch-long:
	PYTHONDONTWRITEBYTECODE=1 PYTHONPATH=src:reference $(PY) -m unittest tests.contract.two_channel_long_run_contract -v

frame-contract:
	PYTHONDONTWRITEBYTECODE=1 PYTHONPATH=src:reference $(PY) -m unittest tests.contract.frame_pipeline_contract -v

stage-contract:
	PYTHONDONTWRITEBYTECODE=1 PYTHONPATH=src:reference $(PY) -m unittest tests.contract.stage_pipeline_contract -v

golden:
	PYTHONDONTWRITEBYTECODE=1 PYTHONPATH=src:reference $(PY) -m unittest tests.golden.test_golden -v

RUFF ?= ruff
MYPY ?= mypy
lint:
	$(RUFF) check src reference tests scripts
	$(MYPY) --no-site-packages src reference

rust-lint:
	cd crates && cargo clippy --workspace --all-targets -- -D warnings

# Development-tree native kernel: builds the in-package extension
# (src/wwise_wem/_core.abi3.so) into the active venv via maturin.
# Required for every byte-producing call — there is no other path.
native:
	maturin develop -F extension-module

build:
	$(PY) -m pip wheel . --no-deps -w dist

wheel-smoke:
	$(PY) scripts/wheel_smoke.py

rust-test:
	cd crates && cargo test --workspace

rust-bench:
	cd crates && cargo build --release -p wem-core && target/release/wwise-wem ../tests/fixtures/input.wav --output /dev/null --time

check: lint rust-lint test wheel-smoke

clean:
	$(PY) scripts/clean.py
