# reference/ — pure-Python reference implementation (test-time oracle)

This tree is the specification the kernel is ported from, and the bit-exact
oracle the kernel is checked against — never the reverse. It is a **test asset,
not an engine**: the distributed facade has a single execution path (the native
extension `wwise_wem._core`) and never imports this package at runtime.

Those two facts, and everything else this tree must satisfy, are product norms:
[`../docs/reference/standards.md`](../docs/reference/standards.md)
([Bit-exactness](../docs/reference/standards.md#bit-exactness),
[Layers and dependency direction](../docs/reference/standards.md#layers-and-dependency-direction),
[Errors](../docs/reference/standards.md#errors)).

## Status

- **Not distributed.** It is not part of the wheel and not listed in
  `tests/parity/distribution_allowlist.json`. The development tree drives
  every test target with `PYTHONPATH=src:reference` (see Makefile).
- **Imported directly by tests and capture tooling only.** Parity suites,
  the frame/stage comparisons, and the container capture call
  `wwise_wem_reference.python_engine` directly and compare its bytes against
  the kernel and the reference fixtures.
- **Two-way naming.** Implementation domains here import facade DTOs and
  profile-metadata types (`wwise_wem.model`, `wwise_wem.profiles.*`) by
  absolute name; everything else resolves by in-package relative imports.

## Style checks

`ruff check src reference tests scripts` and
`mypy --no-site-packages src reference` (ruff E4/E7/E9/F/W + mypy baseline with
`check_untyped_defs`) cover this tree alongside `src/`, `tests/`, and
`scripts/`; both run inside `make test`.
