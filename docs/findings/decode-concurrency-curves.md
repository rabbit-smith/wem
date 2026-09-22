# Decode concurrency: what N simultaneous decodes do to this machine

**Result.** Decode is fast enough that concurrency is the axis a server meets
first, and it behaves well: on a machine already carrying a 1-minute load
average of 22 against 16 cores,

- **Throughput scales with cores up to eight decodes at a time, and then bends
  without wasting work.** On the 6-channel fixture, 31.1 decodes/s at N = 1
  becomes 222.8/s at N = 8 (7.16× the rate on 6.98 cores) and 289.6/s at N = 16
  (9.31× on 9.64 cores). On the 1-second stereo reference, 110.0/s at N = 1
  becomes 681.8/s at N = 8 (6.20× on 5.15 cores) and 919.6/s at N = 16 (8.36×
  on 7.29 cores).
- **CPU per decode does not inflate.** 30.4 ms at N = 1 against 33.4 ms at
  N = 16 on the fixture (+10%), and 7.5 ms against 7.9 ms on stereo (+5%): the
  bend is contention for the cores the machine actually had, not extra work
  invented by the scheduler.
- **The stereo bend is the process floor, not the decoder.** A stereo decode
  takes 5.4 ms of CPU and 8.7 ms of caller-visible latency at N = 1 — 3.3 ms of
  process start-up sits in front of every decode — and at N = 16 the stereo
  cells use only 7.29 of 16 cores (the 6-channel cells use 9.64). The decoder
  finished before the next process could be started.
- **Latency holds at its N = 1 level through N = 8** (31.8 → 33.3 ms on the
  fixture, 8.7 → 9.3 ms on stereo) and rises 21-24% by N = 16 (38.4 ms, p95
  51.8 ms; 10.8 ms, p95 15.4 ms). A caller who keeps N at or below the cores it
  has pays the single-decode latency.
- **The push-chunk size is not a concurrency variable.** Paired inside the same
  repetition and order-alternated, 4 KiB against 64 KiB pushes is 0.96-1.01× on
  throughput, 0.99-1.01× on latency and 0.99-1.03× on CPU per decode, across
  both geometries and every N. It *is* a memory variable — 19.7-20.4 MB
  resident per child at 4 KiB against 22.0-22.8 MB at 64 KiB on the fixture —
  and that difference is the reply buffer, not the decoder
  ([decode-performance.md](decode-performance.md), finding 3).
- **Resident per concurrent decode is the number to size a pool with**, and it
  is where this surface's contract defect lands: 22.0 MB per child for a 3.16 s
  fixture and 9.8 MB for a 1 s stereo stream, so N = 16 is 357 MB and 158 MB
  respectively — and each of those children is holding the stream's whole
  synthesized timeline, which grows with the stream
  ([decode-performance.md](decode-performance.md), finding 1).

## The figure

![Decoding throughput, per-decode latency and CPU per decode against
concurrency, for both installed geometries at two push-chunk
sizes](../figures/decode-concurrency-curves.png)

`docs/figures/decode-concurrency-curves.png`, rendered by
`scripts/plot_decode_concurrency_curves.py` from
`docs/figures/decode-concurrency-samples.json`. **Six panels, 3 × 2**: one row
per geometry, one column per question — throughput, per-decode latency, CPU per
decode. One figure rather than two because the three questions are read
together: a reader who sees throughput flatten wants the latency and CPU panels
beside it. The dotted lines are the figure's two references — the ideal-linear
rate anchored at each series' own N = 1 in the throughput panel, and the N = 1
level in the latency and CPU panels.

## What was measured, and how

Reproduce with:

```sh
cargo test --release --manifest-path crates/Cargo.toml -p wem-core \
    --test decode_stage_timings --no-run
python3 scripts/measure_decode_concurrency.py                    # writes the JSON
./.venv/bin/python scripts/plot_decode_concurrency_curves.py     # re-renders the figure
```

The matrix is `N x {4 KiB, 64 KiB} push chunks x {6 ch/44100 fixture, 2 ch/48000
stereo reference}`, 7 repetitions per cell, 140 batches and 868 decodes in
3.57 s of wall clock. Each child is the `decode_process_point` entry of
`crates/wem-core/tests/decode_stage_timings.rs` — the shipped `DecodeSession`
behind a process boundary, fed the WEM in `WEM_DECODE_CHUNK`-sized reads and
discarding every sample it is handed. The batch starts N of them as close
together as the spawn syscall allows and reaps each with `os.wait4`, so every
child yields its own exit instant, its own `ru_utime`/`ru_stime` and its own
`ru_maxrss`. `wait4(-1)` is what makes the latency column measurable: it returns
as soon as *any* child exits and names which one, so a child's exit instant is
an observation rather than a position in a sequential reap order. Each child
also reports the decode time it measured itself, which is the column that
separates the decoder from the process.

The two chunk regimes are **paired and order-alternated**: within one repetition
both run at the same N back to back, and which of the two leads alternates with
the repetition index, so drift in machine load lands on both sides of the ratio.

### Machine identity (numbers are meaningless without it)

| | |
| --- | --- |
| CPU | Apple M3 Max, 16 logical cores (12 P + 4 E) |
| Memory | 64 GB (68 719 476 736 bytes) |
| OS | macOS 27.0, Darwin 27.0.0, arm64 |
| Toolchain | rustc 1.96.0 (ac68faa20), host aarch64-apple-darwin, LLVM 22.1.2 |
| Python | 3.14.6 |
| Load average at start | 21.73, 21.47, 27.79 (1, 5, 15 min) |
| Load 1-minute during the matrix | min 21.73, median 21.73, max 21.73 |
| Spawn span | median 1.05 ms, max 4.88 ms per batch |

The machine was **busy** for the whole record — a 1-minute load average of 21.7
against 16 cores, which did not move while the matrix ran — so the absolute
rates are what this machine delivered with 5-6 cores already spoken for, not
what an idle machine delivers. The paired ratios, CPU per decode and cores busy
are the load-insensitive quantities.

## The numbers

### 6 ch/44100 — 64 KiB pushes (`tests/fixtures/reference.wem`)

| N | decodes/s | audio-s/s | speedup vs N=1 | ideal decodes/s | latency med ms | latency p95 ms | decode-stage med ms | batch CPU ms | CPU/decode ms | cores busy | RSS/child MB | resident for N MB |
| ---: | ---: | ---: | ---: | ---: | ---: | ---: | ---: | ---: | ---: | ---: | ---: | ---: |
| 1 | 31.10 | 98.31 | 1.00× | 31.1 | 31.8 | 32.7 | 28.1 | 30.4 | 30.4 | 0.95 | 22.0 | 22 |
| 2 | 61.21 | 193.49 | 1.97× | 62.2 | 32.0 | 33.3 | 28.3 | 61.3 | 30.7 | 1.88 | 22.1 | 44 |
| 4 | 117.21 | 370.50 | 3.77× | 124.4 | 32.9 | 33.5 | 29.0 | 124.5 | 31.1 | 3.67 | 22.4 | 90 |
| 8 | 222.77 | 704.16 | 7.16× | 248.8 | 33.3 | 34.5 | 29.2 | 251.2 | 31.4 | 6.98 | 22.8 | 182 |
| 16 | 289.61 | 915.43 | 9.31× | 497.6 | 38.4 | 51.8 | 32.1 | 534.3 | 33.4 | 9.64 | 22.3 | 357 |

### 2 ch/48000 — 64 KiB pushes (`tests/data/2ch-reference/stereo_noise.wem`)

| N | decodes/s | audio-s/s | speedup vs N=1 | ideal decodes/s | latency med ms | latency p95 ms | decode-stage med ms | batch CPU ms | CPU/decode ms | cores busy | RSS/child MB | resident for N MB |
| ---: | ---: | ---: | ---: | ---: | ---: | ---: | ---: | ---: | ---: | ---: | ---: | ---: |
| 1 | 110.01 | 110.01 | 1.00× | 110.0 | 8.7 | 9.3 | 5.4 | 7.5 | 7.5 | 0.83 | 9.8 | 10 |
| 2 | 215.38 | 215.38 | 1.96× | 220.0 | 8.8 | 9.0 | 5.4 | 15.1 | 7.6 | 1.63 | 9.9 | 20 |
| 4 | 402.09 | 402.09 | 3.65× | 440.1 | 8.8 | 9.2 | 5.4 | 29.9 | 7.5 | 2.99 | 9.8 | 39 |
| 8 | 681.75 | 681.75 | 6.20× | 880.1 | 9.3 | 10.4 | 5.5 | 60.8 | 7.6 | 5.15 | 9.9 | 79 |
| 16 | 919.60 | 919.60 | 8.36× | 1760.2 | 10.8 | 15.4 | 5.5 | 126.8 | 7.9 | 7.29 | 9.9 | 158 |

The 4 KiB cells are in the JSON and in `--tables`
(`./.venv/bin/python scripts/plot_decode_concurrency_curves.py --tables`); they
are not tabulated twice here because the paired comparison below is the result
and the two regimes differ by less than the run-to-run spread.

### Paired 4 KiB / 64 KiB pushes, median over 7 repetitions

| geometry | N | throughput ratio | latency ratio | CPU-per-decode ratio |
| --- | ---: | ---: | ---: | ---: |
| 6ch/44100 | 1 | 1.00× | 1.00× | 0.99× |
| 6ch/44100 | 2 | 1.01× | 0.99× | 0.99× |
| 6ch/44100 | 4 | 1.01× | 0.99× | 0.99× |
| 6ch/44100 | 8 | 1.01× | 0.99× | 0.99× |
| 6ch/44100 | 16 | 0.96× | 1.00× | 0.99× |
| 2ch/48000 | 1 | 0.98× | 1.01× | 1.03× |
| 2ch/48000 | 2 | 1.01× | 0.99× | 0.99× |
| 2ch/48000 | 4 | 1.00× | 1.00× | 1.00× |
| 2ch/48000 | 8 | 0.99× | 1.00× | 1.00× |
| 2ch/48000 | 16 | 0.99× | 0.99× | 1.00× |

## Findings

### 1. Decode turns cores into throughput at 1.03-1.05 efficiency to N = 8, then bends — measured

Speedup divided by the cores the batch actually used: 6-channel 1.05 (N = 2),
1.03 (N = 4), 1.03 (N = 8), 0.97 (N = 16); stereo 1.20 (N = 2), 1.22 (N = 4),
1.20 (N = 8), 1.15 (N = 16). Above 1.0 means the batch delivered slightly more
than its own CPU time in rate — the children's `wait4` CPU excludes the parent's
spawn work, which is real work the caller pays. The bend at N = 16 is the
machine: the fixture cells used 9.64 of 16 cores while the 1-minute load average
stood at 21.7, and CPU per decode rose only 10%, so the missing rate is cores
the neighbours had, not work the decoder wasted.

### 2. The stereo curve bends for a different reason: process start-up — measured

Stereo decodes in 5.4 ms but a caller sees 8.7 ms at N = 1: **3.3 ms of
process**. At N = 16 the stereo batch uses 7.29 cores (9.64 for the fixture) and
its latency p95 is 15.4 ms against a 5.5 ms decode. The parent can only spawn
processes so fast — the median spawn span for a whole batch is 1.05 ms and the
worst 4.88 ms — so at high N the batch's wall clock is start-up plus the last
child's decode, not N × decode. A caller that decodes in-process (the Rust
session, the C ABI inside an existing process) does not pay this floor at all;
the stereo numbers here are a *lower bound* on in-process scaling and an upper
bound on the latency a shell-out caller sees.

### 3. Push-chunk size does not interact with concurrency, but it does with memory — measured

Every paired ratio in the table above is within ±4% of 1.00, in both geometries
and at every N — the granularity the bytes are handed over with does not decide
throughput, latency or CPU under concurrency. The one column it does move is
resident: on the fixture, 19.7-20.4 MB per child at 4 KiB pushes against
22.0-22.8 MB at 64 KiB. The arithmetic of the reply buffer explains it: a push
returns everything it completed (`DecodeStep::pcm`), so the largest single push
materializes its share of the stream's PCM — 64 KiB of a 108 771-byte WEM is 60%
of 3.35 MB ≈ 2.0 MB, which is the gap — while 4 KiB holds 4% of it. The same
arithmetic on stereo (a 17 165-byte WEM, so one 64 KiB push carries the whole
384 KB of PCM) is the 0.6 MB gap there.
[decode-performance.md](decode-performance.md) finding 3 measures the same
effect on 7 MB streams, where one push is also 50% slower.

### 4. Resident per concurrent decode inherits the decode surface's defect — projection

The RSS/child column is the number a pool-sizing decision needs, and it is not a
constant: each child holds its stream's whole synthesized timeline
([decode-performance.md](decode-performance.md) finding 1), so resident per
concurrent decode grows with the stream. For this matrix's two short streams,
where the timeline is small next to the ~9.5 MB process baseline, the tolerance
is 9.8-22.8 MB per child. The measured slope on the 6-channel geometry is
22.7 bytes per decoded frame, so the same decoder on a 100× longer stream
(13.9 M frames) projects to roughly **0.3 GB resident per child** and **~5 GB
for N = 16**; the logical timeline the code implies is 0.67 GB per child, so
even that is a lower bound on what the sessions hold. **That is arithmetic from
the measured slope, not a measurement** — the measured curve, and the code read
behind it, are on the decode performance page.

### 5. Concurrency is not a reason to change the push chunk — measured

The two regimes are equal on every concurrency question (finding 3) and the
64 KiB regime is 2.8% *faster* single-threaded (decode-performance finding 5),
while 4 KiB costs 2-3 MB less resident per child. A caller streaming a long
file should pick a bounded chunk in the tens of kilobytes: it is the shape that
keeps both the framing walk and the reply buffer bounded, and nothing in the
concurrency picture argues for a different one. There is no cap and no
threshold here — the choice is a memory-versus-syscall trade the caller owns.

## Trust boundary

- One machine (Apple M3 Max, 16 cores, 64 GB, macOS 27, rustc 1.96.0), one
  session, one revision of the tree, and no quiet window: the 1-minute load
  average held at 21.7 throughout, which is 5-6 cores' worth of neighbours.
  The paired ratios, CPU per decode and cores busy survive that; the absolute
  rates are what this machine gave under that load.
- Latency is the interval from the first spawn of the batch to that child's
  exit, so it includes process start-up — the decode-stage column is printed
  beside it so the two can be told apart. The p95 is over 7 × N observations per
  cell, so the N = 1 row's p95 is the upper end of 7 samples, not a tail
  estimate; read it as a shape.
- The children are the harness test binary, not a shipping CLI: the shipped
  decode surfaces today are the Rust `DecodeSession` and the C ABI's
  `wem_decoder_push`, and the test binary adds libtest's own start-up to every
  child. That is the process floor finding 2 measures, and it is the *pessimistic*
  side of the comparison for an in-process caller.
- Every child's own decode time and frame count are read from the line it
  prints, and a child whose delivered frame count differs from its container's
  declared count fails the batch (a comparison, not a threshold). A batch whose
  child exits non-zero fails the measurement; it never becomes a fast sample.
- No file under `crates/*/src/` or `src/` was changed for this measurement, and
  the harness's own comparison (`staged replay == session PCM`, bit for bit)
  held in the run recorded on the decode performance page.
- The figure is byte-stable: figure size and DPI pinned, every series sorted by
  N, `savefig(..., metadata={})`, default rcParams otherwise. Three consecutive
  renders of the committed JSON produced
  `sha256 f54f229d1f2ffd705f31aa91b647fac8c2eda437bd9c1523cdebdbd5f2c28fde`,
  and `--check` reports byte-identical against the committed file. A different
  Matplotlib version is expected to change the digest — PNG's only text chunk
  is the `Software` string — so regenerate the figure with the version the plot
  script's docstring names (3.11.2).