"""Pure-Python reference implementation of the WAV-to-WEM encoder.

This development-tree package is the bit-exact oracle: every module here is
implementation code that the distributed facade (``wwise_wem``) used to carry
inline, and its output bytes are the project's source of truth.  The facade's
native-first engine switch (``wwise_wem._engine``) delegates to this package
only when the resolved engine is the pure-Python implementation; the native
kernel never touches it.

The package is not part of the distribution allowlist and is not shipped in
the wheel: installed copies of the facade must run on the native kernel, while
the development tree (``PYTHONPATH=src:reference``) keeps the oracle importable
for the frame/stage contracts, the golden tests, and the dual-engine parity
suite.
"""
