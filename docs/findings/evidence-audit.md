# Verification evidence audit

Date: 2026-09-23. This audit distinguishes port consistency from independent
paired-build evidence.

## Input failures found and repaired

- RIFF extent and chunk advancement could overflow `usize` on 32-bit targets.
  Bounds now preserve the parser's permissive truncated-payload behavior without
  wrapping. Maximum `u32` size regressions execute in the container crate's unit
  suite, including a wasm32 WASI runtime in CI.
- Coverage-guided WEM mutation found a setup packet with more than 64 trailing
  bits. Reading its padding into a `u64` shifted by 64 or more and panicked.
  `BitReader::read` now refuses that width before advancing, and a synthetic
  setup regression pins the typed refusal.
- A subsequent WAV mutation reached a successful parse with an incomplete
  multichannel PCM frame. The parser previously trimmed only odd sample bytes,
  and conversion silently dropped a partial frame. It now refuses any data
  length not divisible by the PCM frame size, matching the Python adapter;
  synthetic mono odd-byte and stereo half-frame regressions cover the refusal.
- All language shells now reject a kernel step containing a partial PCM frame
  as `INTERNAL`, with no output delivery and a terminal handle. Python previously
  discarded the remainder; its slice did not itself run out of bounds.

The fuzz targets exercise WEM parsing/streaming and WAV parsing under
AddressSanitizer and overflow checks. PR and nightly jobs preserve crash inputs.
A bounded successful fuzz run is not a universal panic-freedom claim.

## Public error categories

The Rust error surface now describes failure classes rather than individual
refusal sites: `AnalysisError` has 5 variants (previously 151), `ProfileError`
has 7 (previously 29), and `PacketError` has 7 (previously 26). Observed lengths,
indices and causes remain in diagnostics; wrapped domain errors retain their
`Error::source` chain. This is a breaking Rust API change. The language-shell
error codes are unchanged.

Tests exercise real invalid configuration, PCM geometry and terminal-state
failures through the new categories. Diagnostic strings are constructed only
on failure, avoiding new allocations on successful lookups.

## Synthesis checks

The previous decode peak-error bound accepts same-length zero PCM. A live test
now compares native synthesis to the separate NumPy synthesis on the paired 6ch
fixture and paired 2ch stereo-noise container. On the audit machine:

| Input | Overall RMS difference | Maximum sample difference | Minimum channel correlation |
|---|---:|---:|---:|
| 6ch fixture | 2.592e-10 | 4.310e-8 | 0.999999999999932 |
| 2ch stereo noise | 0.000000116503 | 0.000000751606 | 0.99999999999997 |

The initial 6ch comparison had a normalized RMS difference of about 0.13 on its
quietest channel. Packet coefficients matched exactly; an external float decode
agreed with Rust at rounding scale. The NumPy generic path used a fitted
fixed-overlap reconstruction operator, which was wrong even in consecutive short
blocks; it also lacked hybrid window support. It now evaluates the float64 IMDCT
definition directly and derives window support from neighbouring block sizes.
A small long/short transition test pins that geometry.

After the repair, the largest 6ch normalized RMS difference is below 2e-7. The
test now requires normalized RMS at most 1e-5 and correlation at least 0.999999;
these are numerical tolerances, not sample identity. Synthetic mutations
verify rejection of silence, doubled gain, reversed sign, exchanged channels,
and one-frame misalignment. Shared profile values and bitstream utilities limit
the independence of this comparison.

## Remaining external-evidence limits

The native decoder was also compared against vgmstream r2117 on this audit
machine, with looping disabled and float32 output. The reproducible driver is
`scripts/check_decode_external.py`; it checks every sample in bounded blocks,
as well as declared geometry, output length and finite values.

| Input | Frames | Maximum sample difference | RMS difference |
|---|---:|---:|---:|
| Committed 6ch fixture | 139,398 | 5.96046e-8 | 3.01536e-10 |
| Committed 2ch stereo noise | 48,000 | 7.15256e-7 | 1.22993e-7 |
| Local 2ch long stream | 108,504,384 | 7.15256e-7 | 7.25544e-8 |

These are numerical comparisons, not cross-platform byte-identity claims. The
long input is not versioned, so its run is local evidence; a fresh checkout can
repeat the two committed cases with its own installed external decoder.

The committed whole-file anchors remain one 6ch vector and eight 2ch vectors.
The historical 96,000-frame pair is still missing. The long external-decoder
input has not been promoted to the committed corpus. Additional paired-build vectors require
verified paired-build outputs; this project's own output cannot supply that
independence. Broader 6ch coverage and independently auditing all compiled
calibration values remain open evidence gaps.

The documentation check resolves local links and current source-file paths. It
does not infer whether prose, numerical claims or arbitrary symbol names are
semantically correct. Rustdoc separately checks Rust intra-documentation links.
