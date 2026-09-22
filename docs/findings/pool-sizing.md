# Sizing the long-frame channel pool to the work

Date: 2026-09-22. This page is the evidence record for one change: the
long-frame region's two channel-parallel waves run in a worker pool the analysis
session owns and sizes to the encode's channel count, instead of in rayon's
process-global pool. It implements the observation recorded in
[`duration-curves.md`](duration-curves.md) — the wall-time optimum sits at the
channel count, and the extra CPU past it is nearly all system time — and states
what the change cost, what it bought, the load-insensitivity argument the
conclusion rests on, the falsification that separates the two, and the boundary of
what it proves. The norms it serves are in
[`../reference/standards.md`](../reference/standards.md) (Bit-exactness, Caller
streams and ambient state, Portability floor); the neighbouring axis — how many
encodes one machine should run at once — is
[`concurrency-curves.md`](concurrency-curves.md), and the code is
`crates/wem-analysis/src/psychoacoustics/pool.rs`.

**Result.** The size of the pool is a property of the encode, not of the host:
one worker per channel, because one worker per channel is how many jobs each wave
has. Against the process-global pool the same machine would otherwise hand it
(16 workers on a 16-core host, against 6 or 2 jobs), that is

- **6ch/44100, 10 s, CPU per encode 1101.9 → 762.9 ms (−30.8%)**, system time
  338.1 → 57.8 ms (−82.9%), wall 611.9 → 587.6 ms (−4.0%, provisional);
- **2ch/48000, 10 s, CPU per encode 394.9 → 272.2 ms (−31.1%)**, system time
  96.9 → 14.9 ms (−84.6%), wall 257.0 → 242.0 ms (−5.8%, provisional).

The pool's worker count is the *cost*, not its ownership: a private pool of 16
workers costs what the global pool of 16 cost (CPU 1116.5 vs 1095.7 ms). What the
change buys is that the count is the encode's geometry — a number the library can
derive — instead of a number only the host's environment knows. Nothing global is
reconfigured and no environment variable is read, and the encode's bytes are
identical at every pool size from one worker up.

The cost is that each session now owns its workers rather than sharing one pool
with every other session in the process, so that case was measured too: several
encodes at once in one process cost **814.6 ms of CPU per encode at N = 1
(against 1126.9), 860.1 at N = 4 (against 936.8) and 819.6 at N = 8 (against
818.0)** — better at one and four, a wash at eight, where the cores are the limit
either way.

Those before-arms and [`duration-curves.md`](duration-curves.md)'s are the same
comparison on two machine states, and they differ in the arm this change removes:
that record measured the 6ch before arm at 1301 ms of CPU, 585 ms of it system,
against 710 ms at the channel count, where this record's before arm burned
1101.9 ms, 338.1 ms of it system, against 762.9 ms. The system time of the
16-worker arm is what moved between the two records — it is the quantity the
change is about — and both land on the channel count. The 2ch geometry shows the
same shape in both records: it never had much to gain from a pool larger than its
channel count (that record 316 → 229 ms of CPU, this one 394.9 → 272.2).

## What was measured, and how

Every table below is a parent process measuring a child. Reproduce the shape of it
with:

```sh
cd crates
cargo build --release -p wem-core --bin wwise-wem          # the change
# and, for the before arm, the same binary from a pristine tree unpacked beside it:
#   git archive main | tar -x -C "$SCRATCH/wem-base"
#   (cd "$SCRATCH/wem-base/crates" && cargo build --release -p wem-core --bin wwise-wem)
```

The many-sessions table additionally needs the extension built in each tree
(`cargo build --release -p wem-python --features extension-module`, its cdylib
copied to `src/wwise_wem/_core.abi3.so`, which is gitignored) and a child that
starts N threads which each call `wwise_wem.encode(<the 6ch input>)` under one
`PYTHONPATH` per tree; the parent times that child the same way.

Each number is a `RUSAGE_CHILDREN` delta taken by a parent process around one
`subprocess.run` of a real `wwise-wem` encode — the child-process pattern
[`concurrency-curves.md`](concurrency-curves.md) and the encode-performance lane
used. Nothing is timed in-process. Rounds are **paired and order-alternated**: in
each repetition every arm runs once, and the order reverses on odd repetitions, so
a slow window hits the arms evenly instead of the arm that happens to run last.
The columns are the median and the minimum over the repetitions; the minimum is
the least-contended sample and the median the typical one. The load is recorded
per run with `os.getloadavg()` and stated per table, because **on this machine
the load decides the wall time**: the 10 s rows below run between load 19 and 80
against 16 cores, and an arm's wall moves by tens of percent between records while
its CPU moves by less and in the direction the load predicts (the 2ch
before-default arm: wall 257.0 → 278.7 ms and CPU 394.9 → 460.1 ms between a
load-25 and a load-76 record). The paired, order-alternated structure is what
keeps a *comparison* valid under that drift, since both arms of a round meet the
same machine moments apart.

So: **the conclusion is the CPU and `sys` columns; the wall columns are
provisional**, reported because a reader will look for them.

The corpus is two deterministic 10 s PCM16 WAVs and one 1 s one, generated by a
lane-local script (noise bed plus a per-channel tone under a 40 ms burst envelope,
so both short and long frames occur). They are not committed; their digests fix
them:

| Input | Geometry | Frames | SHA-256 |
| --- | --- | ---: | --- |
| `six_10s.wav` | 6ch/44100 | 441000 | `c2ff0baa0a7422bf3b7c33fb4cc3f2746313fbabdd57a07d916255763d8c8b1b` |
| `two_10s.wav` | 2ch/48000 | 480000 | `f4deb2a79398f2f004968edc550945061021f35d40e30cf042822e05eefd3681` |
| `six_short.wav` | 6ch/44100 | 44100 | `5a577a9d136368870e9913dc463e0b8543a0a059147e9826f55d161694a6d6fb` |

The fixture rows use the committed `tests/fixtures/input.wav` (SHA-256
`6e43ae2563a6ac09bbacb493023cffc88772289c7c30c339c1ab32422e66f530`, a digest that
identifies a corpus rather than standing in for a comparison: every equality
below is a byte comparison against the bytes it is about).

**Base for every table below.** The before arms are a pristine `main` at the
landing before this change — the tree named by its newest `crates/` change,
*perf(analysis): build the long seed look once per encode, not per channel job*.
The after arms are that tree plus this lane's change, which was uncommitted while
it was measured. `main` has taken two passes since: documentation, `scripts/` and
language-shell changes, and a refactor that deletes unused analysis surface and
drops the container digest from the encode result — a sub-millisecond per-encode
cost at these container sizes, paid by every arm of every paired comparison
below, and below the resolution of the fixed-cost table's instrument. The merged
tree was re-measured for bytes (see [Byte exactness](#byte-exactness)), and this
lane's hunks were re-applied to the moved base by hand.

## The shape

`AnalysisSession` owns one `ChannelPool`, built in `AnalysisSession::new` — once
per encode — with `rayon::ThreadPoolBuilder::new().num_threads(channels)`, and
the two waves of `analyze_long_frame` run inside it. `channels` is the number of
jobs each wave spawns, so the pool never holds more workers than the wave has
jobs; the measured wall plateau's bottom sits at that same number.

The size is *derived*: there is no pool to submit to, no queue and no dispatch
policy, and a host refusal surfaces as `AnalysisError::PoolUnavailable` from the
session constructor rather than a panic. What a caller can do is *read* the size
the geometry produced — `AnalysisSession::channel_pool_workers()`, the same kind
of fact as the frame count on an encode result — and it tracks the geometry one
for one:

| Session geometry | `channel_pool_workers()` | Threads the process holds |
| --- | ---: | ---: |
| 6ch/44100 (fixture and `six_10s.wav`) | 6 | 7 (main + the pool) |
| 2ch/48000 (`two_10s.wav`) | 2 | 3 (main + the pool) |

Two things are deliberately *not* the input to the size:

- **`available_parallelism`.** It is itself ambient — the process's affinity and
  cgroup limits — and it would make the same encode build a different pool on
  every machine. A host with fewer cores than channels time-slices the runnable
  workers, exactly as it time-sliced the same jobs when they ran in the global
  pool.
- **`RAYON_NUM_THREADS`,** or any other environment variable, for the same
  reason.

| Build | input | `RAYON_NUM_THREADS` = unset | 1 | 2 | 6 | 16 |
| --- | --- | ---: | ---: | ---: | ---: | ---: |
| before (global pool) | 6ch | 17 | 2 | 3 | 7 | 17 |
| after (session pool) | 6ch | 7 | 7 | 7 | 7 | 7 |
| after (session pool) | 2ch | 3 | 3 | 3 | 3 | 3 |

Process thread high-water mark during one real 10 s CLI encode (`ps -M`): the
before build's full row is from the base named above, and the after rows are from
the merged tree re-measured after the last merge. Before, the environment sizes
the pool; after, it is main plus the session's workers — six for a 6ch encode,
two for a 2ch one — at every setting, and the global pool is never initialized at
all, since nothing reaches for it.

## The matrix

10 s encodes, 20 paired order-alternated repetitions per arm, per encode in ms.
Two oracles are carried alongside the change: the *old* shape with the global pool
pinned to the channel count through `RAYON_NUM_THREADS` (what the new shape should
be worth, if the only thing that mattered were the count), and the *new* shape
with that same variable pinned to 1 (what must not change, since nothing reads
it).

| 6ch/44100, load 18.8–28.0 (median 25.3) | wall med | wall min | CPU med | CPU min | user med | sys med |
| --- | ---: | ---: | ---: | ---: | ---: | ---: |
| before — global pool, host default (16) | 611.9 | 604.2 | 1101.9 | 1040.8 | 761.2 | 338.1 |
| after — session pool (6) | 587.6 | 583.3 | **762.9** | **755.0** | 704.3 | **57.8** |
| before — global pinned to 6 (oracle) | 592.0 | 582.5 | 767.4 | 753.7 | 708.2 | 58.2 |
| after — `RAYON_NUM_THREADS=1` | 588.2 | 582.0 | 762.6 | 754.2 | 704.4 | 57.8 |

| 2ch/48000, load 23.3–25.8 | wall med | wall min | CPU med | CPU min | user med | sys med |
| --- | ---: | ---: | ---: | ---: | ---: | ---: |
| before — global pool, host default (16) | 257.0 | 252.0 | 394.9 | 382.0 | 299.1 | 96.9 |
| after — session pool (2) | 242.0 | 235.0 | **272.2** | **264.3** | 256.7 | **14.9** |
| before — global pinned to 2 (oracle) | 239.9 | 237.0 | 269.2 | 266.7 | 255.6 | 13.9 |
| after — `RAYON_NUM_THREADS=1` | 236.9 | 233.7 | 266.3 | 263.7 | 252.3 | 14.0 |

Against the shipped default the change is −30.8% CPU at 6ch and −31.1% at 2ch;
against the channel-count oracle it is −0.6% and +1.1%, i.e. the same number, on
a machine whose global pool had 16 workers for 6 jobs and 16 for 2. The saving
grows with contention: at 2ch under the load of the sweep below (load 72–80) the
before-default arm burned 460.1 ms of CPU against the after arm's 278.2 ms, or
−39.5%.

## The sweep, and where the optimum sits

15 paired repetitions per arm, medians, per encode in ms. The before rows sweep
the process-global pool through `RAYON_NUM_THREADS`. The "shape at n workers" rows
below are *history*: they were taken with a lane-time instrument that could name a
worker count, which the shipped library deliberately does not offer — a session
builds one size, its channel count, and reports it through
`channel_pool_workers()`. The rows are kept because they are what chose the size
and because they show the shape is not size-sensitive; the property they establish
(a wave completes, in channel order, byte-identically, at any worker count
including one) is pinned in tree by this module's unit tests rather than by a
reachable knob.

| 6ch/44100, before — global pool (load 19.6–80.1) | 1 | 2 | 3 | 4 | 6 | 8 | 16 | default (16) |
| --- | ---: | ---: | ---: | ---: | ---: | ---: | ---: | ---: |
| wall | 680.3 | 627.9 | 604.1 | 606.0 | **598.9** | 605.5 | 620.6 | 618.0 |
| CPU | **676.3** | 699.7 | 710.5 | 738.1 | 779.0 | 835.3 | 1095.7 | 1095.6 |
| `sys` | 14.0 | 21.9 | 31.0 | 42.3 | 68.9 | 112.8 | 337.0 | 339.0 |

| 2ch/48000, before — global pool (load 71.9–79.7) | 1 | 2 | 3 | 4 | 6 | 8 | 16 | default (16) |
| --- | ---: | ---: | ---: | ---: | ---: | ---: | ---: | ---: |
| wall | 268.7 | 252.7 | **251.0** | 258.4 | 257.4 | 265.9 | 266.9 | 278.7 |
| CPU | **268.9** | 287.8 | 296.2 | 314.7 | 348.0 | 382.8 | 445.7 | 460.1 |
| `sys` | 13.2 | 22.2 | 31.2 | 44.4 | 69.1 | 96.1 | 144.0 | 154.2 |

| 6ch/44100, the shape at n workers (load 43.1–68.1) | 1 | 2 | 3 | 4 | 6 | 8 | 16 | 6 (shipped) |
| --- | ---: | ---: | ---: | ---: | ---: | ---: | ---: | ---: |
| wall | 676.5 | 621.9 | 610.4 | 608.9 | 603.6 | 602.9 | 622.3 | **598.4** |
| CPU | **676.8** | 699.2 | 717.4 | 746.7 | 799.2 | 844.2 | 1116.5 | 788.5 |
| `sys` | 17.8 | 26.8 | 38.5 | 54.4 | 86.7 | 123.4 | 360.7 | 81.4 |

| 2ch/48000, the shape at n workers (load 31.0–40.1) | 1 | 2 | 3 | 4 | 6 | 8 | 16 | 2 (shipped) |
| --- | ---: | ---: | ---: | ---: | ---: | ---: | ---: | ---: |
| wall | 267.3 | **248.9** | 250.7 | 250.8 | 256.0 | 258.8 | 266.8 | 248.7 |
| CPU | **266.5** | 278.8 | 291.4 | 305.9 | 342.0 | 381.9 | 455.3 | 278.2 |
| `sys` | 11.8 | 18.2 | 26.4 | 38.8 | 64.2 | 96.4 | 152.5 | 17.4 |

Two things are visible in every one of those tables. **CPU and `sys` are monotone
in the worker count** — each additional worker past the job count is pure
scheduling cost (6ch: +15% CPU at 6 workers over one, +62% at 16; `sys` ×24). And
**wall bottoms out at the channel count and then stops improving or worsens**:
6ch reaches 598.9–603.6 ms at 6 workers and is *worse* at 16 (620.6 before,
622.3 after), 2ch reaches 248.7–252.7 ms at 2 and is worse at 16 (266.8–266.9).
Six jobs of ~60 µs each do not get faster with sixteen workers; at one worker the
wall is 13.6% (6ch) and 6.3% (2ch) worse for 13.2% and 6.6% less CPU. Wall is the
reason the size is the job count and not one.

## The falsification

The change reduces the worker count; it does not make workers cheaper. Three
observations separate those:

1. **The shape at 16 workers reproduces the old cost.** 6ch CPU 1116.5 ms against
   the before-build's 1095.7 ms at the host default — within 2%, and both about
   40% above the channel-count row. If the win had come from owning the pool
   rather than from sizing it, the 16-worker private pool would have been cheap.
2. **Matched sizes agree.** Before-build (`RAYON_NUM_THREADS = n`) against
   after-build (shape at n workers) for 6ch CPU: 676.3/676.8 (n=1), 699.7/699.2
   (n=2), 710.5/717.4 (n=3), 738.1/746.7 (n=4), 779.0/799.2 (n=6),
   835.3/844.2 (n=8), 1095.7/1116.5 (n=16) — the same curve within 3% at every
   size, on different loads. The change moved the size and nothing else.
3. **The variable that used to decide the size decides nothing.** The after-build
   at `RAYON_NUM_THREADS=1` has the same CPU as the after-build with it unset
   (762.6 vs 762.9 ms) and the same thread count (7), and produces the same bytes.

The before-build confirms the counterfactual the other way: CPU rises
monotonically with the environment's pool size (676.3 → 1095.7 ms from 1 to 16
workers at 6ch), which is what the removed −45% figure was measuring.

## Fixed cost

Building and tearing down a pool per encode is new work, so it was measured where
it would show most: a 1 s encode, 30 paired repetitions, load 28.4–28.6.

| 6ch/44100, 1 s | wall med | wall min | CPU med | CPU min | sys med |
| --- | ---: | ---: | ---: | ---: | ---: |
| before — global pool, host default (16) | 74.6 | 73.0 | 120.8 | 117.7 | 35.6 |
| before — global pinned to 6 | 72.2 | 70.4 | 87.6 | 85.6 | 8.1 |
| before — global pinned to 1 | 79.5 | 77.5 | 77.2 | 75.6 | 3.3 |
| after — session pool (6) | **71.5** | **70.3** | 87.5 | 85.9 | 8.3 |

The after arm is the same as the before arm at the same worker count to within
0.1% of CPU (−0.1 ms) and 0.7 ms of wall: the per-encode pool build is under this
instrument's resolution, on an encode a tenth as long as the corpus the other
tables use.

## Byte exactness

The pool is scheduling; it must not be able to change a byte, at any size,
including fewer workers than channels and exactly one. Every equality below is a
byte comparison of the containers themselves — `cmp` against the bytes it is
about, or the suites' own word-for-word comparisons — not a digest of two whole
files.

- **The shape at every worker count.** 6ch/10 s and 2ch/10 s encoded at 1, 2, 3,
  4, 6, 8 and 16 workers, and at the shipped size, are byte-identical to one
  another and to the base build's output for the same input; the fixture at 1, 2,
  3, 6, 8, 16 and the shipped size is byte-identical to the committed
  `tests/fixtures/reference.wem`. Every one of those runs terminated, including
  one worker and fewer workers than channels — rayon's scope latch is a stealing
  latch, so the worker that owns a wave runs its remaining jobs itself while it
  waits (`rayon_core::latch::CountLatch::wait`). In tree, the order property is
  pinned by `crates/wem-analysis/src/psychoacoustics/pool.rs`'s
  `map_channels_completes_in_channel_order_at_every_pool_size`, which sweeps
  1, 2, 3, 6, 8 and 16 workers with the feature on, and
  `map_channels_is_sequential_without_the_feature` pins the same order on the
  scalar path; `for_channels_sizes_the_pool_to_the_job_count` pins the size to the
  channel count.
- **`RAYON_NUM_THREADS` 1, 2, 3, 6, 8, 16 and unset**, on the fixture, against
  the committed `tests/fixtures/reference.wem`: identical on the before build and
  on the after build (14 of 14). The after build's bytes do not move because it
  never reads the variable; the before build's do not move because the partition
  and the collection order are size-independent — the property the change had to
  keep, and did.
- **The same inputs through a second implementation** of the one-shot path,
  written against the shipped public surface rather than the CLI: byte-identical
  output for the fixture and for both corpus geometries.
- **The suite.** `cargo test -p wem-core --test complete_wem_bytes` and
  `--test frame_pipeline_parity` (the per-frame, per-channel, per-bin comparison
  against the live Python oracle over all 205 fixture frames) pass under both
  feature configurations, as do `bytes_parity`, `error_variants` and
  `cargo test -p wem-analysis --all-targets` both ways; `cargo clippy
  --workspace -- -D warnings`, `cargo fmt --all --check` and `cargo doc
  --workspace --no-deps` are clean (the doc build emits no warnings, including
  the new module's links), and `cargo check -p wem-wasm --target
  wasm32-unknown-unknown` confirms the scalar configuration still builds for a
  threadless target, where the pool type exists but has no threads at all. One
  target set is red for a reason outside this change: `--all-targets` cannot
  compile `crates/wem-core/tests/stage_timings.rs`, which imports
  `wem_analysis::dsp::transform::vorbis_window`, deleted by the refactor `main`
  took after the base named above; it fails the same way on a pristine `main`,
  and that path belongs to the lane that owns it.

## Many sessions in one process

The pool is per session, so N concurrently live sessions hold N × channels
workers where the global pool they shared held one host-sized pool. That is the
one way this change could cost something, so it was measured rather than argued:
N concurrent encodes inside one process through the installed extension (which
releases the GIL for the kernel call), 6 paired repetitions per arm, per encode
in ms. Each encode builds its own `Encoder` and its own session; every child
verifies that its N returned containers are the same bytes, and each does.

| 6ch/44100, 10 s, load 237–244 | wall med | wall min | CPU med | CPU min | `user` med | `sys` med |
| --- | ---: | ---: | ---: | ---: | ---: | ---: |
| before, N = 1 | 660.7 | 648.7 | 1126.9 | 1105.4 | 780.2 | 346.2 |
| after, N = 1 | **635.7** | **630.4** | **814.6** | **809.3** | 732.3 | **83.7** |
| before, N = 4 | 752.0 | 739.8 | 3747.2 | 3641.6 | 3132.9 | 632.9 |
| after, N = 4 | **744.4** | **711.5** | **3440.4** | **3384.9** | 3039.1 | **403.0** |
| before, N = 8 | 876.3 | 846.2 | 6543.9 | 6314.3 | 6159.2 | **376.0** |
| after, N = 8 | 937.0 | 866.8 | 6556.9 | 6421.0 | 6127.2 | 433.3 |

Per encode, that is CPU 1126.9 → 814.6 ms at N = 1, 936.8 → 860.1 ms at N = 4 and
818.0 → 819.6 ms at N = 8: better at one and four concurrent encodes, and equal
within 0.2% at eight — where 8 × 6 workers and 8 × 2 are already far past the 16
cores and the cores, not the pool, are the limit. The wall column is the noisiest
here (load ~240) and is only worse at N = 8, by 6.9% on the median and 2.4% on the
minimum, against a 0.2% CPU difference; at N = 1 and N = 4 the wall improves too.
No measurement here shows the per-session pool costing throughput.

This does not contradict [`concurrency-curves.md`](concurrency-curves.md): its
recommendation — one encode per core with the `parallel` feature off once encodes
compete for the same cores — is about the feature as a whole, and the feature-off
build has no pool of either kind. What this table adds is that a host which keeps
the feature on and runs several encodes in one process does not pay for the pool
being per session in the range measured.

## What this does not measure

- **Hosts with fewer cores than channels.** This machine has 16 cores for 6 and 2
  channels, so the size choice is never constrained here; the argument for the
  channel count over `available_parallelism` is argued in
  [The shape](#the-shape) rather than measured, because on this machine the two
  coincide.
- **More than eight concurrent sessions**, or a host where the session count and
  the channel count together exceed the cores by more than the table above: the
  per-session pool's thread total grows with the session count by construction.
- **The wall columns as a claim.** They move with the load (19–80 for the paired
  tables, ~240 for the concurrency table, against 16 cores) by more than the
  change is worth. The claim is the CPU and `sys` columns.