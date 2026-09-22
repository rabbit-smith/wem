"""Pure-Python reference implementation of the WAV-to-WEM encoder.

This development-tree package is the bit-exact oracle: every module here is
implementation code that the distributed facade (``wwise_wem``) used to carry
inline, and its output bytes are the project's specification.

It is a **test-time asset, not a runtime engine.** The distributed facade
has a single execution path — the native kernel extension
(``wwise_wem._core``) — and never imports this package.  Only test suites
and capture tooling import it directly (``wwise_wem_reference``), comparing
its bytes against the kernel and against the reference fixtures; parity is a
pinned expectation, checked by the tests, not by a runtime switch.

The package is not part of the distribution allowlist and is not shipped in
the wheel: installed copies of the facade run on the native kernel, while
the development tree (``PYTHONPATH=src:reference``) keeps the oracle
importable for the frame/stage comparisons, the reference tests, and the
parity suites.
"""
