.PHONY: test test-fast frame-contract stage-contract proto-contract proto-smoke golden lint build wheel-smoke check clean

PY ?= python3

test: test-fast frame-contract stage-contract golden proto-contract proto-smoke

test-fast:
	PYTHONDONTWRITEBYTECODE=1 PYTHONPATH=src python3 -m unittest discover -s tests/unit -t . -v
	PYTHONDONTWRITEBYTECODE=1 PYTHONPATH=src python3 -m unittest discover -s tests/integration -t . -v
	PYTHONDONTWRITEBYTECODE=1 PYTHONPATH=src python3 -m unittest discover -s tests/contract -t . -v

frame-contract:
	PYTHONDONTWRITEBYTECODE=1 PYTHONPATH=src python3 -m unittest tests.contract.frame_pipeline_contract -v

stage-contract:
	PYTHONDONTWRITEBYTECODE=1 PYTHONPATH=src python3 -m unittest tests.contract.stage_pipeline_contract -v

proto-contract:
	PYTHONDONTWRITEBYTECODE=1 PYTHONPATH=src $(PY) -m unittest tests.contract.test_proto_contract -v

proto-smoke:
	PYTHONDONTWRITEBYTECODE=1 PYTHONPATH=src $(PY) scripts/mock_grpc_smoke.py

golden:
	PYTHONDONTWRITEBYTECODE=1 PYTHONPATH=src python3 -m unittest tests.golden.test_golden -v

RUFF ?= ruff
MYPY ?= mypy
lint:
	$(RUFF) check src tests scripts
	$(MYPY) src

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
