.PHONY: test test-fast frame-contract stage-contract golden lint build wheel-smoke check clean native

PY ?= python3

test: test-fast frame-contract stage-contract golden

test-fast:
	PYTHONDONTWRITEBYTECODE=1 PYTHONPATH=src:reference python3 -m unittest discover -s tests/unit -t . -v
	PYTHONDONTWRITEBYTECODE=1 PYTHONPATH=src:reference python3 -m unittest discover -s tests/integration -t . -v
	PYTHONDONTWRITEBYTECODE=1 PYTHONPATH=src:reference python3 -m unittest discover -s tests/contract -t . -v

frame-contract:
	PYTHONDONTWRITEBYTECODE=1 PYTHONPATH=src:reference python3 -m unittest tests.contract.frame_pipeline_contract -v

stage-contract:
	PYTHONDONTWRITEBYTECODE=1 PYTHONPATH=src:reference python3 -m unittest tests.contract.stage_pipeline_contract -v

golden:
	PYTHONDONTWRITEBYTECODE=1 PYTHONPATH=src:reference python3 -m unittest tests.golden.test_golden -v

RUFF ?= ruff
MYPY ?= mypy
lint:
	$(RUFF) check src reference tests scripts
	$(MYPY) src reference

# Development-tree native kernel: builds the in-package extension
# (src/wwise_wem/_core.abi3.so) into the active venv via maturin.
# Required for every byte-producing call — there is no other path.
native:
	maturin develop -F extension-module

build:
	python3 -m pip wheel . --no-deps -w dist

wheel-smoke:
	python3 scripts/wheel_smoke.py

rust-test:
	cd crates && cargo test --workspace

rust-bench:
	cd crates && cargo build --release -p wem-core && target/release/wwise-wem ../tests/fixtures/input.wav --output /dev/null --time

check: lint test wheel-smoke

clean:
	python3 scripts/clean.py
