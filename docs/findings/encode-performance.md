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

**Ledger status — the quiet-window pass (2026-09-22).** A later pass re-took, in
a window with no other lane running, the pair this page was missing: the two
feature configurations paired and order-alternated on both installed geometries,
and the five-minute memory point on both paths. It is
[below](#the-quiet-window-pass-the-paired-feature-configurations-and-the-five-minute-point),
and it moves today's headline: the fixture encodes in a **99.41 ms** median
through the released CLI against the **122.38 ms** recorded here, and the
five-minute one-shot peak is **2 444.8 MB** against the 2 445.0 MB the sibling
record carries. The window is not why — four changes this page's own findings
name, and the pool-sizing change its finding 2 was carried into, landed between
the two readings. The pass also found that the per-stage table below has been
*the scalar configuration's* since `parallel` became opt-in, which is an
instrument finding in its own right. **Nothing on this page is rewritten**: every
number here stays where it is, with the revision and the machine it measured.

## What was measured, and how

Reproduce with:

```sh
cargo build --release -p wem-core --features parallel   # the CLI is parallel-only
python3 scripts/measure_encode_perf.py            # numbers + machine identity
```

The `--features parallel` was a default when these numbers were taken and is
explicit now (`crates/wem-core/Cargo.toml`): a default build is scalar, and the
CLI — the binary this page measures — is a bin with `required-features`, so a
bare `cargo build --release -p wem-core` builds no CLI at all.

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
| Build | `cargo build --release -p wem-core --features parallel` (the feature is opt-in; it was a default when these numbers were taken) |

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
`cargo build --release -p wem-core --features parallel` and the same kernel built
`--no-default-features`, 11 runs each — the CLI in both configurations, which is
what existed when this was measured. Both configurations are still reachable
today, though no longer from one source: `cargo build --release -p wem-core
--features parallel` builds the CLI, and the scalar side is
`crates/wem-core/tests/concurrency_worker.rs` — the instrument
`scripts/measure_concurrency.py` builds in both configurations — because a bin
with `required-features` has no scalar build.

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

## The quiet-window pass: the paired feature configurations, and the five-minute point

Date: 2026-09-22. Every number above was taken while other lanes were loading
this machine, which is why this page carries a machine identity but no load
average, and why its comparisons are quoted as direction rather than as results.
This section is the pass that closes the gap the ledger was missing: one window
with no lane running, the **two feature configurations paired and
order-alternated** on both installed geometries, and the **five-minute point**
on both paths. Nothing above is rewritten; what follows says which readings it
supersedes and which it leaves standing, and why.

### The window, and what "quiet" turned out to mean

No other lane was running and nothing of this lane's was running either. The
machine was nevertheless **not idle**, and saying so is part of the record. The
1-minute load average over the 15 minutes the measurements span was
**11.51-26.22, median 16.38**, against 16 logical cores; the resting floor,
sampled every 10 s for 150 s with nothing of this lane's running, was
**12-15**.

That floor is not a lane. It is system processes that have been resident for
days: a storage-management service pair that has been burning 50-75% of a core
since well before this window, the Metal shader compiler XPC service (178% on
its own at the floor), an OrbStack VM helper, a browser with its rendering
helpers, the window server, and four long-lived interactive agent processes.
Measured at the floor the process table held **674% of the machine's 1600%
across 16 cores, with 60 runnable threads**. So the honest description of this
window is *no lane running*, not *idle machine* — and its load sits at the low
end of the 17-230 range the sibling records already carry, rather than outside
it. A reader comparing this section with an earlier one should read it that way.

The consequence is the one [`duration-curves.md`](duration-curves.md) already
established, and it holds here: **the RSS and CPU columns are load-independent
and the wall columns are not.** This is not an assumption — the five-minute
one-shot peak below varies by 0.08% while the load around it moves by 5.4. Every
wall column in this section was taken at load 11.5-26.2 and is its least
portable part; the paired, order-alternated structure is what keeps a
*comparison* valid under that drift, since both arms of a round meet the same
machine moments apart.

### Machine identity (the same machine as every record on this page and its siblings)

| | |
| --- | --- |
| CPU | Apple M3 Max, 16 logical cores (12 P + 4 E) |
| Memory | 64 GB |
| OS | macOS 27.0 (26A5378j), Darwin 27.0.0, arm64 |
| Toolchain | rustc 1.96.0 (ac68faa20), host aarch64-apple-darwin, LLVM 22.1.2 |
| Tree | this page's tree at 5e4be10, the commit that last changed the reader |
| Load | 1-minute 11.51-26.22, median 16.38; stated per series below |

Before any of it, the tree was confirmed to be the tree it was thought to be:
`make wem-bytes` (whole-file container against the committed reference) and the
whole `tests/parity` layer, **139 tests, both green**. Nothing under
`crates/*/src`, `src/`, `tests/` or `scripts/` was changed for this pass, and no
instrument in the tree was edited; the driver that arranges the pairs is
scratch, outside the tracked tree.

### The two feature configurations, paired and order-alternated

The instrument is `crates/wem-core/tests/concurrency_worker.rs` — **one program
built twice** (`cargo test --release -p wem-core --test concurrency_worker
--no-run`, with and without `--features parallel`), so an arm cannot differ from
the other by its entry point or its startup. That matters here: the scalar
configuration carries **no CLI at all**, because `wwise-wem` is a bin with
`required-features`, so a comparison whose scalar arm were a CLI would not be a
comparison. Each run is one child doing exactly one encode, spawned and reaped
by the parent with `os.wait4`, so the CPU column is that child's own `ru_utime` +
`ru_stime` and nothing else's. Rounds are paired and order-alternated: within
one round both arms run once and the order reverses on odd rounds.

20 rounds per geometry. *Wall* is the child's own `encode_ms`, bracketing
`encode_pcm`; *child wall* (not tabulated) adds process start. CPU is per
encode.

| 10 s corpus, 6 ch/44100 — **load 13.83-16.55** | min | median | p95 | max | spread | CPU median |
| --- | ---: | ---: | ---: | ---: | ---: | ---: |
| scalar — `cargo build --release -p wem-core` | 469.62 | **476.45** | 491.54 | 494.97 | 25.35 (5.3%) | **481.16** |
| `--features parallel` | 365.26 | **373.80** | 407.06 | 490.01 | 124.75 (33.4%) | **655.99** |
| ratio parallel / scalar | 0.778x | **0.785x** | 0.828x | 0.990x | | 1.363x |

| 10 s corpus, 2 ch/48000 — **load 16.55-17.23** | min | median | p95 | max | spread | CPU median |
| --- | ---: | ---: | ---: | ---: | ---: | ---: |
| scalar | 173.68 | **176.23** | 183.47 | 184.53 | 10.85 (6.2%) | **180.96** |
| `--features parallel` | 161.26 | **163.56** | 178.86 | 180.37 | 19.10 (11.7%) | **220.57** |
| ratio parallel / scalar | 0.928x | **0.928x** | 0.975x | 0.977x | | 1.219x |

| fixture (3.161 s), 6 ch/44100 — **load 16.97-17.23** | min | median | p95 | max | spread | CPU median |
| --- | ---: | ---: | ---: | ---: | ---: | ---: |
| scalar | 131.45 | **133.84** | 137.59 | 139.65 | 8.20 (6.1%) | **138.64** |
| `--features parallel` | 97.00 | **99.39** | 105.57 | 106.53 | 9.53 (9.6%) | **195.67** |
| ratio parallel / scalar | 0.738x | **0.743x** | 0.767x | 0.763x | | 1.411x |

**What the pair says.** The opt-in configuration returns **1.27x wall for 1.36x
CPU** at 6 ch/44100/10 s, **1.08x for 1.22x** at 2 ch/48000/10 s, and **1.35x for
1.41x** on the fixture. The wall return tracks the channel count, and the CPU
premium is small at six channels — which is the shape
[`pool-sizing.md`](pool-sizing.md) chose the pool size to produce, arriving from
the other direction. The 2 ch geometry, two jobs for whatever pool it gets,
returns almost nothing in wall and still pays 22% more CPU: the same fact read
from the other side.

**The 6 ch parallel arm's spread is the column worth naming.** 33.4% of its own
median against the scalar arm's 5.3%, driven by two samples of twenty (490.01
and 402.69 ms). That is scheduling, not arithmetic: the same arm's `user` time
spreads 4.6% while its `sys` time spreads 49.6%. It is the per-region pool
wake-up this page's finding 2 named, still visible in the tail after the pool
was sized to the job count. **Quote the median for this arm; a p95 from 20
samples is not stable.**

**Cross-check, in tree.** `scripts/measure_concurrency.py` already owns this
child pattern and this pairing; run at one encode per arm
(`--n-values 1 --repetitions 10`, load 19.3-19.7) it reports the paired ratios
**latency 0.739x / CPU 1.400x** at 6 ch/44100 and **0.958x / 1.207x** at
2 ch/48000 — within 1% of the fixture's CPU ratio above and within 3% on both
other columns, from an instrument built for a different question. The 2 ch
disagreement is the input: that script's 2 ch arm is the 1 s reference corpus,
not the 10 s one, and at 1 s the arm's own process overhead is a fifth of its
encode.

### The released CLI, and what this page's 122.4 ms is worth today

The CLI is the released `wwise-wem` built with `--features parallel`, run as
`wwise-wem <wav> --output /dev/null --time`. Paired against the parallel worker,
20 rounds, **load 16.28-16.56**:

| fixture, 20 paired rounds | min | median | p95 | max | spread | CPU median |
| --- | ---: | ---: | ---: | ---: | ---: | ---: |
| parallel worker | 96.41 | **98.99** | 101.71 | 103.49 | 7.08 (7.2%) | 194.78 |
| released CLI | 97.86 | **99.41** | 105.17 | 117.04 | 19.18 (19.3%) | 195.22 |
| CLI / worker | 1.015x | **1.004x** | 1.034x | 1.131x | | 1.002x |

**The entry point is not a confound.** The CLI's `encode` stage and the worker's
`encode_ms` agree to **0.4% on the median and 0.2% on CPU** — which is what
licenses the scalar arm above to be the worker rather than a CLI, since the
scalar configuration has no CLI to use.

The same CLI through the tree's own instrument,
`python3 scripts/measure_encode_perf.py --runs 9`, **load 20.65 before and 19.16
after the series**:

| fixture, CLI stage timers | min | median | p95 | max | spread |
| --- | ---: | ---: | ---: | ---: | ---: |
| `wav_load` | 0.35 | 0.36 | 0.67 | 0.84 | 0.49 |
| `profile_assembly` | 1.46 | 1.51 | 2.53 | 3.18 | 1.72 |
| **`encode`** | **97.35** | **99.82** | **102.51** | **103.51** | **6.16 (6.2%)** |
| `output_write` | 0.04 | 0.07 | 0.10 | 0.11 | 0.07 |
| total | 99.23 | 101.84 | 104.91 | 105.55 | 6.32 |

**31.7x real time**, against the 25.8x this page records for the same fixture.

**Superseded.** The 122.38 ms median this page's opening result states — and the
min 119.88 / p95 126.37 / max 129.46 / spread 9.58 ms beside it — is superseded
by the **99.41 ms** median above (and, through the tree's own instrument, by
99.82 ms). The reading it replaces was provisional for the reason this page
already gives: it was taken with no load average recorded, on a machine other
lanes were loading. **But the load is not why the number moved.** Four changes
this page's own findings name, plus the pool-sizing change finding 2 was carried
into, landed between the two readings; the same tree measured twice at the same
load returns the same number, and the 122.38 ms figure stays on this page as the
record of the revision it measured. The ratio moved the same way:
this page's 156.93 → 123.49 ms scalar-vs-parallel pair (**1.26x**) is now
133.84 → 99.39 ms (**1.35x**) on the fixture.

### The five-minute point

This is the case the product is actually used for. The instrument is
`crates/wem-core/tests/duration_curves.rs`: its `duration_curves_point` reporter
spawns one **fresh child** per point and reaps it with `wait4`, printing the
child's own `ru_maxrss_bytes`, `user_ms` and `sys_ms` — the same reporting
[`duration-curves.md`](duration-curves.md) established, driven here at 300 s
only. The inputs are that record's own generator,
`scripts/generate_duration_curve_inputs.py`; its cross-check against
`scripts/generate_2ch_long_program.py` passed, so the corpus is the one that
record measured, not a new one:

| input | geometry | frames | SHA-256 |
| --- | --- | ---: | --- |
| 300 s | 6 ch/44100 | 13 230 000 | `a6cfead72defb18d0d99a9b043837057d9f57adb113ccde968dd25eb855294ff` |
| 300 s | 2 ch/48000 | 14 400 000 | `2b765495648534614192a1e8ff8f749025c0f6afe643e1687372480160a4f6f6` |

Both paths, both configurations, paired and order-alternated, **10 rounds for
the parallel configuration and 6 for the scalar one** across three passes;
**load 12.22-25.69**. Peak resident of a fresh child, MB:

| 300 s | n | min | median | max | spread |
| --- | ---: | ---: | ---: | ---: | ---: |
| 6 ch one-shot, parallel | 10 | 2 234.1 | **2 444.8** | 2 446.0 | 211.9 (8.67%) |
| 6 ch one-shot, scalar | 6 | 2 339.2 | **2 441.1** | 2 442.0 | 102.8 (4.21%) |
| 6 ch streaming, parallel | 10 | 124.6 | **125.9** | 127.7 | 3.1 (2.47%) |
| 6 ch streaming, scalar | 6 | 121.1 | **122.1** | 122.8 | 1.8 (1.44%) |
| 2 ch one-shot, parallel | 10 | 1 214.3 | **1 216.5** | 1 217.0 | 2.7 (0.22%) |
| 2 ch one-shot, scalar | 6 | 1 213.7 | **1 214.9** | 1 216.2 | 2.5 (0.21%) |
| 2 ch streaming, parallel | 10 | 52.8 | **55.5** | 55.7 | 2.9 (5.11%) |
| 2 ch streaming, scalar | 6 | 53.2 | **54.6** | 55.2 | 2.0 (3.72%) |

**Against the earlier record.** [`duration-curves.md`](duration-curves.md)
records 6 ch/300 s at **2 445.0 MB** one-shot and **123.6 MB** streaming, and
2 ch/300 s at 1 214.8 and 53.8 MB.

| 6 ch/44100, 300 s | earlier record | this pass (median) | this pass (min) | change on median |
| --- | ---: | ---: | ---: | ---: |
| one-shot | 2 445.0 | **2 444.8** | 2 444.0 | **-0.01%** |
| streaming | 123.6 | 125.9 | 124.6 | +1.9% |

**The one-shot figure reproduces essentially exactly** — 2 444.8 MB against
2 445.0 MB, a 0.01% difference on a point whose own agreeing samples sit within
0.08% of each other. That is the number this pass exists to re-take, and it is
confirmed. **The streaming figure does not contradict the record but does sit
above it**: the record's 123.6 MB comes from that table's min/median column, and
today's minimum is 124.6 MB (+0.8%) while today's median 125.9 MB is +1.9% above
it. On a point whose ten samples span 2.47%, a single quoted figure for
6 ch/300 s streaming is worth about ±3%, and the record's value is inside that.
The 2 ch pair behaves the same way: one-shot 1 216.5 against 1 214.8 (+0.14%),
streaming min 52.8 against 53.8 (-1.9%) with a median 3.2% above it.

**Memory does not depend on the feature configuration.** One-shot ratios
parallel/scalar are **1.00x** at both geometries and streaming **1.03x** /
**1.02x**: the five-minute peak is a property of the path, not of whether the
kernel runs its channel waves in a pool. The earlier record's 2 445.0 MB was
taken with `parallel` on by default; today's scalar build reaches 2 441.1 MB,
0.16% lower. One-shot over streaming is **19.4x** at six channels and **21.9x**
at two.

**The wall at five minutes, and what `parallel` is worth there.** From the same
children's own `encode_ms`, minimum of the samples. The wall columns are the one
pass that carried that field (3 rounds per arm, load 12.22-13.40); the CPU
column is merged over all 6 rounds of the two feature-pairing passes:

| 300 s, one-shot | scalar | parallel | parallel/scalar wall | parallel/scalar CPU |
| --- | ---: | ---: | ---: | ---: |
| 6 ch/44100 | 13.85 s (21.7x real time) | **10.63 s (28.2x)** | **0.77x** | 1.33x |
| 2 ch/48000 | 5.08 s (59.0x real time) | **4.66 s (64.4x)** | **0.92x** | 1.22x |

`duration-curves.md` measured the same comparison on the pre-pool tree and found
at 300 s that `parallel` returned **1.19x wall for 2.32x the CPU** at 6 ch and
**1.00x for 1.94x** at 2 ch. Today it returns **1.30x for 1.33x** and **1.09x
for 1.22x**. The wall return went up and the CPU premium came down by about 40%
at both geometries — which is the pool-sizing change doing at five minutes
exactly what it claimed at ten seconds. The record's wall ratio also holds: at
six channels the feature still pays at 300 s, and at two channels it still
barely does.

CPU per audio-second at 300 s is now **62.2 ms** at 6 ch (parallel) and 46.7 ms
(scalar), **20.8 ms** at 2 ch (parallel) and 17.1 ms (scalar). The sibling
record's 117.7-120.2 ms at 6 ch is therefore superseded, and the pool-sizing
change landed between the two records is the named cause; the difference is
**not decomposed here**, because decomposing it would mean rebuilding a
configuration the tree no longer has.

### The stage split, re-taken — and the configuration it is actually measuring

Re-run at the same time as everything above, fixture, 9 runs per configuration,
**load 20.35 / 19.81 / 22.20** at the three series boundaries:

| fixture, 9 runs | this pass, scalar build | this pass, `--features parallel` | this page records |
| --- | ---: | ---: | ---: |
| `pcm_decode_ms` | 0.575 | 0.599 | 0.59 |
| `select_modes_ms` | 12.046 | 12.421 | 12.14 |
| `plan_and_windows_ms` | 16.043 | 16.655 | 17.04 |
| `analyze_short_ms` | 10.939 | 10.786 | 11.21 |
| **`analyze_long_ms`** | **62.956** | **23.678** | 43.05 |
| `pack_ms` | 41.849 | 42.425 | 43.36 |
| `container_ms` | 0.049 | 0.050 | 0.05 |
| `staged_total_ms` | 132.596 | 93.868 | 115.00 |
| **`encode_pcm_ms`** | **129.562** | **94.331** | 114.91 |
| staged replay == `encode_pcm` | yes | yes | yes |

The replay assertion held in every repetition of both configurations, so each
column describes the shipped pipeline.

**The finding: the split `scripts/measure_encode_perf.py` prints is the scalar
configuration's, and the CLI beside it is the parallel one.** The script runs its
harness as `cargo test --release -p wem-core --test stage_timings -- --ignored
--exact stage_timings_report`, with **no `--features parallel`** — while the
binary it measures in the same invocation requires that feature. When this page's
split was recorded, `parallel` was on by default, so the harness was parallel and
the two halves of the report described one build. It is opt-in now (this page's
own note at the top says so), and the harness silently became the scalar one.
The two configurations differ by **2.66x on `analyze_long_ms`** and **1.37x on
`encode_pcm_ms`**, so this is not a rounding difference.

**Read the recorded 43.05 ms against either of today's columns and you get a
false result**: against the 62.96 ms the script prints you would report a 46%
regression in long-frame analysis that does not exist, and against today's
parallel 23.68 ms a 45% improvement, which is real but belongs to the
pool-sizing change rather than to the window. The recorded figure was the
parallel configuration on the pre-pool tree and is comparable only with the
`--features parallel` column here. **This is an instrument gap, reported rather
than edited**: `scripts/` is not this lane's to change, and closing it is a
one-flag change to a command line the script owns.

What the split does say on its own, both configurations agreeing: `pack_ms` is
unchanged at 41.8-42.4 ms against the recorded 43.36, and it is now the largest
single stage in the parallel configuration. The attribution block reproduced
too — `fft_log_curve_ms` **25.44 (scalar) / 25.57 (parallel)** against the
recorded 37.49, which is this page's finding 1 (closed by 915050b) showing up as
an 11.9 ms saving; `fft_twiddle_recurrence_cost_model_ms` unchanged at 19.11 /
19.31, as it must be, since it replays the recurrence standalone;
`window_rebuild_calls` **1230** against the recorded 2460, this page's finding 4
having removed the redundant rebuild; `window_rebuild_ms` 1.10 → 0.50;
`mdct_ms` 4.60 → 4.46 / 4.53; `transient_ms` 3.38 → 3.18 / 3.26. The call counts
that must not move did not: `fft_calls` 1230, `mdct_calls` 1230,
`twiddle_steps` 9 123 840, `transient_quanta` 2289.

### What this pass supersedes, what it leaves standing, and one anomaly

**Superseded, with the reading that replaces it:**

| this page / sibling | recorded | superseded by | why the earlier reading was provisional |
| --- | ---: | ---: | --- |
| fixture CLI `encode` median | 122.38 ms | **99.41 ms** (CLI, 20 paired rounds) | no load average recorded; other lanes loading the machine |
| fixture CLI x real time | 25.8x | **31.7x** | same |
| scalar vs parallel, fixture | 156.93 / 123.49 ms, 1.26x | **133.84 / 99.39 ms, 1.35x** | same, and the pool-sizing change landed after |
| 6 ch/300 s one-shot peak | 2 445.0 MB | **2 444.8 MB** (median of 10) | its own agreeing samples were 3 of 4; one outlier |
| 6 ch/300 s streaming peak | 123.6 MB | **125.9 MB** (median of 10, min 124.6) | a minimum of a point with a 2.5% spread |
| 6 ch/300 s CPU per audio-second | 117.7-120.2 ms | **62.2 ms** (parallel) / 46.7 ms (scalar) | the pool-sizing change landed after that record |
| stage split, `analyze_long_ms` | 43.05 ms | **23.68 ms** parallel (scalar is 62.96) | measured while `parallel` was a default; see above |

**Left standing, and why:**

- **The five-minute one-shot peak's order of magnitude, and the streaming
  path's bound.** Both reproduce; the streaming ring is still bounded and the
  one-shot peak is still output-proportional. Nothing here changes the shape
  findings of [`duration-curves.md`](duration-curves.md).
- **This page's findings 1-6.** No instrument here re-tests them, and the two
  the split touches (1, the twiddle recurrence; 4, the redundant rebuilds) are
  confirmed by their own call counts and stage times moving the way their
  closures predicted.
- **[`pool-sizing.md`](pool-sizing.md)'s before/after arms.** Its claim is the
  CPU and `sys` columns, and its before arm is a pristine older tree with a
  process-global pool that no longer exists in this one. Re-taking it would mean
  rebuilding a deleted configuration to re-answer a question it already
  answered; its wall columns stay provisional, as it says of itself.
- **This page's scaling-in-stream-length table.** Re-taking it needs a 40x
  payload corpus that is not this page's instrument and that no lane has
  rebuilt; the claim it carries (no quadratic term) is not the pair the ledger
  was missing, and the measurement would cost more than the number is worth.

**One anomaly, re-taken and not resolved away.** `duration-curves.md` reports
that the 6 ch/300 s one-shot point is bimodal: three samples at 2 443.7-2 445.9
MB and one at 2 021.8 MB, 17% below, never reproduced. **This pass reproduced
the anomaly's kind, at a smaller magnitude, and detached it from the feature
configuration.** Across sixteen fresh-child samples of that point:

- nine of the ten parallel samples lie in **2 444.0-2 446.0 MB**, a 0.08% band;
  the tenth is **2 234.1 MB**, 8.6% below;
- five of the six scalar samples lie in **2 439.9-2 442.0 MB**, a 0.09% band;
  the sixth is **2 339.2 MB**, 4.2% below.

So the modal value is stable to a tenth of a percent, an occasional sample
under-measures by 4-9%, and it happens on **both** configurations — the earlier
record saw its low sample on the then-default parallel build, this pass saw one
on each. The 17% excursion did not recur. That is consistent with the earlier
record's own reasoning that a one-shot peak below the `detector` phase worker's
2 336.7 MB cannot be a lower memory requirement, and it means the point should
be quoted as the mode with the anomaly stated beside it — which is what both
records now do. **It is not a contradiction of any number; it is the reason a
single sample of this point should never be quoted alone.**

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
- **The quiet-window pass is one window, not an idle machine.** Its load is
  stated on every table it contributes (1-minute 11.51-26.22, median 16.38
  across 15 minutes, against a measured resting floor of 12-15 on 16 cores).
  Its wall columns are load-dependent and are the least portable part of it; its
  RSS and CPU columns are not, and the five-minute peak's 0.08% agreement across
  a 5.4-wide load swing is the evidence for that rather than an assumption.
- **The paired series measures the kernel, and the CLI check licenses that.**
  The scalar configuration has no CLI, so both arms of the feature comparison
  are `concurrency_worker` built twice. The one place the released CLI is
  measured against the worker (fixture, 20 paired rounds) it agrees to 0.4% on
  the median and 0.2% on CPU, which is what makes the worker a stand-in for the
  shipped surface rather than a substitute for it.
- **The five-minute memory numbers come from the same `wait4` reporting as
  [`duration-curves.md`](duration-curves.md)**, driving that record's own
  instrument and its own generator, and are read the same way: `ru_maxrss` of a
  fresh child, one child per point. The 6 ch one-shot point's bimodality is
  carried forward rather than smoothed: the median quoted is the mode, and the
  under-measuring samples are listed beside it.
- **The quiet-window pass changed no instrument.** The driver that arranges the
  pairs and alternates their order is scratch, outside the tracked tree, and
  both `scripts/` instruments it drives are the committed ones, run unmodified.
  The configuration gap it found in `scripts/measure_encode_perf.py`'s harness
  invocation is reported here and **not fixed**, because `scripts/` was outside
  this pass's fence.
- **The attribution block remains a cost model**, and its `twiddle_steps` figure
  is the same replayed recurrence it always was; its invariance across the two
  configurations is a check that it is replayed the same way, not a measurement
  of removed work.