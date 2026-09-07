.PHONY: test test-fast frame-contract stage-contract golden lint build wheel-smoke check clean

test: test-fast frame-contract stage-contract golden

test-fast:
	PYTHONDONTWRITEBYTECODE=1 PYTHONPATH=src python3 -m unittest discover -s tests/unit -t . -v
	PYTHONDONTWRITEBYTECODE=1 PYTHONPATH=src python3 -m unittest discover -s tests/integration -t . -v
	PYTHONDONTWRITEBYTECODE=1 PYTHONPATH=src python3 -m unittest discover -s tests/contract -t . -v

frame-contract:
	PYTHONDONTWRITEBYTECODE=1 PYTHONPATH=src python3 -m unittest tests.contract.frame_pipeline_contract -v

stage-contract:
	PYTHONDONTWRITEBYTECODE=1 PYTHONPATH=src python3 -m unittest tests.contract.stage_pipeline_contract -v

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

check: lint test wheel-smoke

clean:
	python3 scripts/clean.py
