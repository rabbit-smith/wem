# Changelog

## Unreleased

- Breaking Rust interface change: analysis, profile and packet errors expose
  domain categories instead of a public variant per validation site. Match the
  category and read the diagnostic for operation-specific values; nested causes
  remain available through `Error::source()`.
- Refuse bit reads wider than 64 bits before advancing the reader, preventing a
  malformed setup packet from causing a shift panic.
- Prevent RIFF extent and chunk-step overflow on 32-bit targets.
- Refuse incomplete PCM frames in WAV input instead of dropping trailing data.
- Correct the research decoder's generic inverse transform and hybrid windows;
  tighten the live synthesis comparison to rounding-scale numerical tolerances.
- Reject partial PCM frames from a kernel decode step as an internal failure
  before language shells deliver that step's output.
- Add input fuzzing, live Rust/NumPy synthesis comparison, documentation path
  checks and pytest discovery without a manual import path.
- Add an explicit, streaming comparison against a separately installed external
  decoder, usable on both committed fixtures and local long streams.
- Limit byte-exactness claims to the committed paired-build vectors and identify
  the independence limits of comparisons that share profile values.
