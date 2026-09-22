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
# it here. Two targets below are exceptions and neither is a check:
# `benchmark`, the measurement entry -- `test` never runs it and it never fails
# on a number -- and `hooks`, which arms this clone's pre-commit layer. Neither
# is a second opinion on the tree.
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

# -----------------------------------------------------------------------------
# Neutral build paths.
#
# The compiler writes the absolute path of every file it reads into the binary
# it produces -- registry sources, toolchain files -- so an artifact built in a
# checkout from a developer's account names that account's home directory. That
# is not reproducible, and it is not the artifact's business: the prefixes are
# rewritten to fixed roots before the compiler records them.
#
# The rewrite expands the building account's `HOME` and `CARGO_HOME`, which are
# per machine, so it is built here and exported into the environment of every
# command this file runs: maturin (`native`, `build`), wasm-pack
# (`wasm-build`), cargo (`benchmark`). One export rather than one env prefix
# per recipe is what keeps those three build paths from drifting apart. A
# committed `.cargo/config.toml` cannot carry it -- its strings do not expand a
# variable, and the prefix differs on every machine -- so this tree has none.
#
# `CARGO_ENCODED_RUSTFLAGS`, not `RUSTFLAGS`: its values are `0x1f`-separated
# rather than space-separated, so a home directory containing a space still
# reaches rustc as one argument, and cargo ignores `RUSTFLAGS` outright once
# the encoded form is set -- one mechanism, so a tool that sets a `RUSTFLAGS`
# of its own for the builds that ask for one (wasm-pack has that path) cannot
# displace the rewrite. Order matters as well: of the prefixes matching a path
# the last one wins, `CARGO_HOME` normally sits inside `HOME` and matches both,
# so it is written second and the registry lands under `/cargo`, not
# `/build/.cargo`.
#
# The value replaces one the developer exported by hand, deliberately: a second
# mechanism is the failure mode this file is avoiding. `make <target>
# CARGO_ENCODED_RUSTFLAGS=...` still wins, because a command-line assignment
# beats an assignment made here.
# -----------------------------------------------------------------------------
CARGO_HOME_DIR := $(if $(CARGO_HOME),$(CARGO_HOME),$(HOME)/.cargo)
FLAG_SEPARATOR := $(shell printf '\037')
export CARGO_ENCODED_RUSTFLAGS := --remap-path-prefix=$(HOME)=/build$(FLAG_SEPARATOR)--remap-path-prefix=$(CARGO_HOME_DIR)=/cargo

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

.PHONY: test test-fast wem-bytes 2ch-long native build wasm-build clean hooks benchmark

# -----------------------------------------------------------------------------
# The verdict. Cheap first: the static checks, then the suites, then the checks
# that build an artifact (the wheel, the wasm packages) before comparing bytes.
# Every line is a check -- each must pass, and the first failure stops the run.
#
# The Rust suites run in both configurations, because the kernel ships one and
# is verified in the other. A default build is scalar -- the `parallel` feature
# is opt-in -- and the second leg of each pair is the configuration our CLI
# requires, which a single default run would neither build nor test. The second
# clippy leg is there for the same reason from the other side: `--workspace
# --all-targets` does not lint the bin that feature gates.
#
# The measurement instruments are deliberately not in this list: they report
# numbers and never fail on one. They have a single entry, `benchmark`, below.
# -----------------------------------------------------------------------------
test: native
	$(PY) scripts/check_documentation.py
	PYTHONPATH= $(PY) -m pytest --collect-only -q
	$(RUFF) check src reference tests scripts
	$(MYPY) --no-site-packages src reference
	cd crates && cargo fmt --all --check
	cd crates && cargo clippy --workspace --all-targets -- -D warnings
	cd crates && cargo clippy -p wem-analysis -p wem-core --all-targets --features parallel -- -D warnings
	cd crates && RUSTDOCFLAGS="-D warnings" cargo doc --workspace --no-deps
	$(MAKE) test-fast
	PYTHONDONTWRITEBYTECODE=1 PYTHONPATH=src:reference $(PY) scripts/native_smoke.py
	$(MAKE) wem-bytes
	$(MAKE) 2ch-long
	cd crates && cargo test --workspace --all-targets
	cd crates && cargo test -p wem-analysis -p wem-core --all-targets --features parallel
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

# -----------------------------------------------------------------------------
# The local pre-commit layer: arm the versioned hooks in this clone.
#
# `core.hooksPath` is per clone -- it lives in .git/config, so cloning, a new
# worktree and CI do not carry it. This is the one command that sets it, and it
# prints the setting back so the run says what it did. The hook is early
# feedback, not the verdict: `make test` and CI still decide
# (docs/guides/hooks.md).
# -----------------------------------------------------------------------------
hooks:
	git config core.hooksPath .githooks
	@printf 'core.hooksPath = %s\n' "$$(git config core.hooksPath)"

# -----------------------------------------------------------------------------
# The one measurement entry, and not a check.
#
# It reports numbers over several runs, takes minutes, and its exit status says
# only whether the run completed. No threshold and no recorded baseline live
# here, because a number compared against a recorded one hides a change behind a
# re-record step (docs/reference/standards.md). `test` never runs it.
#
# This is a name invented for a tool invocation, which the header above says
# this file does not do. It earns the exception because the two scripts are read
# together -- encode and decode, one machine, one load -- and because their
# paths, flags and the release build they need are not worth remembering.
# -----------------------------------------------------------------------------
benchmark:
	cd crates && cargo build --release -p wem-core --features parallel
	$(PY) scripts/measure_encode_perf.py
	$(PY) scripts/measure_decode_perf.py
