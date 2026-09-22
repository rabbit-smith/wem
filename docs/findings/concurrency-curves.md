# Concurrency: what N simultaneous encodes do to this machine

**Result.** Every performance number this tree had was a single encode. This is
the missing axis, and it is the one a server plans with. On this machine
(Apple M3 Max, 16 logical cores), with the 1-minute load average stated because
on this machine it decides the answer:

- **The kernel's internal `parallel` feature stops paying at a concurrency that
  depends on how loaded the machine is.** Paired against the sequential
  configuration at the same N inside the same repetition, its throughput ratio
  on the 6-channel fixture is 1.18× at N = 1, and it falls below 1 **between
  N = 4 and N = 6 when the 6-channel cells ran at load ~178** (1.06× at N = 4,
  0.91× at N = 6) but **already between N = 3 and N = 4 when they ran at load
  ~32** (1.02× at N = 3, 0.94× at N = 4). The machine was the same; only the
  neighbours changed. **For stereo it never pays at any N or any load** — 0.96×
  at N = 1 and 0.94× at N = 1 in the two records.
- **The sequential configuration turns cores into throughput at ~1.00
  efficiency** — speedup divided by the cores it actually used is 1.02, 1.02,
  1.02, 1.01, 1.01, 0.99 through N = 8 on the fixture. With the feature on, the
  same ratio is 0.33 at N = 1 rising only to 0.62 at N = 16. That is the
  scaling result, and it is the load-insensitive one: the 6-channel sequential
  rates agree within 5% between the two records, where the 6-channel parallel
  rates move by up to 17% (specifically between N = 4 and N = 8, the range where
  the feature is competing with itself for the same cores).
- **More concurrency with the feature on is worse than less concurrency without
  it**, on throughput, latency *and* CPU per encode, once the crossing is
  passed. The honest deployment shape is **one encode per core with the feature
  off**; the internal parallelism is worth having only when a single encode
  cannot be split across requests, which on the fixture means **N ≤ 3**.
- **CPU per encode never favours the feature.** At N = 1 on the fixture, in the
  loaded record, it burns **2.61× the CPU to deliver 1.18× the throughput**, and
  at N = 16 it is still 1.19× the CPU for 0.77× the throughput.

Everything here is **provisional**: two full records were taken, at 6-channel
cell loads of ~178 and ~32 against 16 cores, and neither window was an idle
machine. The load-insensitive quantities are the paired ratios, CPU per encode
and cores busy; the absolute rates are not. [What a quiet machine would
change](#what-a-quiet-machine-would-change) reports what halving the load
actually did and what is still extrapolation.

## What was measured, and how

Reproduce with:

```sh
python3 scripts/measure_concurrency.py                    # builds both arms, writes the JSON
./.venv/bin/python scripts/plot_concurrency_curves.py     # re-renders the figure
```

**Later change — the feature is opt-in, and the CLI is not the instrument.**
This page was measured on a tree whose `parallel` feature was on by default and
whose two arms were the `wwise-wem` CLI built with and without it — the two
commands that stood in the block above, `cargo build --release -p wem-core` and
`cargo build --release -p wem-core --no-default-features --target-dir
target/no-parallel`. Today a default build is scalar, the CLI *requires* the
feature (so it has no scalar build to pair against), and the matrix is measured
from its own instrument on both sides:
`crates/wem-core/tests/concurrency_worker.rs`, one test target built in the two
configurations, which `scripts/measure_concurrency.py` builds itself — the block
above is that recipe, and it needs no binary built by hand. A caller can also
bound the pool at construction
(`EncoderOptions::max_channel_pool_workers`, with
`StreamSession::channel_pool_workers()` as the reading), so "the feature on"
below is the geometry-sized configuration a caller who asks for it gets; the
norm and the deviation from the ecosystem's environment-variable lever are in
[`../reference/standards.md`](../reference/standards.md#portability-floor). The
finding itself stands: internal parallelism pays at low concurrency, stops paying
at a load-dependent N, and never pays for stereo.

`scripts/measure_concurrency.py` starts N child processes as close together as
the spawn syscall allows and reaps each with `os.wait4`, so every child yields
its own exit instant, its own `ru_utime`/`ru_stime` and its own `ru_maxrss` —
the same child-process pattern the encode-performance lane used. `wait4(-1)` is
what makes the latency column measurable: it returns as soon as *any* child
exits and names which one, so a child's exit instant is an observation rather
than a position in a sequential reap order.

The matrix is **N ∈ {1, 2, 3, 4, 6, 8, 16} × {`parallel` on, off} × both
installed geometries**, 20 repetitions per cell, N ascending. 2 is in the set
for the stereo geometry and 6 for the six-channel one, because those are the
channel counts the internal partition actually splits across.

| Geometry | Input | Channels | Rate | Audio | Encodes per record |
| --- | --- | ---: | ---: | ---: | ---: |
| 6ch/44100 | `tests/fixtures/input.wav` | 6 | 44100 Hz | 3.161 s | 1600 |
| 2ch/48000 | `tests/data/2ch-reference/stereo_noise.wav` | 2 | 48000 Hz | 1.000 s | 1600 |

It was run **five times**. Two of them are committed as records, because the
difference between them is itself a result:

| Record | File | Load 1m, 6-channel cells | Load 1m, stereo cells | Role |
| --- | --- | --- | --- | --- |
| Loaded | [`figures/concurrency-samples.json`](../figures/concurrency-samples.json) | 145–231 (median 178) | 135–150 | The tables and the figure below |
| Quiet | [`figures/concurrency-samples-quiet-window.json`](../figures/concurrency-samples-quiet-window.json) | 25–60 (median 32) | 60–113 | The load comparison the crossing conclusion rests on |

The other three are quoted only as summary ratios, under
[Reproduced](#where-scaling-stops-per-configuration).

**The two configurations are paired and order-alternated.** Within one
repetition both run at the same N back to back, and which of the two goes first
alternates with the repetition index. A machine whose load drifts — and this
one does, by a factor of two over the session — moves both sides of the ratio
together, which is why the paired table is the one the conclusions rest on and
the raw rate tables are the context for it.

### Machine identity

| | |
| --- | --- |
| CPU | Apple M3 Max, 16 logical cores |
| Memory | 64 GB |
| OS | macOS 27.0, Darwin 27.0.0, arm64 |
| Toolchain | rustc 1.96.0 (ac68faa20), host aarch64-apple-darwin |
| Build | release; both arms are `crates/wem-core/tests/concurrency_worker.rs`, built with and without `--features parallel` (at the time of these numbers: `parallel` default-on, and `--no-default-features` for the sequential binary) |
| Load average, 1 min, 6-channel cells | **145–231** (loaded record, the tables below) / **25–60** (quiet record) |

Both binaries were checked to emit the same bytes before anything was timed:
the fixture produces SHA-256 `17851d26c6210b85…`, the digest
[`standards.md`](../reference/standards.md#what-is-established) records, from
either build. Nothing in this document is about a different encoder.

### Two derived quantities, and why they carry the conclusions

A load average says what the whole machine was doing and counts threads that
are not on a core. Two quantities computed from the batch's own rusage do not
have that problem:

- **Cores busy** — the batch's summed user + sys divided by its wall clock: the
  average number of cores that were running *this* batch's work. It measures
  the share of the machine the batch actually received instead of assuming it.
  The ceiling observed here was **10.06 cores** (6ch, sequential, N = 16), which
  is the environment's answer to "how much of this machine may I have", not the
  encoder's.
- **Cores-to-throughput efficiency** — speedup against that series' own N = 1,
  divided by cores busy. 1.00 means every core obtained became throughput in
  proportion. This is the number that says whether concurrency is buying
  anything, and it is a ratio of two things the batch measured about itself.

## The tables

All rows are medians over 20 repetitions; latency is over the 20 × N individual
child observations in the cell. `batch CPU` is the whole batch's user + sys as
the parent reaped it, and `CPU/encode` is that divided by N — the efficiency
number. Load is the cell's 1-minute load average (min/median/max, sampled before
and after every batch). **Every rate in these four tables is provisional** — CPU
per encode and cores busy are not. They are the **loaded record**; the quiet
record's comparison is under
[Reproduced](#where-scaling-stops-per-configuration).

### 6ch/44100 — internal parallel on

| N | encodes/s | audio-s/s | speedup vs N=1 | ideal encodes/s | latency med ms | latency p95 ms | encode-stage med ms | batch CPU ms | CPU/encode ms | cores busy | RSS/child MB | resident for N MB | load 1m |
| ---: | ---: | ---: | ---: | ---: | ---: | ---: | ---: | ---: | ---: | ---: | ---: | ---: | ---: | ---: |
| 1 | 8.39 | 26.51 | 1.00× | 8.4 | 118.8 | 146.1 | 112.5 | 367.6 | 367.6 | 3.06 | 39.5 | 40 | 145/179/231 |
| 2 | 16.50 | 52.15 | 1.97× | 16.8 | 120.5 | 149.7 | 114.5 | 664.6 | 332.3 | 5.55 | 43.6 | 87 | 145/178/216 |
| 3 | 22.97 | 72.60 | 2.74× | 25.2 | 128.9 | 204.1 | 122.7 | 851.5 | 283.8 | 6.53 | 39.6 | 119 | 145/178/216 |
| 4 | 29.10 | 91.97 | 3.47× | 33.5 | 135.2 | 163.8 | 129.2 | 1018.9 | 254.7 | 7.38 | 41.3 | 165 | 145/178/216 |
| 6 | 36.83 | 116.41 | 4.39× | 50.3 | 159.5 | 300.2 | 152.8 | 1299.4 | 216.6 | 7.89 | 43.2 | 259 | 145/178/216 |
| 8 | 41.96 | 132.65 | 5.00× | 67.1 | 186.4 | 245.1 | 179.9 | 1613.6 | 201.7 | 8.32 | 41.0 | 328 | 145/178/216 |
| 16 | 47.37 | 149.73 | 5.65× | 134.2 | 325.2 | 465.9 | 313.4 | 3029.3 | 189.3 | 9.07 | 40.1 | 642 | 145/178/216 |

### 6ch/44100 — internal parallel off

| N | encodes/s | audio-s/s | speedup vs N=1 | ideal encodes/s | latency med ms | latency p95 ms | encode-stage med ms | batch CPU ms | CPU/encode ms | cores busy | RSS/child MB | resident for N MB | load 1m |
| ---: | ---: | ---: | ---: | ---: | ---: | ---: | ---: | ---: | ---: | ---: | ---: | ---: | ---: | ---: |
| 1 | 7.03 | 22.21 | 1.00× | 7.0 | 141.7 | 145.4 | 135.6 | 139.7 | 139.7 | 0.98 | 38.1 | 38 | 145/179/231 |
| 2 | 14.05 | 44.40 | 2.00× | 14.1 | 140.7 | 149.1 | 134.7 | 278.3 | 139.1 | 1.95 | 41.6 | 83 | 145/178/216 |
| 3 | 20.83 | 65.85 | 2.96× | 21.1 | 142.1 | 146.9 | 136.0 | 420.4 | 140.1 | 2.91 | 38.1 | 114 | 145/178/216 |
| 4 | 27.51 | 86.96 | 3.91× | 28.1 | 143.5 | 166.7 | 137.4 | 561.6 | 140.4 | 3.87 | 39.7 | 159 | 145/178/216 |
| 6 | 40.94 | 129.40 | 5.83× | 42.2 | 144.3 | 155.7 | 137.7 | 848.5 | 141.4 | 5.76 | 39.2 | 235 | 145/178/216 |
| 8 | 52.31 | 165.34 | 7.44× | 56.2 | 146.7 | 185.0 | 140.2 | 1146.4 | 143.3 | 7.51 | 39.2 | 313 | 145/178/216 |
| 16 | 63.83 | 201.77 | 9.08× | 112.4 | 218.5 | 286.2 | 206.5 | 2537.8 | 158.6 | 10.06 | 38.2 | 612 | 145/178/216 |

### 2ch/48000 — internal parallel on

| N | encodes/s | audio-s/s | speedup vs N=1 | ideal encodes/s | latency med ms | latency p95 ms | encode-stage med ms | batch CPU ms | CPU/encode ms | cores busy | RSS/child MB | resident for N MB | load 1m |
| ---: | ---: | ---: | ---: | ---: | ---: | ---: | ---: | ---: | ---: | ---: | ---: | ---: | ---: | ---: |
| 1 | 39.79 | 39.79 | 1.00× | 39.8 | 24.7 | 30.2 | 19.3 | 49.5 | 49.5 | 1.96 | 14.3 | 14 | 135/140/150 |
| 2 | 73.91 | 73.91 | 1.86× | 79.6 | 26.4 | 27.9 | 21.1 | 125.5 | 62.8 | 4.66 | 14.4 | 29 | 135/140/150 |
| 3 | 101.54 | 101.54 | 2.55× | 119.4 | 28.5 | 33.2 | 23.2 | 195.6 | 65.2 | 6.79 | 14.3 | 43 | 135/140/150 |
| 4 | 131.12 | 131.12 | 3.30× | 159.1 | 29.0 | 31.5 | 23.7 | 217.2 | 54.3 | 7.03 | 14.4 | 57 | 135/135/150 |
| 6 | 167.65 | 167.65 | 4.21× | 238.7 | 33.6 | 42.1 | 28.1 | 272.0 | 45.3 | 7.52 | 14.4 | 86 | 135/135/150 |
| 8 | 190.55 | 190.55 | 4.79× | 318.3 | 40.0 | 47.6 | 34.0 | 330.6 | 41.3 | 7.75 | 14.4 | 115 | 135/135/150 |
| 16 | 238.55 | 238.55 | 6.00× | 636.6 | 60.6 | 77.3 | 52.4 | 593.9 | 37.1 | 8.79 | 14.4 | 230 | 135/135/150 |

### 2ch/48000 — internal parallel off

| N | encodes/s | audio-s/s | speedup vs N=1 | ideal encodes/s | latency med ms | latency p95 ms | encode-stage med ms | batch CPU ms | CPU/encode ms | cores busy | RSS/child MB | resident for N MB | load 1m |
| ---: | ---: | ---: | ---: | ---: | ---: | ---: | ---: | ---: | ---: | ---: | ---: | ---: | ---: | ---: |
| 1 | 41.09 | 41.09 | 1.00× | 41.1 | 23.9 | 25.5 | 18.8 | 22.5 | 22.5 | 0.93 | 13.5 | 14 | 135/140/150 |
| 2 | 81.23 | 81.23 | 1.98× | 82.2 | 23.9 | 26.1 | 18.8 | 45.0 | 22.5 | 1.83 | 13.6 | 27 | 135/140/150 |
| 3 | 120.02 | 120.02 | 2.92× | 123.3 | 24.0 | 24.8 | 18.8 | 67.4 | 22.5 | 2.69 | 13.5 | 41 | 135/135/150 |
| 4 | 157.05 | 157.05 | 3.82× | 164.4 | 24.1 | 25.8 | 18.9 | 89.8 | 22.5 | 3.54 | 13.5 | 54 | 135/135/150 |
| 6 | 226.39 | 226.39 | 5.51× | 246.6 | 24.4 | 25.8 | 19.0 | 135.9 | 22.7 | 5.10 | 13.5 | 81 | 135/135/150 |
| 8 | 289.01 | 289.01 | 7.03× | 328.7 | 24.9 | 28.0 | 19.1 | 181.6 | 22.7 | 6.58 | 13.5 | 108 | 135/135/150 |
| 16 | 339.38 | 339.38 | 8.26× | 657.5 | 32.8 | 44.6 | 23.2 | 391.1 | 24.4 | 8.30 | 13.6 | 217 | 135/140/150 |

`audio-s/s` is the same rate as audio-seconds of input encoded per second of
wall clock; because the two geometries are different lengths (3.161 s against
1.000 s), encodes per second alone would not compare them.

### Paired parallel / sequential, median over 20 repetitions

The ratio of the two configurations measured at the same N inside the same
repetition. **Above 1, the internal parallelism is still paying for itself;
at or below 1 it is being paid for out of somebody else's core.**

| Geometry | N | pairs | throughput ratio | latency ratio | CPU-per-encode ratio |
| --- | ---: | ---: | ---: | ---: | ---: |
| 6ch/44100 | 1 | 20 | 1.18× | 0.84× | 2.61× |
| 6ch/44100 | 2 | 20 | 1.16× | 0.86× | 2.40× |
| 6ch/44100 | 3 | 20 | 1.10× | 0.91× | 2.03× |
| 6ch/44100 | 4 | 20 | **1.06×** | 0.94× | 1.80× |
| 6ch/44100 | 6 | 20 | **0.91×** | 1.09× | 1.52× |
| 6ch/44100 | 8 | 20 | 0.80× | 1.26× | 1.42× |
| 6ch/44100 | 16 | 20 | 0.77× | 1.47× | 1.19× |
| 2ch/48000 | 1 | 20 | **0.96×** | 1.04× | 2.20× |
| 2ch/48000 | 2 | 20 | 0.91× | 1.10× | 2.81× |
| 2ch/48000 | 3 | 20 | 0.85× | 1.19× | 2.96× |
| 2ch/48000 | 4 | 20 | 0.85× | 1.19× | 2.43× |
| 2ch/48000 | 6 | 20 | 0.74× | 1.37× | 2.00× |
| 2ch/48000 | 8 | 20 | 0.66× | 1.59× | 1.81× |
| 2ch/48000 | 16 | 20 | 0.69× | 1.84× | 1.50× |

### Cores to throughput: what each configuration does with the cores it gets

Speedup against that series' own N = 1, divided by cores busy. 1.00 means every
core obtained became throughput in proportion.

| Geometry | Configuration | N=1 | N=2 | N=3 | N=4 | N=6 | N=8 | N=16 |
| --- | --- | ---: | ---: | ---: | ---: | ---: | ---: | ---: |
| 6ch/44100 | parallel on | 0.33 | 0.35 | 0.42 | 0.47 | 0.56 | 0.60 | 0.62 |
| 6ch/44100 | parallel off | 1.02 | 1.02 | 1.02 | 1.01 | 1.01 | 0.99 | 0.90 |
| 2ch/48000 | parallel on | 0.51 | 0.40 | 0.38 | 0.47 | 0.56 | 0.62 | 0.68 |
| 2ch/48000 | parallel off | 1.08 | 1.08 | 1.08 | 1.08 | 1.08 | 1.07 | 0.99 |

## Where scaling stops, per configuration

**6ch/44100, internal parallel off — it does not stop; the machine runs out
first.** Throughput rises monotonically to the largest N measured, speedup
9.08× at N = 16, and the cores-to-throughput efficiency stays at 0.99–1.02
through N = 8. The shortfall against ideal at N = 16 (63.83/s measured against
112.4/s ideal) is the environment: the batch only ever obtained 10.06 cores, so
it delivered 9.08× on 10.06 cores. The curve stops at the machine, not at the
encoder. Latency backs this up — 141.7 ms to 146.7 ms and flat all the way to
N = 8, +3.5% over an 8× range, which is what one encode per core looks like.

**6ch/44100, internal parallel on — it stops sooner the less loaded the machine
is.** In the loaded record the throughput ratio crosses 1 between N = 4 and
N = 6 (1.06× at N = 4, 0.91× at N = 6); in the quiet record it has already
crossed by N = 4 (1.02× at N = 3, 0.94× at N = 4). The efficiency number shows
the mechanism: at N = 1 the batch holds 3.06 cores to do one encode's work and
converts them at 0.33 — two thirds of the machine time it obtains is lost. As N
rises it obtains more cores (9.07 at N = 16) but converts them no better than
0.62, and at N = 16 it is slower than the sequential configuration while using
*almost the same number of cores* (47.37/s on 9.07 cores against 63.83/s on
10.06). By N = 6 the sequential configuration has already overtaken it in raw
throughput (40.94/s against 36.83/s), and in the quiet record it has overtaken
it by N = 4 (27.52/s against 25.49/s), so past the crossing the feature is not a
trade-off, it is a loss. The latency column shows the same crossing first: p95
goes 149.7 ms at N = 2 to 300.2 ms at N = 6 and 465.9 ms at N = 16, against
185.0 ms and 286.2 ms for the sequential configuration at N = 8 and 16.

**Why the crossing moves, in one line.** The sequential configuration's numbers
barely change between the two records — 27.51/s against 27.52/s at N = 4,
63.83/s against 66.66/s at N = 16 — because one encode per core is a shape the
machine can always serve. The parallel configuration's do change: 29.10/s
against 25.49/s at N = 4. A `parallel` encode's cost is whatever it can grab and
then fail to convert, so when the neighbours take cores it is *held back* into
being less wasteful, and when they stop taking cores it is free to waste more.
That is why less pressure means an earlier crossing, and it is the reason the
deployment advice below is a bound rather than a number.

**2ch/48000, internal parallel off — same shape as the 6-channel case.**
Monotonic to N = 16, speedup 8.26×, efficiency 1.07–1.08 flat through N = 8,
latency 23.9 ms → 24.9 ms through N = 8.

**2ch/48000, internal parallel on — it never pays, not even at N = 1.** There
is no crossing to find: the ratio starts at 0.96× in the loaded record and 0.94×
in the quiet one, and falls to 0.69× and 0.66×. A 1.000 s stereo encode is
~19 ms of work, which is too little to amortize a per-channel partition across
two channels; the batch holds 1.96 cores to do one encode's work at N = 1 and
converts them at 0.51. The per-encode latency tells the same story from the
caller's side: 24.7 → 60.6 ms across the range, 2.45×, against 23.9 → 32.8 ms
(1.37×) with the feature off.

**The quiet record, for comparison.** The same matrix at 6-channel cell loads of
25–60 and stereo cell loads of 60–113. The column that matters is the pair
`par enc/s` against `seq enc/s`: the feature is ahead at N = 1–3 and behind from
N = 4. Medians over 20 repetitions.

| Geometry | N | par enc/s | seq enc/s | par lat med | seq lat med | par CPU/enc | seq CPU/enc | par cores | seq cores | load 1m |
| --- | ---: | ---: | ---: | ---: | ---: | ---: | ---: | ---: | ---: | ---: |
| 6ch/44100 | 1 | 8.30 | 7.06 | 120.2 | 141.1 | 343.7 | 139.6 | 2.87 | 0.98 | 25/31/60 |
| 6ch/44100 | 2 | 15.89 | 13.93 | 124.8 | 142.3 | 289.0 | 140.1 | 4.65 | 1.96 | 25/31/60 |
| 6ch/44100 | 3 | 21.13 | 20.68 | 139.2 | 143.1 | 238.8 | 141.0 | 5.05 | 2.91 | 25/32/60 |
| 6ch/44100 | 4 | 25.49 | 27.52 | 154.3 | 143.2 | 218.7 | 140.7 | 5.44 | 3.87 | 25/32/60 |
| 6ch/44100 | 6 | 30.75 | 40.20 | 191.0 | 144.8 | 195.8 | 141.8 | 6.03 | 5.72 | 25/32/60 |
| 6ch/44100 | 8 | 36.10 | 51.59 | 215.8 | 148.0 | 191.8 | 143.3 | 6.85 | 7.40 | 25/32/60 |
| 6ch/44100 | 16 | 47.00 | 66.66 | 326.2 | 208.3 | 184.6 | 152.2 | 8.74 | 10.07 | 25/32/60 |
| 2ch/48000 | 1 | 37.65 | 40.32 | 26.2 | 24.3 | 55.8 | 22.9 | 1.96 | 0.93 | 60/73/96 |
| 2ch/48000 | 2 | 68.62 | 76.85 | 28.2 | 24.6 | 58.9 | 23.1 | 4.28 | 1.80 | 60/73/96 |
| 2ch/48000 | 3 | 83.74 | 117.47 | 34.8 | 24.5 | 47.5 | 22.9 | 4.30 | 2.66 | 60/73/96 |
| 2ch/48000 | 4 | 105.41 | 153.64 | 34.9 | 24.6 | 48.1 | 22.7 | 4.83 | 3.49 | 60/73/96 |
| 2ch/48000 | 6 | 151.40 | 214.65 | 36.9 | 25.0 | 44.7 | 22.7 | 6.69 | 4.88 | 60/73/113 |
| 2ch/48000 | 8 | 172.77 | 286.07 | 42.6 | 25.1 | 39.7 | 22.8 | 6.73 | 6.49 | 60/73/113 |
| 2ch/48000 | 16 | 229.55 | 300.65 | 62.3 | 35.4 | 37.7 | 24.2 | 8.63 | 7.10 | 60/73/113 |

**Reproduced — and the one run that moved.** Five full runs at different machine
loads. Four of them sat in the same load band and agreed; the fifth was taken in
the quietest window of the session and moved the crossing earlier, which is the
result rather than a disagreement:

| Run | load 1m (6ch cells) | 6ch ratio N=3 | 6ch ratio N=4 | 6ch ratio N=6 | crossing | 2ch ratio N=1 → N=16 |
| --- | --- | ---: | ---: | ---: | --- | --- |
| 1 (20 reps) | 96–228 | — | 1.09× | 0.95× | between 4 and 6 | 0.95× → 0.71× |
| 2 (20 reps, committed record) | 145–231 | — | 1.06× | 0.91× | between 4 and 6 | 0.96× → 0.69× |
| 3 (20 reps) | 76–140 | — | 1.08× | 0.96× | between 4 and 6 | 0.97× → 0.68× |
| 4 (20 reps) | 100–150 | — | 1.03× | 0.88× | between 4 and 6 | 0.97× → 0.71× |
| 5 (20 reps, quiet record) | 25–60 | 1.02× | **0.94×** | 0.77× | **between 3 and 4** | 0.94× → 0.66× |

Runs 2 and 5 have their raw per-child samples committed
([loaded](../figures/concurrency-samples.json),
[quiet](../figures/concurrency-samples-quiet-window.json)); runs 1, 3 and 4 are
quoted from their summary tables and their `N = 3` ratios were not recorded, so
that column is a dash rather than a guess. The matrix is about a minute of wall
clock per run, so any of it can be re-measured rather than taken on trust.

**What the five runs settle, and what they do not.** Load was the variable: the
6-channel cells ran at a median 1-minute load of 178 in the loaded record and 32
in the quiet one, on the same machine, same binaries, same inputs, and the
crossing interval moved from (4, 6] to (3, 4]. The direction is the one the
mechanism predicts and the movement is larger than the run-to-run spread among
the four loaded runs, so the crossing is a function of available cores and not a
property of the configuration alone. It is *not* evidence about an idle machine:
the quiet window still ran at load 25–60 against 16 cores, and the batch never
obtained more than 10–11 cores in any record. The extrapolation to a machine
with nothing else running is in
[What a quiet machine would change](#what-a-quiet-machine-would-change), marked
as extrapolation.

## The figure

![Encoding throughput, per-encode latency and CPU per encode against
concurrency, for both installed geometries with the internal parallel feature
on and off](../figures/concurrency-curves.png)

`docs/figures/concurrency-curves.png`, rendered by
`scripts/plot_concurrency_curves.py` from the loaded record's JSON. The quiet
record is a JSON only: the figure's job is to show the shape of the curve once,
and the load comparison is a table because it is a comparison between two runs
rather than an extra series.

**Six panels, 3 × 2.** One row per geometry, one column per question:
throughput, per-encode latency, CPU per encode. One figure rather than two
because the three questions are read together — a reader who sees throughput
flatten wants the latency and CPU panels beside it, not in another image.

- **A row per geometry rather than four series sharing one panel,** because the
  geometries are about five times apart in both rate (8/s against 41/s) and
  latency (119 ms against 24 ms). Sharing an axis would compress one of them to
  a flat line; the alternative, twin axes in one panel, makes the reader check
  which series belongs to which scale. Three columns with per-panel scales costs
  one extra look and removes the ambiguity.
- **The ideal-linear reference is per series, anchored at that series' own
  N = 1**, drawn as a thin dotted line in the same colour. Anchoring each series
  at its own starting rate is what makes the gap readable as lost scaling rather
  than as a difference in baseline; one shared reference line would compare a
  parallel run against a sequential baseline it never claimed. Drawn to scale it
  also sets the throughput panels' y range — it climbs to 134.2/s and 657.5/s
  where the measured series reach 63.8/s and 339.4/s — so the measured series
  fill 41% and 44% of those panels against 91% for every latency and CPU panel.
  That band *is* the gap the panel exists to show, so it is left whole: an axis
  clamped to the data would take the parallel series' reference off the top of
  the frame from N ≈ 6, which is where the feature is losing.
- **The latency panel draws median solid and p95 dashed**, because the p95 is
  where oversubscription appears first and it is the number a caller with a
  timeout cares about: on the fixture it leaves the median behind at N = 6.
- **The CPU panel is drawn, not only tabulated.** It is the load-insensitive
  quantity and so the one that survives this machine's noise; a reader deciding
  a deployment shape should be able to see that the parallel series starts
  higher than the sequential one and never crosses below it.
- **The N axis is linear, because the samples are not evenly spaced and the
  axis must say so.** The matrix is N = 1, 2, 3, 4, 6, 8, 16, so the last
  segment is drawn eight units wide against one for the first. The flattening
  that shows is in the data and not in the spacing: on the linear axis, the
  sequential 6-channel series loses slope from 7.02 encodes/s per unit of N at
  the first step to 1.44 at the last, and that is what the reader sees. Evenly
  spaced ticks would be the misleading choice, drawing the 8 → 16 step — 11.52
  encodes/s over eight units of N — across the same width as the 1 → 2 step's
  7.02 over one, and so at eight times its true slope.

**The size is a width budget, and 11 in did not meet it.** At 110 DPI the widest
object a single panel has to hold is the per-geometry input caption (467 px),
then the longest panel title (380 px) and the widest legend (276 px); the
tightest is the run of tick labels at N = 1, 2, 3, 4, which sit one N apart. At
11 in a panel was 198 px, so all three of those objects were wider than the
panel they belonged to: the third column's titles were clipped by the right edge
of the figure (measured: they ended at x = 1284 in a 1210 px image) and the
throughput legends by the left edge (x = -4.5), while the four tick labels had
2 px between them and read as one number. The caption under column 1 was not
merely a symptom — it is an unclipped annotation and so part of that column's
tight bounding box, so its 467 px widened the column from 198 px to 529 px,
which `tight_layout` paid for by shrinking *every* panel and leaving a 260 px
gap between the columns, wider than the panels themselves. At 16.5 in a panel is
492 px: the caption takes 0.95 of it, the title 0.77 and the legend 0.56, the
four tight tick labels have 19.8 px between them, no artist is drawn outside the
figure, and the gap between columns is 122 px. The caption is anchored in points
under its axes rather than in axes fractions, so it no longer moves with the
panel height it is sitting below.

**Style is the default `rcParams`, unmodified.** No style sheet, no
`rcParams.update`. The only chosen values are the figure size (16.5 × 11.5 in)
and the DPI (110), both for legibility; the default colour cycle (`tab10`) and
the default font (DejaVu Sans) are left alone, and the figure is meant to be
recognisable as an ordinary Matplotlib figure. Matplotlib is a documentation
tool for this one script: it is not in `pyproject.toml`, not in any crate, and
not in the shared `.venv`. It was installed into a throwaway venv inside the
worktree, which is ignored.

### Byte-stability

The figure is a committed generated asset, so re-rendering it must leave an
empty diff. `--check` re-renders to a scratch directory of its own and compares
digests without writing anything, which is the form to use after a
regeneration:

```sh
./.venv/bin/python scripts/plot_concurrency_curves.py --check
```

For the explicit two-file comparison, render twice and compare; `TMPDIR` rather
than a fixed scratch path, so the second copy lands wherever this machine puts
temporary files:

```sh
./.venv/bin/python scripts/plot_concurrency_curves.py
shasum -a256 docs/figures/concurrency-curves.png
./.venv/bin/python scripts/plot_concurrency_curves.py --out "$TMPDIR/run2.png"
cmp docs/figures/concurrency-curves.png "$TMPDIR/run2.png"
```

Three consecutive renders of the committed JSON, at the figure size above, all
produced
`sha256 e199b898cc0f4568ae3793782da0720a59c39185fb8b84de17e42d304d310421`, and
`cmp` reported no difference. (The digest the 11 × 13 in figure had,
`d928f2b5c8cb6fddb06602f62fd85e605c45f03efde2952d9639aebd09b8e8ac`, is
superseded: the raster geometry is part of the image, so a deliberate change of
figure size changes the digest and that is the only reason this one did.)

What makes it hold: figure size and DPI pinned, so the raster geometry cannot
drift with a display; every series sorted by N before it is drawn, so the drawn
path is a function of the data and not of the JSON's key order; nothing jittered
and no random state seeded; `metadata={}`.

**SVG is not byte-stable here, and that is why the committed format is PNG.**
Two renders of one dataset gave different digests
(`bdd82e1b97b153af…` against `b791a5faeb75b440…`). Diffing them names two
causes: a wall-clock `<dc:date>` element, and element ids and
`clip-path`/marker references (`p061c9c1b00`, `m8705d8dd01`) derived from a
per-process value, which differ between interpreter runs. Setting
`metadata={"Date": None}` removes neither. The documented remedy is the
`svg.hashsalt` rcParam; tuning an rcParam is what the default-style rule above
rules out, so SVG is not produced. PDF is also byte-stable and is not needed.

One limit worth stating: PNG's only text chunk is `Software`, carrying the
Matplotlib version — Matplotlib 3.11.2 wrote no creation date here at all, with
or without `metadata={}`, so `metadata={}` is the declaration that nothing
ambient is embedded rather than the thing suppressing a date. The practical
consequence is that byte-stability holds **for a pinned Matplotlib**: a
different version changes that chunk and therefore the digest. A regeneration
check that fails after a Matplotlib upgrade has failed on the version, not on
the data.

## What it means for a caller

**How many concurrent encodes this machine sustains.** With the internal
parallelism off, the batch obtained and used ~10 of the 16 cores, so this
machine sustains about **10 concurrent encodes of this shape** — beyond that
concurrency is queued, not served. At N = 16 the two geometries delivered
**202 audio-s/s** (6-channel, 3.161 s each) and **339 audio-s/s** (stereo,
1.000 s each) — 202× and 339× real time in aggregate, against 22× and 41× for
one encode at a time.

**The deployment shape.** Run **one encode per core with the internal
`parallel` feature off.** On this machine the sequential configuration is the
fastest measured for both geometries (63.83/s and 339.38/s at N = 16 in the
loaded record, 66.66/s and 300.65/s in the quiet one), it is the one that keeps
a caller's wait flat (fixture latency +3.5% from N = 1 to N = 8), and it is the
one that does not spend CPU it cannot convert. Reach for the feature in exactly
two situations:

- **a single long encode that cannot be split across requests**, where 1.17× on
  the fixture's 3.161 s is real even though it costs 2.43× the CPU (quiet
  record; the loaded record measures the same trade at 1.18× and 2.61×); and
- **a 6-channel workload whose concurrency is capped at 3 or below** — for
  instance a fixed worker pool of 2 or 3 — where even the quieter record still
  shows the feature 1.02× to 1.17× ahead. Treat 3 as the ceiling: it is the
  highest N measured at which the feature is still ahead, and the crossing
  moved down, not up, when the machine got quieter.

Past the crossing — beyond N = 3 in the quieter record, beyond N = 4 in the
loaded one — turning the feature on is worse on throughput, worse on latency and
worse on CPU per encode at the same time. There is no N and no load at which
the stereo geometry benefits.

**The number to size a worker pool with.** Peak RSS is 38–44 MB per child for
6-channel and 13.5 MB for stereo, nearly independent of N — so budget
**N × 40 MB** for six-channel workers and **N × 14 MB** for stereo ones: about
642 MB and 217 MB at N = 16. The batch's own CPU per encode is 140–159 ms for
6-channel and 22–25 ms for stereo with the feature off, and it stays within 14%
of its N = 1 value across the whole range — the sequential configuration does
not degrade as it is stacked.

### What a quiet machine would change

Two full records bracket a factor of five in load: the 6-channel cells ran at a
median 1-minute load of 178 and then of 32, against 16 cores. That is enough to
answer part of the question by measurement and leaves the rest as
extrapolation. Direction of each movement:

- **The crossing for the feature moved earlier, and by measurement rather than
  prediction.** Halving the load five times over took the crossing interval from
  (4, 6] to (3, 4]. The mechanism is in the efficiency number: a `parallel`
  encode's cost is what it can grab and then fail to convert, so neighbour
  pressure *suppresses* the waste, and removing that pressure lets the waste
  back in. **Extrapolation:** on a machine with nothing else running, the
  crossing should be lower still — the honest reading of the two records is
  that **N = 3 is the highest concurrency at which the feature is still ahead,
  and it may not be ahead even there** once the machine is idle. Size a worker
  pool for the sequential configuration and treat the feature as a
  single-encode option.
- **The sequential configuration's rates rose only slightly, and its shape did
  not change.** 27.52/s against 27.51/s at N = 4, 66.66/s against 63.83/s at
  N = 16, and cores busy 0.98/0.98, 1.96/1.95, 2.91/2.91 and 3.87/3.87 at
  N = 1, 2, 3 and 4 (quiet against loaded) — the same cores for the same work.
  One encode per core is a shape the machine serves the same way at load 32 and
  at load 178, which is why the deployment advice rests on it.
- **The throughput ceilings should rise on a genuinely idle machine, and the
  measurements hint at how much.** The batch never obtained more than 10.07
  cores in the quiet record (6-channel, sequential, N = 16) even at load 32, so
  the ceiling is still the environment rather than the encoder. On a quiet
  16-core machine the sequential configuration should approach its ideal column
  — about **112/s** for 6-channel and **658/s** for stereo at N = 16 — where
  66.66/s and 300.65/s were observed in the quiet record. That is
  **extrapolation from the efficiency ratio**, not a measurement: it assumes the
  0.99–1.02 cores-to-throughput efficiency holds out to 16 cores, and nothing
  here observes more than ~11.
- **CPU per encode and peak RSS should barely move, and across the two records
  they did not.** They are properties of a single encode's work and its process
  image rather than of contention. 6-channel sequential CPU per encode is
  139.6/139.7 ms at N = 1 and 152.2/158.6 ms at N = 16 across the quiet and
  loaded records; peak RSS on the sequential configuration is 37.4–41.6 MB
  against 38.1–41.6 MB. The cross-check agrees: the encode-performance lane
  measured 156.93 ms for the same fixture on the sequential build, the same
  quantity at a third moment and the same order.
- **Latency at N = 1 is the one row that is nearly trustworthy already**, since
  a single process mostly waits for a core; 118.8 ms for the fixture in the
  loaded record and 120.2 ms in the quiet one sit next to the 114.91 ms
  `encode_pcm` and 122.38 ms CLI medians the encode-performance lane recorded.

Nothing in this document is a threshold and nothing in the tree compares
against these numbers. They are one machine, one session and a 1-minute load
average that never fell below 25 against 16 cores and reached 231; re-run the
matrix rather than quoting a row.

## Trust boundary

- One machine (Apple M3 Max, 16 cores, 64 GB, macOS 27, rustc 1.96.0), one
  session, release builds, and no idle window: across the five runs the
  1-minute load average sat between 25 and 231, with the two committed records
  at 6-channel cell loads of 145–231 and 25–60. The paired ratios, CPU per
  encode and cores busy are the quantities that survive that; the rate tables
  are context.
- Latency is the interval from the first spawn of the batch to that child's
  exit, so it includes process start-up — the `--time` encode-stage column is
  printed beside it so the two can be told apart. It is the interval a caller
  who shells out sees, not the interval an in-process caller sees.
- The two configurations are timed through two separate binaries built from the
  same source, and both were verified to produce identical bytes for both
  inputs before anything was timed. The comparison is therefore between
  scheduling configurations, not between code.
- `cores busy` and `cpu/encode` come from each child's own `wait4` rusage, which
  counts the whole process including its rayon workers and its start-up; that is
  the intended quantity for "what does this configuration cost the machine".
- The p95 is over 20 × N observations per cell, so the N = 1 row's p95 is the
  upper end of 20 samples rather than a tail estimate, and the N = 16 row's is
  over 320. Read the p95 column as a shape, not as a service objective.
- No file under `crates/*/src/` or `src/` was changed for this measurement, so
  every number describes the tree as committed, and the byte-exactness suites
  ran unchanged.