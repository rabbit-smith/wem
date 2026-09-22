# Encode performance: the measured split, and where the headroom is

**Result.** The release build encodes the reference fixture (`tests/fixtures/
input.wav`, 139 398 frames = 3.161 s, 6 ch/44100) in a median **122.4 ms** —
**25.8× real time**. That confirms the ~25× figure this investigation started
from. The time is not spread evenly: **Vorbis packing and the long-frame
psychoacoustic analysis are 74% of it**, and inside the analysis the FFT log
curve alone is a third of the whole encode. Two findings are worth acting on;
the rest are real but small, and two candidate pathologies are refuted.

This document is the evidence record. It is a measurement, not a budget: there
is no threshold here, and `scripts/measure_encode_perf.py` has none either
(see [Trust boundary](#trust-boundary)).

**Ledger status after landing.** The numbers below were measured on this
branch's tree before this document landed (ee542f4; every line reference was
re-checked at c1c8f73, the branch tip). `main` had meanwhile acted on four of
the findings from its own lanes, so each of those is marked **closed** in place
with the commit that closed it, and the tables below are the record of the
measured revision rather than a description of today's tree. The later
measurements live in [`duration-curves.md`](duration-curves.md),
[`concurrency-curves.md`](concurrency-curves.md) and
[`pool-sizing.md`](pool-sizing.md); this page keeps the result it started from,
and a closed item stays on it (see
[Watching performance](../guides/development.md#watching-performance)).

## What was measured, and how

Reproduce with:

```sh
cargo build --release -p wem-core
python3 scripts/measure_encode_perf.py            # numbers + machine identity
```

The per-stage split comes from `crates/wem-core/tests/stage_timings.rs`, an
`#[ignore]`d test-only harness (driven with `--ignored --nocapture`). It replays
`Encoder::encode_pcm`'s call sequence through the **public** kernel API
(`AnalysisSession`, `selected_windows`, `analyze_window`, `pack_analysis_packet`,
`build_vorbis_wem`) with `std::time::Instant` around each existing call. No
shipping code carries instrumentation for it. Its one assertion is a
comparison, not a threshold: the container the staged replay assembles must
equal the one `encode_pcm` produces, so the split can never describe a
pipeline other than the shipped one. It held for every repetition.

A second, clearly separated block *attributes* the coarse stages by re-running
`mdct_forward`, `wwise_log_curve`, the hybrid window
(`apply_vorbis_window_in_place`, once per channel-frame) and the transient
detector on the material the first block produced. Those are single-threaded
re-runs of the same work, not disjoint stages, and are labelled as such.

### Machine identity (numbers are meaningless without it)

| | |
| --- | --- |
| CPU | Apple M3 Max, 16 logical cores (12 P + 4 E) |
| Memory | 64 GB |
| OS | macOS 27.0 (26A5378j), Darwin 27.0.0, arm64 |
| Toolchain | rustc 1.96.0 (ac68faa20), host aarch64-apple-darwin, LLVM 22.1.2 |
| Build | `cargo build --release -p wem-core`, default features (`parallel` on) |

## The measured split (fixture, release, median of 11 / 7)

| Stage | Site (at the measured revision) | median ms | share | frames |
| --- | --- | ---: | ---: | ---: |
| CLI: `wav_load` + `profile_assembly` + `output_write` | `crates/wem-core/src/bin/wwise-wem.rs:69-103` | 1.94 | 1.6% | — |
| PCM decode to float rows | `crates/wem-core/src/encoder.rs:635` (`to_float_rows`) | 0.59 | 0.5% | — |
| Input conditioner | `crates/wem-analysis/src/session.rs:214-225` | 0.12 | 0.1% | — |
| Scheduling + window materialization | `crates/wem-analysis/src/session.rs:504-519` | 17.04 | 14.8% | 205 planned |
| ↳ transient detection (of the above) | `crates/wem-analysis/src/transient/detector.rs` | 3.38 | 2.9% | 2289 quanta |
| Short-frame analysis | `crates/wem-analysis/src/session.rs:550-590` | 11.21 | 9.7% | 77 |
| Long-frame analysis | `crates/wem-analysis/src/session.rs:609-640` | 43.05 | 37.4% | 128 |
| Vorbis packing | `crates/wem-core/src/pack.rs:137-155` | 43.36 | 37.7% | 205 |
| Container assembly | `crates/wem-container/src/wem.rs` | 0.05 | 0.04% | 206 packets |
| **staged total** | | **115.00** | | |
| **`encode_pcm` end to end** | `crates/wem-core/src/encoder.rs:613` | **114.91** | | |

`encode` reported by the CLI over 11 runs: min 119.88, **median 122.38**,
p95 126.37, max 129.46, spread 9.58 ms. The CLI's `encode` is the staged total
plus the process-level effects the replay does not have (first-touch of a fresh
output buffer, the `--output /dev/null` write).

### Attribution (single-threaded re-runs, not additive)

| Inner stage | Site (at the measured revision) | median ms | calls |
| --- | --- | ---: | ---: |
| FFT + log curve | `crates/wem-analysis/src/dsp/spectrum.rs:104-119` | 37.49 | 1230 |
| ↳ the twiddle recurrence inside it (cost model, finding 1) | `spectrum.rs:82-87` | 19.26 | 9 123 840 steps |
| MDCT | `crates/wem-analysis/src/dsp/transform.rs:316-395` | 4.60 | 1230 |
| Frozen-window rebuild | `crates/wem-analysis/src/dsp/transform.rs:507-520` | 1.10 | 2460 |
| Transient detector | `crates/wem-analysis/src/transient/detector.rs:243` | 3.38 | 2289 |

**Where the time goes, in one line:** the long-frame psychoacoustic chain
(43.05 ms, 37.4%) and Vorbis packing (43.36 ms, 37.7%). Inside the long chain,
the FFT log curve is 37.49 ms — 87% of it — of which the twiddle recurrence is
19.26 ms. MDCT is 4.60 ms (4.0% of the encode): the transform is not the cost,
the spectral and envelope work around it is.

### Scaling in stream length (no quadratic term)

| Input | frames | audio | median encode | ratio |
| --- | ---: | ---: | ---: | ---: |
| 1× fixture | 139 398 | 3.161 s | 123.71 ms | 25.6× real time |
| 10× payload | 1 393 980 | 31.61 s | 1202.51 ms | 26.3× |
| 40× payload | 5 575 920 | 126.44 s | 4597.49 ms | 27.5× |

Linear to within 7%, and mildly *better* per sample at length (amortized
start-up), not worse. **Nothing in the frame or packet path is quadratic in
stream length.**

## Findings

### 1. The FFT rebuilds its twiddle factor per butterfly — algorithmic, act — **closed by 915050b**

**Closed.** Commit 915050b (*derive the FFT twiddles once per stage instead of per
butterfly*) landed exactly the remedy below: the sequence is built once per stage
by the identical recurrence, through the same two `f32` rounding points, with
the step still read from the frozen profile bank, and the butterfly body is
unchanged. Its paired, order-alternated measurement on a busy machine (load
50-190, which is why the load is stated): fixture encode median 117.38 →
114.07 ms, child user CPU 223.18 → 204.82 ms, scalar build 155.52 → 142.71 ms,
FFT log curve 51.19 → 34.88 ms. The wall-clock gain is smaller than the model
estimate below, and the closure message gives the reason: the long-frame FFT
runs inside the per-channel regions of finding 2, whose time is dominated by
dispatch, so removing arithmetic inside them does not shorten them — the scalar
build shows the full saving. The estimate and the outcome are both kept here.

**Where.** `crates/wem-analysis/src/dsp/spectrum.rs:68-89`. Inside the
per-stage loop, `wr`/`wi` are reset to `(1, 0)` for **every** `start` block
(line 69-70) and advanced by one complex multiplication per butterfly
(lines 82-87).

**Cost.** `wwise_fft_packed` is 37.49 ms of the 115.00 ms sequential staged
total (32.6%). The recurrence replayed at the shipped iteration count — 9 123 840
steps, same operations, same `f32` rounding points — costs **19.26 ms**, i.e.
51% of the FFT. Process-profile agreement: `sample` on a single-threaded
(`--no-default-features`) release build of a 40× input puts
`wem_analysis::dsp::spectrum::wwise_fft_packed` at 25.7% of all leaf samples,
the single hottest function in the binary (next: `Codebook::best_vq` 15.6%).

**Why it is redundant.** `(wr_k, wi_k)` is a pure function of `(step_re,
step_im, k)`: `step_re`/`step_im` come from the frozen profile bank for the
stage's `length`, and the recurrence starts from the same `(1, 0)` in every
block. The value is recomputed identically `n/length` times per stage per call.

**Remedy.** Derive the sequence once per stage — a `Vec<(f64, f64)>` of `half`
entries built by the *identical* recurrence, or a table in the profile's frozen
domain — and read `tw[k]` in the butterfly.

**Byte-exactness.** The stored values are the recurrence's own values, so every
butterfly receives the same `(wr, wi)` pair it receives today; the butterfly's
arithmetic (lines 74-81) is untouched. The change removes work, it does not
reassociate or reorder any sum. Proven by `cargo test -p wem-core --test
complete_wem_bytes` (whole-file SHA-256), `-p wem-core --test frame_pipeline_parity`
(per-frame, per-stage values against the oracle) and `-p wem-core --test
stage_parity`.

**What it would buy.** Up to ~19 ms of single-threaded CPU; the region is
parallelized across channels, so the wall-clock share is smaller — roughly
19.26 / 1.85 ≈ **10 ms of the 123.5 ms (~8%)**, and removing the serial
dependency chain should beat that estimate rather than miss it.

### 2. The per-channel rayon partition returns 1.26× on six channels — algorithmic, act — **carried forward: 6c70b0a, 92b25cc, fafa463**

**Landed, and continued elsewhere.** Three commits took this up from the other
side: 6c70b0a announces one job per channel instead of an adaptive range
(`analyze_long` 44.42 → 42.56 ms, and the parallel-to-scalar ratio moved only
1.249× → 1.264×, with the measured reason being per-frame pool cold starts);
92b25cc builds the long seed look once per encode rather than once per channel
job (~17 ms of CPU per encode); and fafa463 gives the two waves a pool the
session owns and sizes to the channel count, because a six-way job handed to
sixteen workers is reached through a chain of steals (30.8% more CPU for the
same bytes). The remedy below named a latency-oriented pool or fewer, larger
regions; the shape that landed is the pool. Its evidence record — the sweep, the
load-insensitivity argument and what it proves — is
[`pool-sizing.md`](pool-sizing.md), and the duration scaling it rests on is
[`duration-curves.md`](duration-curves.md). Whether the feature should be
default-on at all is a separate decision with its own lanes and is not this
page's finding.

**Where.** `crates/wem-analysis/src/psychoacoustics/pipeline.rs:443-461`
(`transform_channel_rows`) and `:480-496` (`channel_psych_map`), reached from
`analyze_long_frame` for every long frame. Two `into_par_iter()` regions per
long frame × 128 long frames = **256 parallel regions**, each covering six
channels of ~350 µs of work. The short-frame path is already sequential
(`pipeline.rs:344-347`).

**Cost.** Same fixture, same machine, same release profile, the two builds
`cargo build --release -p wem-core` and `... --no-default-features`, 11 runs
each:

| Build | min | median | max |
| --- | ---: | ---: | ---: |
| default (`parallel` on) | 120.14 | **123.49** | 134.72 |
| `--no-default-features` (scalar) | 154.35 | **156.93** | 168.40 |

**1.26× on 6 channels of a 16-core machine.** The harness agrees: `analyze_long`
is 79.8 ms scalar and 43.05 ms parallel — a **1.85×** speedup of the one
parallelized region (whole-encode 156.93 → 123.49 = 1.27×, consistent). A
perfect 6-way partition would return ~5×; the gap is per-region pool
wake-up/park, not the arithmetic. The `sample` profile of the default build
agrees from the other side: `__psynch_cvwait` 67% and `swtch_pri` 20% of all
leaf samples, with `crossbeam_deque::Stealer::steal` (102), `crossbeam_epoch::
with_handle` (93), `rayon_core::sleep::Sleep::wake_specific_thread` (13) and
`WorkerThread::wait_until_cold` (16) all more expensive than `mdct_forward`
(82).

**Remedy.** Fewer, larger regions rather than removing the feature (scalar is
32 ms *slower*). The two regions per frame cannot simply merge — `global_specmax`
is a genuine cross-channel barrier between them
(`pipeline.rs:154-157`) — so the options are to reduce the number of wake-ups
per region (a latency-oriented pool, or a pool that does not park between the
two regions of one frame) or to batch the frames' transforms where the
cross-frame state allows.

**Byte-exactness.** Any remedy here is a scheduling change over per-channel
independent work. The existing safety argument at `pipeline.rs:131-140`
already covers the feature and is guarded end to end by the same three tests;
a coarser region inherits it only if it keeps the per-channel read/write
disjointness that argument states. Re-prove with `complete_wem_bytes`,
`frame_pipeline_parity`, `stage_parity`.

**What it would buy.** Moving from 1.26× to even 2.5× would return ~60 ms of
the 123.5 ms. This is the largest single number in this document and the one
with the least certain remedy.

### 3. The one-shot path materializes the whole analysis input before analyzing a frame — algorithmic, small in ms, opposite of the streaming path — **closed by 539194e**

**Closed.** Commit 539194e (*hand out one analysis frame at a time from session
scratch*) made the one-shot path own its PCM-derived state for one conversion and
refill caller-owned row buffers one frame at a time (`PlannedWindowSource`), the
batch counterpart of the streaming feeder, so peak frame storage is one frame
instead of the 13.5 MB below. `encode_pcm` keeps that scratch and takes the rows
back from the finished frame; `condition_pcm` borrows its rows unless a
conditioner rewrites them. Values, rounding points and the order frames are
analysed in are unchanged, and the streaming path was not touched.

**Where.** `crates/wem-analysis/src/preprocessing/windowing.rs:43-173`:
`iter_planned_pcm_windows` returns `Vec<WindowedFrame>`, every frame of the
stream, each holding `Vec<Vec<f64>>`; `crates/wem-core/src/encoder.rs:636`
consumes it afterwards. The streaming path does the opposite one frame at a
time over a bounded ring (`crates/wem-core/src/stream.rs:262-300`).

**Cost.** 128 long × 6 ch × 2048 + 77 short × 6 ch × 256 = 1 691 136 `f64` =
**13.5 MB resident simultaneously**, where one frame needs 96 KB. The
per-channel-frame `raw` (`windowing.rs:128`) and `apply_vorbis_window`'s `out`
(`transform.rs:524`) are two fresh allocations each: **2460 allocations,
27.1 MB of churn** for one encode. Measured cost in time is small —
`plan_and_windows` 17.04 minus `select_modes` 12.14 = **4.90 ms (4.3%)** — so
this is a memory-shape finding, not a speed finding.

**Verdict: algorithmic, act only alongside other work in this file.** An
iterator or a caller-owned scratch frame at the session boundary would remove
both the residency and the churn, and would bring the one-shot path in line
with the crate's own rule ("no `Vec` returns from inner stages where a scratch
buffer exists; keep scratch ownership at the session boundary") and with the
streaming path. Byte-exactness is immediate — same values, same order, only
where they live changes — and `complete_wem_bytes` plus
`stream_giant_chunk_segmentation_parity` (`crates/wem-core/tests/streaming.rs`)
already pin the two paths against each other.

### 4. Per-call scratch allocations in the inner stages — algorithmic but diffuse, ~9% ceiling — **the named rows closed by 539194e**

**Closed for the rows this finding named as worth taking.** 539194e removed
the three redundant ones: the per-channel-frame `raw` row (the source now
refills a caller-owned row), `apply_vorbis_window`'s `out` copy (the hybrid
window reads the frozen half directly and applies it in place — the
`half ++ reverse(half)` rebuild was never read), and the whole-PCM `to_vec()`
when the profile selects no conditioner (the rows are borrowed). 92b25cc
removed a fourth of the same kind, the long seed look rebuilt per channel job.
The rest of the table stays an observation: those allocations are below the
noise floor individually, which is what the verdict below already said.

**Where.** Each of these allocates once per call on the hot path:

| Allocation | Site | Calls/encode | Bytes |
| --- | --- | ---: | ---: |
| `real`, `imag`, `packed` | `wem-analysis/src/dsp/spectrum.rs:41-42,93` | 1230 | 3 × n × 8 |
| `w`, `out` | `wem-analysis/src/dsp/transform.rs:327,379` | 1230 | (n + n/2) × 8 |
| `out = samples[..n].to_vec()` | `wem-analysis/src/dsp/transform.rs:524` | 2460 | n × 8 |
| frozen-window rebuild (`get_window`) | `wem-analysis/src/dsp/transform.rs:507-520` | 2460 | 4 × n/2 × 8 |
| `raw: Vec<f64>` | `wem-analysis/src/preprocessing/windowing.rs:128` | 1230 | n × 8 |
| `work` rows + `partword` | `wem-vorbis/src/residue.rs:349-368` | 205 | ~2 × nch × n × 8 |
| `posts.to_vec()` + `out` | `wem-vorbis/src/floor.rs:401-402` | 1230 | ~2 × nvals × 8 |
| whole-PCM `to_vec()` | `wem-analysis/src/session.rs:223` | 1 | **6.69 MB** |
| `packet.clone()` ×2 | `wem-core/src/stream.rs:326,595` | 205 | ≤ 218 KB |

**Cost.** Allocator `malloc`/`free` self-time is **6.9%** of the single-threaded
profile, with `_platform_memmove` 1.3% and `_platform_memset` 0.9% alongside
it — about **9%** for allocation and the copying it implies. The frozen-window
rebuild, the largest *redundant* one, is measured directly: **1.10 ms (1.0%)**.

**Verdict: split.** The whole-PCM `to_vec()` at `session.rs:223` runs when the
profile has no input conditioner — which is the fixture's case
(`conditioner=false`) — and it is pure waste: 6.69 MB copied, measured
0.12 ms, and 6.69 MB of residency. That one is worth removing on its own
(borrow the rows; the caller already owns them). The rest are below the noise
floor individually and only add up under an ownership redesign; the
frozen-window rebuild in particular is **1%, not the pathology it looks like**
— measured, not guessed.

### 5. Streaming is 3.8% slower than one-shot, independent of chunk size — not actionable

`StreamSession` over the fixture: one chunk 119.19 ms, 4096-frame chunks
119.44 ms (median of 7); `encode_pcm` 114.91 ms. **+4.3 ms (+3.8%)**, and
chunking does not matter, so it is a constant, not a per-chunk cost. All of it
sits in `finish` (4.1 ms, against the batch path's 0.03 ms container assembly):
the delayed tail frames and the end-of-stream work the batch path pays up front
inside `plan_and_windows`. The two extra `packet.clone()`s on that path
(`stream.rs:326` and `:595`) copy at most 218 KB — about 0.02 ms — so they are
**not** the cause and are not worth changing for speed.

**Verdict: noise.** The path buys chunking invariance and bounded input memory
for 3.8%; leave it alone.

### 6. Refuted: quadratic behaviour, and the MDCT as a bottleneck

- **Quadratic in frames or packets:** refuted, see the scaling table — 25.6× /
  26.3× / 27.5× real time at 1× / 10× / 40× payload. The one genuinely
  quadratic routine found, `floor1_neighbor_tables`
  (`wem-vorbis/src/floor.rs:322-350`, O(postlist²), recomputed identically
  1230 times from a `postlist` that depends only on the setup floor, via
  `floor.rs:401` and `floor_fit.rs:465`), is **0.73%** of CPU. Deriving it once
  per floor is correct and trivially byte-safe, but it is not worth a change on
  its own.
- **The MDCT:** `mdct_forward` is 4.60 ms of 115.00 ms — **4.0%**, and 2.9% of
  the process profile. Its butterfly order is fixed by the bit-exactness rule
  and its scratch allocation is at the noise floor. **Do not optimize it.**
- **`Codebook::best_vq`** is 15.6% of the profile and is the second hottest
  function, but its comparison is a plain `f64` L2 sum whose accumulation order
  decides the argmin on ties. Any reformulation (norm precomputation, the
  dot-product identity, a partial-distance cutoff) changes that sum and
  therefore could move bytes. **Reported as an observation, not a proposal**:
  it is off limits without its own byte-exactness argument.

## Trust boundary

- These numbers are one machine (Apple M3 Max, macOS 27, rustc 1.96.0), one
  build configuration (release, `parallel` on unless stated) and one tree (the
  measured revision named under *Ledger status after landing*, which `main` has
  since changed in the places findings 1-4 name). They are not a budget and
  nothing in the tree compares against them.
- The stage split is wall-clock around shipped calls, so it inherits the
  parallel regions: `analyze_long_ms` is a wall-clock figure over a
  per-channel rayon partition, not CPU time, and the attribution block is
  single-threaded by construction. The two are not additive and are not
  presented as such.
- The twiddle cost model replays the recurrence standalone; it prices the
  redundancy, it does not prove what removing it would save (the removed
  multiplies also shorten the dependency chain, which this model does not
  account for). The closure of finding 1 is what settled that: 3.31 ms of wall
  clock against a 19.26 ms model, with the difference attributed to the
  dispatch-bound region around it.
- No file under `crates/*/src/` was changed for this investigation, so every
  number describes the tree as committed at the time, and every byte-exactness
  suite ran unchanged.
- The load the machine was under is part of any *paired* reading, which is why
  `scripts/measure_encode_perf.py` prints the load average before and after
  every series. That arrived after this measurement, so this page's numbers
  carry the machine identity and the machine's role in the investigation, not a
  load average; the readings that follow it do.