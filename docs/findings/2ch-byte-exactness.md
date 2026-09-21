# 2ch/48 kHz byte exactness

Date: 2026-09-20. This page is the evidence record for the 2ch/48 kHz result:
what was proven, the six root causes behind it, the accounting that closed, and
what the corpus does not prove. The diagnostic method that produced it is the
sibling playbook [`../methodology/byte-exact-diagnosis.md`](../methodology/byte-exact-diagnosis.md);
current limits and outstanding work are in [`../roadmap.md`](../roadmap.md).

## Result

The fixed paired input is 96,000-frame, 2ch/48 kHz PCM with input SHA-256
`90c3a5b4b2c5008badff42da7e9ecc46d0ec87c944f9aa32a1151903da2a1ff3`.

| Implementation | Bytes | SHA-256 |
|---|---:|---|
| Wwise 2013.2.10 build 4884 | 37,658 | `41fe43e2b99077e9ae6f57a00443d4a8dfc60513b8c5c46311b9be97e83ef629` |
| Rust native kernel | 37,658 | `41fe43e2b99077e9ae6f57a00443d4a8dfc60513b8c5c46311b9be97e83ef629` |
| Python oracle | 37,658 | `41fe43e2b99077e9ae6f57a00443d4a8dfc60513b8c5c46311b9be97e83ef629` |

All three complete WEMs are byte-for-byte identical and all 142 audio packets are
exact, with a mode sequence of 55 short + 87 long.
`tests/integration/test_core_oracle_golden.py` also holds an asset-free
deterministic 2ch check pinning the output SHA, packet count and short/long
counts for the native core, the direct core and the oracle; those layers are
described in [`../reference/architecture.md`](../reference/architecture.md).

## Root causes

The final difference was assembled from six mutually independent gaps, each
established by independent observation rather than fitting
([method](../methodology/byte-exact-diagnosis.md)).

### 1. Input boundary

The real build applies a stateful DC high-pass first, then a signed-16 storage
boundary: `float32((x - previous_x) + coefficient * previous_y)` with coefficient
bit pattern `0x3f7f546d`, followed by
`round_ties_even(float32(y * 32767)) / 32768`. All 192,000 output samples of the
complete conversion were checked bit for bit.

### 2. Residue coupled quantization and classification

The 2ch type-2 path must use aoTuV beta6.03's coupled quantization, nonzero
propagation and coupled-mid classification table. The real handoff's 217,088
quantized integers and 4,836 partition classes were both closed out by the
repository decoder.

### 3. Psychoacoustic look

The runtime reads the `tonecurves` pointer map of the four looks directly. The
two short looks share one 17×8×58 table and the two long looks share one table,
but the 2ch/48 kHz tables differ from the 6ch/44.1 kHz tables that had been
carried over. The short table's raw bytes have SHA-256
`dab0ce2556b2dbacc8927579b03d033eb963ae7c68238c4b72f16ee91e8f2e96`; the long
table's is `a4b2eb1e15485451897beac834e6fce858c6dac062537601dd1e4a24a033399f`.
Directly replacing the 2ch tone bank lifted the audio packets from 119/142 to
139/142.

### 4. Short look

The short remap's peak cap must follow the transient selection of
`short_look_0/1`; fixing it to look 0 fails whenever a strong low-frequency peak
triggers. This makes frames 135 and 138 exact.

### 5. Transient energy ring

In the 15-slot energy state, `last_energy` is the sliding accumulator and
`energy_sum` the segmented accumulator rebuilt each round; the original
implementation swapped their roles, so the falling edge arrived one 64-sample
quantum early and the wrong short profile was selected. The fix rests on the
saved per-instruction layout of the detector's energy-update section; no
threshold was fitted to the target packets.

### 6. EOS state

The 32-tap LPC training length is the amount of PCM still in the analysis buffer
when the end marker arrives, truncated at `blocksizes[1]`; it varies with the
actual hop of the last blocks and cannot be fixed at 2048. Public Xiph
`vorbis_analysis_wrote` uses the same dynamic length, then extrapolates with the
32-tap predictor
([block.c](https://github.com/xiph/vorbis/blob/1b75110b5a2754ba1931d82dd83cb822b266a21d/lib/block.c#L468-L505),
[lpc.c](https://github.com/xiph/vorbis/blob/1b75110b5a2754ba1931d82dd83cb822b266a21d/lib/lpc.c#L53-L147)).
Correcting the EOS training window made frame 141's 128 predicted samples
bit-identical and reached 142/142.

Packet-count attribution: 119/142 → 139/142 is the 2ch tone bank (cause 3);
frames 135 and 138 fall to the per-look peak cap (cause 4); frame 141 and the
final 142/142 fall to the EOS training window (cause 6). Causes 1, 2 and 5 are
established by full-sample and per-coefficient agreement, not by a packet-count
step.

## Closed bit ledger

This +990 B decomposition is the closed baseline from before the aoTuV coupled
quantization was implemented: it documents the gap of the older implementation —
38,648 B, SHA-256
`3a31227bd4ea175c21f7cdc6218daa083d1aa1a182b54b7834e0bdc334225fa9` — against the
build's 37,658 B, and does not describe a current residual difference.

Coefficients, classwords and per-stage VQ entries were decoded by the repository
type-2 decoder in `scripts/decode_wem.py`; the one-off summary script was not
kept as a tool. Both sides decode exactly 142 audio packets, 55 short and 87 long,
with the same complete mode sequence; every residue satisfies
`status in {complete, empty}` and leaves fewer than 8 bits at the packet end.
Per-packet statistics cover every packet, with no selection by whether samples can
be paired.

| bins | Build bits | Then-implementation bits | Then − build |
|---|---:|---:|---:|
| 0–64 | 69,584 | 66,357 | **−3,227** |
| 64–128 | 31,005 | 34,658 | **+3,653** |
| 128–256 | 44,620 | 47,463 | **+2,843** |
| 256–512 | 82,147 | 83,483 | **+1,336** |
| 512–1024 | 35,292 | 38,268 | **+2,976** |
| **VQ total** | **262,648** | **270,229** | **+7,581** |
| Classword, all bins | 14,770 | 15,201 | **+431** |

VQ and classword together are 8,012 bits more, while the actual audio packet
payload is 7,920 bits more — exactly the **990 B**; the remaining −92 bits come
from floor, header and per-packet padding. This decomposition closes the account.

Category and reconstruction agreement over the same packets:

| bins | Same class | Same coefficients | Build nonzero | Then nonzero |
|---|---:|---:|---:|---:|
| 0–64 | 39.3% | 61.9% | 12,378 | 11,603 |
| 64–128 | 35.7% | 83.1% | 6,286 | 6,058 |
| 128–256 | 60.9% | 79.1% | 10,693 | 9,138 |
| 256–512 | 66.2% | 73.2% | 19,066 | 19,405 |
| 512–1024 | 58.0% | 93.6% | 7,411 | 9,689 |

The 64–256 output is not larger because of having far more nonzero coefficients
than the build: the reliable decoder finds fewer, so the overspend comes from
different class/stage/book and different VQ entry code-length combinations. One
directional phenomenon is that the older classes sat mainly in 1/3/5/7 while the
build uses 2/4/6/8 heavily in the same partitions, and the reconstructed residue's
channel 1 magnitude is systematically lower (128–256 mean absolute value 0.724
build against 0.561 then). With the type-2 classifier using channel 0 as magnitude
peak and channel 1 as angle peak, that supports the candidate "the integer angle
residue differs before classification" — still an inference, since VQ
reconstruction values cannot stand in for the integer input the build-side
classifier reads.

## Representative corpora

Six 48,000-frame real-build cases cover silence, opposed DC, low tone, high tones,
isolated impulses and independent stereo noise. Their WAV and WEM pairs live in
`tests/data/2ch-reference/`; the complete WEM of all six inputs is byte-identical
to the Rust output, and `tests/contract/test_2ch_reference_corpus.py` pins the
input/output SHA-256, the packet count and the short/long counts.
`tests/data/2ch-reference/manifest.json` records each case's digests and counts.

The two stress cases live in `tests/data/2ch-stress/` and are part of the default
green suite:

- A square-wave burst alternating between left and right every 2,048 samples: all
  369 audio packets are exact and the complete WEM is 35,172 B. It covers
  multi-channel OR, the falling edge, consecutive short-profile switching and a
  long short run.
- A tail signal that changes in the last 2,048 samples: all 68 audio packets are
  exact and the complete WEM is 4,909 B. It covers the dynamic EOS LPC training
  length and the last frame.

`tests/contract/test_2ch_stress_corpus.py` checks the batch kernel, the Python
oracle and the irregular-chunk streaming kernel; all three paths are whole-file
byte-identical to the real build. The reference and stress WAVs are
deterministically rebuilt by `scripts/generate_2ch_reference_inputs.py` and
`scripts/generate_2ch_stress_inputs.py` respectively.

## Confirmed handoff algorithm

The two-channel residue handoff of Wwise 2013.2 is not the `round(mdct / floor)`
plus reversible mapping coupling the repository implemented before, and stock
libvorbis 1.3.3 does not describe it sufficiently. The function the Wwise runtime
calls matches aoTuV beta6.03's `_vp_couple_quantize_normalize`.

Three observations are decisive:

1. Audiokinetic's Wwise 2011.2.2 release notes record that the Vorbis encoder was
   updated to aoTuV beta6.03, and the 2013.2 the paired build still presents that
   function's parameters, structure fields and branch constants.
2. Runtime observation gives a short block `n=128, normal_partition=8` and a long
   block `n=1024, normal_partition=32`; the point limits are 42/341, the encoding
   lowpass 96/768, the pre/post point threshold 0/2.5 and the rephase threshold
   0/0.5.
3. Feeding the same call's raw MDCT, integer floor curve, impulse peak and nonzero
   inputs into the public aoTuV algorithm reproduces two sample batches' 217,088
   output integers coefficient for coefficient, with difference 0; the samples
   cover both short and long.

This establishes that classification and VQ must be preceded by aoTuV's joint
quantize/coupling. Disassembly addresses only propose candidates; the final
semantics come from function input/output observation together with public source.

For every normal partition the confirmed steps are:

1. The floor1 encoder first produces the integer 0..255 floor curve; the
   quantizer then takes magnitudes through `FLOOR1_fromdB_LOOKUP`.
2. `flag_lossless` computes `mdct / floor` and, with the coupling point limit,
   impulse peak, point threshold and rephase threshold, selects lossless, point or
   rephase candidates.
3. Each channel is integerized and noise-normalized first; this profile has
   `normal_start=9999`, which degenerates to nearest-even `rint` over the observed
   interval, but the algorithm's partitioning and later coupling still apply.
4. M6 measures the phase-inversion ratio and the two-channel residue deviation
   over whole blocks; it may promote the current partition's rephase candidate to
   lossless.
5. The lossless branch couples the floating-point residue and the already
   integerized residue at once; the point branch uses aoTuV's
   `min_indemnity_dipole_hypot`, zeroes the angle and then quantizes the
   magnitude.
6. Coefficients after the lowpass are cleared, and the coupling pair's nonzero
   state propagates to both rows.

The short block's `normal_partition=8` is the key value: with 32 hardcoded, long
blocks matched completely while short blocks still differed; with the
runtime-observed 8, short and long match coefficient for coefficient.

Two independent fixes belong with this path. Type-2 is skipped only when all
channels are unused: when either floor of a coupling pair is used, the nonzero
state must propagate to both rows and the fixed flat stride `[M0,A0,M1,A1,...]`
must be kept; Python and Rust now propagate that state explicitly, with a
regression test for the case where one side's floor is unused. Classword setup
covers the complete `10²` combinations; the old code silently substituted the
smallest available entry for an unencoded classword, which changed the
classification, and now raises an explicit error.

The quantization function being identical on real Wwise input proves the handoff
semantics directly. The final WEM also depends on the upstream MDCT, floor fit,
impulse peak, frame plan and downstream class/VQ packing; those boundaries were
closed afterwards through the complete WEM, per-packet comparison and the
repository decoder's independent observation.

Primary sources:

- [aoTuV beta6.03 official source](https://ao-yumi.github.io/aotuv_web/source_code/libvorbis-aotuv_b6.03.tar.bz2),
  in particular `flag_lossless`, `noise_normalize` and
  `_vp_couple_quantize_normalize` in `lib/psy.c`.
- [Wwise 2011.2.2 Release Notes](https://www.audiokinetic.com/download/documents/Wwise_v2011.2.2_ReleaseNotes.pdf),
  which record the encoder update to aoTuV beta6.03.
- [Vorbis I specification](https://xiph.org/vorbis/doc/Vorbis_I_spec.html), used
  to confirm mapping nonzero propagation, the type-2 flat domain and decoder
  order.
- [Xiph libvorbis v1.3.3](https://github.com/xiph/vorbis/tree/v1.3.3), used to
  check the generic type-2/classword/VQ structure; it does not substitute for
  aoTuV's encoder semantics.

## Retracted conclusions

- The old hand-written reader had the classword group, stage and partition loop
  order wrong and then kept reading VQ from a desynchronised bitstream, so all of
  its numbers are void: the bins 64–128 nonzero count of 38× or 42×, the bins
  64–128 surplus of 21,518 bits, the bins 128–256 surplus of 40,014 bits, and the
  "0–64 current residue is too large" inference drawn from a paired subset. The
  reader has been deleted; the ledger above comes from the repository decoder's
  re-verification.
- "Floor excluded": the floor curves on both sides are close in the observed
  samples, but that does not mean the floor and the residue quantization can be
  separated — the aoTuV handoff consumes the integer floor curve together with an
  additional peak surface.
- "Tone bank excluded": earlier work carried the 6ch/44.1 kHz tables into 2ch and
  thereby excluded the tone bank as a source of difference. That exclusion is
  invalid — the 2ch/48 kHz tables differ, as the two table hashes above record,
  and replacing the 2ch tone bank moved audio packets from 119/142 to 139/142.
- Attempting only a reversible pre-image of the decoder's four-branch coupling
  cannot produce aoTuV's forward handoff and is therefore not a fix; stock Xiph
  1.3.3 explains the generic Vorbis structure but cannot establish Wwise's
  encoder branch.
- Continuing to adjust class/VQ packing was withdrawn once the integer residue
  handoff had been observed directly; that observation led to porting the joint
  quantization on the Python and Rust sides.

## Trust boundary

What is proven:

- Profile values come only from direct runtime reads or public reference
  implementations; nothing was fitted to the target output.
- Residue and classword comparisons use the repository decoder; statistics the
  early hand-written reader produced from loop desynchronisation have been deleted
  or marked historically invalid.
- Disassembly serves only to locate runtime observation points; semantics are
  confirmed again from observed inputs and outputs, the bitstream, or public
  source.

What is not proven:

- Float32 last-bit differences remain observable in `fft`/tone seed that do not
  change floor posts or the bitstream; byte correctness is therefore constrained
  jointly by the complete WEM, per-packet comparison and the two implementation
  paths.
- Independent real-build evidence covers one 96,000-frame paired input and eight
  48,000-frame representative and stress inputs. Boundary lengths such as 4,096,
  4,097 and 8,192 frames are covered by native/oracle agreement and the fuzz
  comparison, but have no independent real-build output, so the finite-corpus
  conclusions are not an exhaustive proof for arbitrary PCM.

The no-fitting rule: profile data comes from direct reads or published reference
algorithms, never output fitting, and local rules must stay anchored to
independent observables rather than being back-derived from the total byte count.
The maintenance rules that keep both paired results and the corpora unchanged
are in [`../roadmap.md`](../roadmap.md).