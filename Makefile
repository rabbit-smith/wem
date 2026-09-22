# =============================================================================
# One verdict.
#
# `make test` is the whole verdict: every check this tree has, cheapest first,
# and the only entry a developer or CI needs to know. Green means the tree is
# good; the first failing line names what broke.
#
# Nothing else here is a second opinion. The other targets are either a build
# that produces what the checks read (`native`, `build`, `wasm-build`, `clean`),
# or a layer of `test` -- the same commands over a cheaper subset, worth running
# on its own: `test-fast` (the Python suites), and the two suites the tier table
# names by path, `wem-bytes` (`tests/whole_file/test_whole_file.py`) and
# `2ch-long` (`tests/parity/two_channel_long_run.py`). Anything else a developer
# might want to run alone is the underlying tool's own command, and
# docs/guides/development.md shows that command rather than a name invented for
# it here.
# =============================================================================

PY ?= python3
NODE ?= node

# -----------------------------------------------------------------------------
# The toolchain, in one place.
#
# On this machine `rustdoc` and `clippy-driver` resolve from Homebrew while
# `cargo` and `rustc` come from rustup, and the mix fails with a spurious E0514
# -- "found crate `wem_scheduling` compiled by an incompatible version of
# rustc", with both sides printing the same version number -- because the
# Homebrew rustdoc and clippy-driver read metadata written by a different build
# of that rustc. `cargo doc` and `cargo clippy` therefore needed the rustup
# toolchain's `bin` prepended by hand, which is not knowledge a developer should
# have to have. Exported here once, for every command this file runs.
#
# Safe to export:
#   * the directory is the `bin` of the toolchain `cargo` already dispatches to
#     through the rustup shim, so no compiler changes -- the same binaries,
#     reached by name instead of through the shim;
#   * `RUSTDOC` names that toolchain's rustdoc, which is what `cargo doc` would
#     resolve once the `bin` is on `PATH`; cargo reads it for doc builds only;
#   * a machine without that directory -- CI, a container whose rustup lives
#     elsewhere -- keeps its own `PATH` and its own `rustdoc`, because the whole
#     export sits inside the `ifneq`;
#   * it applies to the commands this makefile runs, never to the developer's
#     shell.
RUSTUP_TOOLCHAIN := $(firstword $(shell rustup show active-toolchain 2>/dev/null))
RUSTUP_BIN := $(wildcard $(HOME)/.rustup/toolchains/$(RUSTUP_TOOLCHAIN)/bin)
ifneq ($(RUSTUP_BIN),)
export PATH := $(RUSTUP_BIN):$(PATH)
export RUSTDOC := $(RUSTUP_BIN)/rustdoc
endif

# Lint and build tools come from the project venv when it exists, so `make test`
# works in a checkout whose venv is not activated (ruff, mypy and maturin are
# not on a bare PATH). Without the venv they resolve from PATH as before, which
# is how "the Python lint could not be run locally" happened. `maturin` is not
# part of an editable install -- pip fetches the build backend into an isolated
# environment and leaves the command behind nowhere -- so it falls back to a
# `maturin` on PATH when the venv has none.
VENV_BIN := $(if $(wildcard .venv/bin),.venv/bin/,)
RUFF ?= $(VENV_BIN)ruff
MYPY ?= $(VENV_BIN)mypy
MATURIN ?= $(firstword $(wildcard $(VENV_BIN)maturin) maturin)

.PHONY: test test-fast wem-bytes 2ch-long native build wasm-build clean

# -----------------------------------------------------------------------------
# The verdict. Cheap first: the static checks, then the suites, then the checks
# that build an artifact (the wheel, the wasm packages) before comparing bytes.
# Every line is a check -- each must pass, and the first failure stops the run.
#
# The measurement instruments are deliberately not in this list and have no
# target: they report numbers and never fail on one. Their commands are in
# docs/guides/development.md, under "Watching performance".
# -----------------------------------------------------------------------------
test: native
	$(RUFF) check src reference tests scripts
	$(MYPY) --no-site-packages src reference
	cd crates && cargo fmt --all --check
	cd crates && cargo clippy --workspace --all-targets -- -D warnings
	cd crates && RUSTDOCFLAGS="-D warnings" cargo doc --workspace --no-deps
	$(MAKE) test-fast
	PYTHONDONTWRITEBYTECODE=1 PYTHONPATH=src:reference $(PY) scripts/native_smoke.py
	$(MAKE) wem-bytes
	$(MAKE) 2ch-long
	cd crates && cargo test --workspace
	PYTHONDONTWRITEBYTECODE=1 PYTHONPATH=src:reference $(PY) scripts/fuzz_diff_parity.py --pr
	PYTHONPATH= $(PY) scripts/wheel_smoke.py
	$(MAKE) wasm-build
	$(NODE) js/test-node.mjs

# -----------------------------------------------------------------------------
# The layers `test` runs, and the only check-targets that stay: the two suites
# named by path in the tier table (tests/AGENTS.md) keep a target because the
# build names their path, and discovery does not walk either one.
# -----------------------------------------------------------------------------

# The Python suites -- unit, integration, cross-implementation -- in the order
# the ladder climbs them. This is the layer to run while iterating.
test-fast:
	PYTHONDONTWRITEBYTECODE=1 PYTHONPATH=src:reference $(PY) -m unittest discover -s tests/unit -t . -v
	PYTHONDONTWRITEBYTECODE=1 PYTHONPATH=src:reference $(PY) -m unittest discover -s tests/integration -t . -v
	PYTHONDONTWRITEBYTECODE=1 PYTHONPATH=src:reference $(PY) -m unittest discover -s tests/parity -t . -v

# Whole-file reference WEM parity: tests/whole_file/test_whole_file.py, the one
# module under tests/whole_file, which `unittest discover` over tests/ does not
# reach from `test-fast`'s three directories.
wem-bytes:
	PYTHONDONTWRITEBYTECODE=1 PYTHONPATH=src:reference $(PY) -m unittest tests.whole_file.test_whole_file -v

# The long-run cross-implementation case: tests/parity/two_channel_long_run.py,
# which keeps its non-`test_` prefix (and so is invisible to discovery) for the
# same reason.
2ch-long:
	PYTHONDONTWRITEBYTECODE=1 PYTHONPATH=src:reference $(PY) -m unittest tests.parity.two_channel_long_run -v

# -----------------------------------------------------------------------------
# Builds. These produce or remove artifacts; they judge nothing.
# -----------------------------------------------------------------------------

# Development-tree native kernel: builds the in-package extension
# (src/wwise_wem/_core.abi3.so) into the active venv via maturin. Required for
# every byte-producing call -- there is no other path -- which is why `test`
# depends on it.
native:
	$(MATURIN) develop -F extension-module

build:
	$(PY) -m pip wheel . --no-deps -w dist

# Web/Node shell packages: wasm-pack builds both targets -- js/pkg (web) and
# js/pkg-node (nodejs). They are wasm-pack output and are not committed, so
# build them before the Node parity check or the browser demo. The npm scripts
# carry the absolute --out-dir, because wasm-pack resolves a relative one
# against the crate directory rather than the current one.
wasm-build:
	cd js && npm run build

clean:
	$(PY) scripts/clean.py