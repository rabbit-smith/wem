# Duration curves: memory, CPU and the stage split from 10 s to 10 minutes

**This document is the record for two measurements, taken on two different
bases, of the same question.** The owner asked what happens at *realistic*
input sizes — music of five minutes or less — and whether anything is still
worth optimizing there. The first measurement answered it; two changes then
landed on the one-shot path, and the second measurement re-ran the same matrix
on the new base to see what they did. Both sets are kept: a finding is history,
and the earlier numbers and conclusions are reproduced below with the base they
belong to named on every table.

| set | base commit | when it is the answer |
| --- | --- | --- |
| **earlier** | *Merge commit 'a2e19c7' into codex/library-standards-2* | what the curves were before the one-frame-at-a-time change |
| **current** | *perf(analysis): build the long seed look once per encode, not per channel job* (current `main`) | what the curves are now |

The change between them is **the one-frame-at-a-time change**, *perf(analysis):
hand out one analysis frame at a time from session scratch*, one commit that did
two things to `encode_pcm`: it replaced `selected_windows` (which materializes
the whole frame sequence) with `selected_window_source` + `analyze_window` one
frame at a time, and it made `condition_pcm` return a `Cow`, so the conditioned
rows are *borrowed* when the profile selects no input conditioner.

Reproduce either set with `scripts/measure_duration_curves.py`; it has no
thresholds and no recorded baseline, and nothing in the tree compares against
it. The inputs are byte-identical between the two sets (all twelve generated
WAVs hash the same), so the only difference in the numbers is the code.

## Result — current main

* **Both of the changes did what they were meant to do, and the five-minute
  one-shot peak fell 41%.** At 300 s / 6 ch the one-shot path peaks at
  **2 445 MB** against the earlier base's 4 157 MB. The **materialized frame
  sequence is gone from the one-shot path** — the batch peak no longer contains
  it at all (it still costs 2 664.6 MB over `condition` if you call the
  collecting `selected_windows`, which is why the worker that asks for it still
  shows 3 625.0 MB) — and the **conditioned copy is gone for the 6 ch profile**
  (0.6 MB, from 636 MB: the rows are borrowed, not copied). The 2 ch profile
  *does* select an input conditioner, so its copy remains: 231 MB, unchanged.
* **The peak did not fall as far as the two removed terms, because the
  replacement brought its own whole-input copies, and they are now the largest
  term.** The path that hands out one frame at a time first builds the detector
  stream sets inside `select_modes`, and it holds **two** of them at once while
  the second is assigned over the first: measured at **1 376 MB, 56.3% of the
  five-minute one-shot peak**, against 2 664.6 MB for the frame sequence it
  replaced. So the five-minute one-shot peak is now: detector stream overlap
  1 376 MB (56.3%) + float rows 639 MB (26.2%) + caller PCM 318 MB (13.0%) +
  packets and the container builder 108 MB (4.4%) + conditioned rows 0.6 MB +
  process baseline 2.7 MB.
* **There is still no knee.** All four curves are straight lines. 6 ch one-shot
  is **8.10 MB per audio-second** (`resid 0.05%`, slope drift `1.006x`), 2 ch
  one-shot **4.01** (resid 0.0%, drift `1.00x`), 6 ch streaming
  **21.4 MB + 0.347 per audio-second** (resid 0.9%), 2 ch streaming
  **14.7 MB + 0.126** (resid 1.2%). Ten minutes costs 1.92x five minutes at
  6 ch (whose 600 s samples span 3.8% of their own) and 1.99x at 2 ch.
* **The wall-clock ratio holds to five minutes and past it**: **24.80-25.29x
  real time at 6 ch/44100** and **53.99-56.75x at 2 ch/48000** from 10 s to
  600 s. CPU per audio-second is a constant (6 ch: 117.7-120.2 ms, 44-46% of it
  system; 2 ch: 40.4-42.0 ms, 34-37% system) and the stage mix does not move
  (packing + the two analysis stages = 76.6% at 300 s).
* **The output side is what still grows with duration, and the container
  builder is where it goes.** The container is output-proportional — 15.6 MB at
  five minutes and 31.3 MB at ten, 9.7-9.9% of the input's size — and the streaming
  `finish` adds **5.01x the container** on top of the push-side retention
  (5.02x at ten minutes; 5.85x and 5.20x for 2 ch). A child that runs *nothing
  but* `build_vorbis_wem` over a packet set of the measured shape reaches
  **1.07-1.20x the whole `finish` addition**, so essentially all of it is the
  container builder's own materialization (4.4-4.6x the container) and at most
  about one container's worth (7.3 MB measured, ≤ 15.6 MB bounded) is the
  session's own extra copy.

## Result — the earlier set, as first recorded

Kept verbatim; the two levers it ranked 2 and 3 have since been taken, which is
why the current set exists. Its numbers are the earlier base's and are not
restated as current:

* **Nothing on either curve has a knee.** Peak RSS is a straight line through
  the origin for the one-shot path (**13.8 MB per audio-second at 6 ch/44100**,
  4.64 at 2 ch/48000 — `resid 0.1%` to its own fit) and a much shallower line
  for the streaming path (**23.9 MB + 0.354 MB per audio-second**, `resid
  1.0%`). Wall time is a straight line too, and the encode ratio *holds*:
  **23.5-24.2x real time at 6 ch/44100 and 53.8-55.8x at 2 ch/48000 from 10 s
  to 600 s**, against 25.8x for the 3.161 s fixture. Five minutes is not a
  special size: it is 30x the fixture, encoded 30x slower, byte-identically.
* **The textbook reading of the two paths is right, but the sizes are the
  story.** At five minutes the one-shot path peaks at **4.16 GB** (6 ch) and
  the streaming path at **128 MB** — and of the 4.16 GB, **2.56 GB is the
  materialized frame sequence** (`iter_planned_pcm_windows`), not the copies
  the earlier finding named. The streaming path's ring *is* bounded, but its
  RSS still grows linearly with duration because `finish()` retains and then
  re-copies every emitted packet: **measured 6.7x the container size at the
  peak**, 128 MB at five minutes and 237 MB at ten (the repository's own
  bounded-memory ceiling is 150 MB).
* **The one wall-clock lever is still the rayon partition — and the measured
  fix is to bound the pool, not to remove the feature.** At five minutes the
  parallel build returns **1.19x wall for 2.32x the CPU** at 6 ch, and
  **1.00x for 1.94x the CPU** at 2 ch. Sweeping the pool on the 10 s input
  puts the wall-time optimum at the **channel count** (6 threads for 6 ch,
  2 for 2 ch) with the default 16 threads past it: **424.0 -> 386.7 ms (-9%)
  and 1301 -> 710 ms of CPU (-45%)** at 6 ch, 178.9 -> 175.7 ms and
  316 -> 229 ms at 2 ch. The extra CPU is nearly all *system* time (585 -> 8 ms
  per 10 s encode between the 16-thread and scalar runs), so it is pool
  wake-up and per-thread memory churn, not arithmetic.
* The stage mix is **content-dependent but duration-independent**: Vorbis
  packing plus the two analysis stages are 75-77% of the encode at 10 s, 60 s
  and 300 s alike.

Its verdict items 2 (consume the one-shot frames instead of materializing
them), 3 (stop copying the PCM the caller already owns) and 4 (the streaming
`finish` makes ~5 copies of the output) are addressed by the current set below,
item by item.

## What was measured, and how

| | |
| --- | --- |
| Instrument | `crates/wem-core/tests/duration_curves.rs` (test-only, `#[ignore]`d): one `#[ignore]`d worker per path in a freshly spawned child, plus a parent reporter that reaps it with `wait4` and prints `ru_maxrss` / `ru_utime` / `ru_stime` — the child-process pattern `streaming.rs` already uses, extended from RSS to RSS+CPU |
| Driver | `scripts/measure_duration_curves.py`: owns the matrix, the round ordering, the contamination re-runs and the statistics |
| Stage split | `crates/wem-core/tests/stage_timings.rs`, taken verbatim from `codex/measure-and-investigate` with one additive `#[ignore]`d driver (`stage_timings_duration_report`) that replays a generated input through the same `staged_run`; the fixture report is untouched |
| Inputs | `scripts/generate_duration_curve_inputs.py`: integer fixed-point, no platform math, byte-stable; it **cross-checks itself against `scripts/generate_2ch_long_program.render_pcm16le`** at 2 ch/48000/20 s and refuses to emit if the general renderer stops reducing to it (it did not: `89b916a151ad1d42d313efdc2ae970a2671fee9b5c5b979aeb9982f7b1f85554`) |
| Durations | 10, 30, 60, 120, 300, 600 s, both installed geometries (6 ch/44100 via `fixture`, 2 ch/48000 via `two_channel`) |
| Rounds | `batch` and `stream` measured next to each other inside one round with the order flipped every round; 3 rounds for <= 120 s and 2 for >= 300 s; a point whose slowest wall sample exceeded 1.5x its median was re-run |
| Build | `cargo build --release --features parallel` (the feature is opt-in; it was a default when these numbers were taken) |
| Machine | Apple M3 Max, 16 logical cores, 64 GB, macOS 27.0, rustc 1.96.0 (ac68faa20), LLVM 22.1.2 |

The paths, all reading the *same* generated WAV for their point, so the only
difference between them is where the input lives:

| path | what it does |
| --- | --- |
| `noop` | process baseline, no input |
| `load` | `read_pcm16` + `to_pcm16` only: the caller's PCM resident |
| `batch` | `Encoder::encode_pcm` over that PCM, WAV bytes still held as the CLI holds them |
| `stream` | `StreamSession` fed from a **streaming** WAV reader one 10 s block at a time — the whole input is never resident |
| `rows`, `condition` | phase replays of `encode_pcm`'s own statements (float rows / conditioned rows), each holding what it built |
| `detector`, `source` | phase replays of the two whole-input terms the one-frame-at-a-time change added: the detector stream sets `select_modes` builds, held as `select_modes` holds them; and `selected_window_source` held |
| `windows` | the *collecting* frame sequence (`selected_windows`), which the one-shot path no longer calls |
| `stream_push` | `stream` stopped before `finish`, to attribute the streaming peak |
| `assembly` | `build_vorbis_wem` **alone**, over a synthetic packet set carrying a measured packet count and output size, so the container builder's own copies can be separated from the session's retention |

`detector`, `source` and `assembly` are additive to the earlier set's
instrument, which is otherwise taken unchanged; the two `Cow`-induced
signature changes to the phase replays are mechanical (the rows are bound to a
`let`, which is what `encode_pcm` itself does) and change no measurement.

## (b) Peak RSS, per path and geometry — current main

Min/median MB, `ru_maxrss` of a fresh child. The two encode paths were
measured at all six durations; the reference and phase workers replay the whole
pipeline, so the driver runs them at the reference durations only, which for
this set were 10, 300 and 600 s:

| geometry | path | 10 s | 30 s | 60 s | 120 s | 300 s | 600 s |
| --- | --- | ---: | ---: | ---: | ---: | ---: | ---: |
| 6ch44100 | noop | 2.7 | — | — | — | 2.7 | 2.7 |
| 6ch44100 | load | 13.4 | — | — | — | 320.3 | 637.8 |
| 6ch44100 | rows | 39.1 | — | — | — | 959.8 | 1 912.2 |
| 6ch44100 | condition | 39.7 | — | — | — | 960.4 | 1 913.0 |
| 6ch44100 | **detector** | 85.8 | — | — | — | **2 336.7** | **4 665.3** |
| 6ch44100 | **source** | 86.8 | — | — | — | **2 342.9** | **4 677.6** |
| 6ch44100 | windows | 129.7 | — | — | — | 3 625.0 | 7 242.4 |
| 6ch44100 | **batch** | **92.4** | **254.7** | **498.8** | **985.3** | **2 445.0** | **4 692.9** |
| 6ch44100 | **stream** | **25.0** | **32.7** | **40.8** | **65.1** | **123.6** | **230.7** |
| 2ch48000 | noop | 2.7 | — | — | — | 2.7 | 2.7 |
| 2ch48000 | load | 6.6 | — | — | — | 118.0 | 233.2 |
| 2ch48000 | rows | 22.7 | — | — | — | 355.0 | 700.4 |
| 2ch48000 | condition | 31.1 | — | — | — | 585.9 | 1 161.9 |
| 2ch48000 | **detector** | 42.2 | — | — | — | **931.3** | **1 852.9** |
| 2ch48000 | **source** | 50.9 | — | — | — | **1 167.8** | **2 326.8** |
| 2ch48000 | windows | 66.5 | — | — | — | 1 636.6 | 3 260.4 |
| 2ch48000 | **batch** | **51.2** | **131.4** | **251.5** | **492.4** | **1 214.8** | **2 417.1** |
| 2ch48000 | **stream** | **16.6** | **17.9** | **21.4** | **30.2** | **53.8** | **90.1** |

The 6 ch `batch` cell at 300 s is the **mode of four pooled samples**, not the
`min` the driver prints: three of them (2 443.7, 2 444.5, 2 445.9) agree within
0.1% and the fourth (2 021.8) does not; see the note under the comparison
table. The 600 s cell is the `min` of four pooled samples (4 692.9, 4 720.6,
4 748.4, 4 870.7), which span 3.8%.

The five-minute column is the deliverable. Reading the peaks against the
earlier base:

| path | geometry | earlier base (300 s) | current main (300 s) | change |
| --- | --- | ---: | ---: | ---: |
| one-shot | 6ch44100 | 4 157.4 MB | **2 445.0 MB** | **-41.2%** |
| one-shot | 2ch48000 | 1 407.5 MB | **1 214.8 MB** | **-13.7%** |
| streaming | 6ch44100 | 128.3 MB | **123.6 MB** | -3.7% |
| streaming | 2ch48000 | 55.9 MB | **53.8 MB** | -3.8% |

The streaming path barely moves, as it should: it never materialized the frame
sequence, so neither change was about it.

**The 6 ch 300 s one-shot point is bimodal, and the table quotes the mode.**
Four samples were taken across two passes: 2 443.7, 2 444.5, 2 445.9 and
2 021.8 MB. The three agreeing samples are the value quoted; the fourth is
17% below them and was not reproduced (the second pass took two samples at
that point and both were 2 444-2 446 MB). It is reported rather than dropped
because it is the only point in either set that behaves this way, and because
the phase workers say it cannot be the whole story: the `detector` worker
alone peaks at 2 336.7 MB on the same input, so a 2 021.8 MB *one-shot* peak —
which does strictly more work — is an under-measurement of some allocator
path, not a lower memory requirement.

### Shape

| geometry | path | shape | fit slope | intercept | resid% | slope drift |
| --- | --- | --- | ---: | ---: | ---: | ---: |
| 6ch44100 | batch | linear | 8.099 MB/audio-s | 12.7 MB | 0.05% | 1.006x |
| 6ch44100 | stream | linear | 0.347 MB/audio-s | 21.4 MB | 0.9% | 1.49x |
| 2ch48000 | batch | linear | 4.010 MB/audio-s | 11.1 MB | 0.0% | 1.00x |
| 2ch48000 | stream | linear | 0.126 MB/audio-s | 14.7 MB | 1.2% | 2.39x |

`resid%` is the largest residual to the point set's own least-squares line as a
share of the curve's largest value. The 6 ch one-shot fit uses 2 445.0 MB at
300 s (see above); fitting the unreproduced 2 021.8 MB sample instead gives
`resid 5.7%` and `drift 1.55x`, which is the outlier and not a knee.

**There is no knee anywhere.** Interval slopes for the 6 ch one-shot curve are
+8.115, +8.137, +8.108, +8.109, +8.086 MB/s in duration order — a 0.6% spread
across a 60x duration range. Ten minutes costs 1.92x five minutes on 6 ch
one-shot (4 692.9 / 2 445.0) and 1.99x on 2 ch (2 417.1 / 1 214.8); the 6 ch
figure is 4% under 2x because its 600 s samples (4 692.9-4 871.1 across the two
passes) have a 3.8% spread of their own.

The earlier base's shape verdict is **confirmed, not revised**: both one-shot
curves and both streaming curves are straight lines before and after the
change, and the one-shot slope simply fell (13.82 -> 8.10 MB/audio-s at 6 ch,
4.64 -> 4.01 at 2 ch).

### Where the five-minute one-shot peak comes from — item by item

MB, min RSS, cumulative statements of `encode_pcm`, 300 s (the five-minute
point) and 600 s:

| geometry | dur | baseline | +PCM | +rows | +condition | +detector | +source | +windows | =batch |
| --- | ---: | ---: | ---: | ---: | ---: | ---: | ---: | ---: | ---: |
| 6ch44100 | 300 s | 2.7 | 320.3 | 959.8 | 960.4 | 2 336.7 | 2 342.9 | 3 625.0 | **2 445.0** |
| 6ch44100 | 600 s | 2.7 | 637.8 | 1 912.2 | 1 913.0 | 4 665.3 | 4 677.6 | 7 242.4 | **4 692.9** |
| 2ch48000 | 300 s | 2.7 | 118.0 | 355.0 | 585.9 | 931.3 | 1 167.8 | 1 636.6 | **1 214.8** |
| 2ch48000 | 600 s | 2.7 | 233.2 | 700.4 | 1 161.9 | 1 852.9 | 2 326.8 | 3 260.4 | **2 417.1** |

`+detector` and `+source` are the two whole-input terms the one-frame-at-a-time
change added; `+windows` is the collecting call the one-shot path no longer
makes. The columns are statements of `encode_pcm`, not deltas to be summed — but
here they happen to be, because `batch` peaks last.

The five-minute 6 ch peak, term by term, against the earlier decomposition:

| term | earlier base | current main | share now | status |
| --- | ---: | ---: | ---: | --- |
| materialized frame sequence (`selected_windows`) | 2 556 MB | **0 MB in `batch`** (2 664.6 MB over `condition` if the collecting call is used: `windows` 3 625.0 - `condition` 960.4) | 0% | **gone from the one-shot path** — confirmed. The API still costs it; the shipped path no longer calls it |
| conditioned rows (`condition_pcm`) | 636 MB | **0.6 MB** for 6 ch (`condition` 960.4 - `rows` 959.8) | 0.02% | **gone for 6 ch** — confirmed, the `Cow` is borrowed. **Unchanged for 2 ch** (585.9 - 355.0 = **230.9 MB**): that profile selects an input conditioner, so the `Cow` is `Owned` and the copy is real. The `conditioner` stage time is 0.000 ms for 6 ch and 118.6 ms at 2 ch/300 s, which is the same fact from the other side |
| float rows (`to_float_rows`) | 639 MB | **639.5 MB** (`rows` 959.8 - `load` 320.3) | 26.2% | **unchanged** — the remaining input-proportional cost, and now the second-largest term |
| caller PCM (WAV + `Pcm16`) | 318 MB | **317.6 MB** (`load` 320.3 - `noop` 2.7) | 13.0% | **unchanged** |
| packets + per-frame scratch | 6 MB | **108.3 MB** (`batch` minus everything above) | 4.4% | **changed in composition, and newly measured**: the per-frame scratch is now one frame (the loop reuses it), and what took its place is the container builder's own copies, measured separately at **71.1 MB = 4.56x the 15.6 MB container** by the `assembly` worker |
| **detector stream sets** (`select_modes`) | — | **1 376.3 MB** (`detector` 2 336.7 - `condition` 960.4) | 56.3% | **new, and now the largest term.** `select_modes` calls `detector_pcm_streams` twice into the same binding, so the second set is built while the first is still alive: two whole-input f64 stream sets, each one copy of the source per channel |
| `PlannedWindowSource`'s own copy | — | 635 MB of rows, which the detector peak **hides** | under the above | **new, but not on the peak**: `source` (2 342.9) is within 6.2 MB of `detector` (2 336.7), so the source's copy of the rows is made after the detector sets are freed and lands below the peak they set |

Sum at 300 s / 6 ch: 2.7 + 317.6 + 639.5 + 0.6 + 1 376.3 + 108.3 = **2 445.0 MB**,
which is the measured `batch` peak.

**What this says about the two changes.** They removed 3 192 MB of the 4 157 MB
five-minute peak (the frame sequence and the conditioned copy) and put back
1 376 MB, for a net 1 712 MB. The replacement is not free: handing out one
frame at a time requires the mode sequence first, and building the mode
sequence builds two full detector stream sets. Whether *that* can be done in
one pass — the first call's set is only needed for the pre-EOS quanta, the
second for the rest — is a question for a change, not for this measurement.

## (b′) The earlier set's memory tables, as measured on the earlier base

Kept as recorded. These are the numbers the current set is compared against,
and they were taken on a machine whose 1-minute load average was 79.8 (min) /
206.2 (median) / 232.8 (max) against this set's 18.35 / 24.35 / 99.73.

| geometry | path | 10 s | 30 s | 60 s | 120 s | 300 s | 600 s |
| --- | --- | ---: | ---: | ---: | ---: | ---: | ---: |
| 6ch44100 | noop | 2.7 | 2.7 | 2.7 | 2.7 | 2.7 | 2.7 |
| 6ch44100 | load | 13.4 | 34.6 | 66.3 | 129.8 | 320.4 | 637.9 |
| 6ch44100 | rows | 38.8 | 102.3 | 197.5 | 388.1 | 959.6 | 1 912.1 |
| 6ch44100 | condition | 60.8 | 166.5 | 325.3 | 642.8 | 1 595.3 | 3 183.0 |
| 6ch44100 | windows | 129.4 | 423.3 | 837.6 | 1 666.3 | 4 151.5 | 8 294.7 |
| 6ch44100 | **batch** | **135.6** | **428.8** | **843.3** | **1 671.9** | **4 157.4** | **8 300.8** |
| 6ch44100 | **stream** | **25.8** | **35.4** | **44.8** | **68.8** | **128.3** | **236.7** |
| 2ch48000 | load | 6.7 | 14.4 | 25.9 | 48.9 | 118.0 | 233.3 |
| 2ch48000 | rows | 20.7 | 43.6 | 78.2 | 147.4 | 354.6 | 700.2 |
| 2ch48000 | condition | 30.7 | 67.3 | 124.8 | 240.0 | 585.6 | 1 161.7 |
| 2ch48000 | windows | 58.4 | 149.3 | 287.9 | 566.3 | 1 401.9 | 2 795.2 |
| 2ch48000 | **batch** | **58.3** | **154.3** | **293.8** | **572.1** | **1 407.5** | **2 800.0** |
| 2ch48000 | **stream** | **16.1** | **18.4** | **23.5** | **31.5** | **55.9** | **89.2** |

Its shape table, and its decomposition of the five-minute 6 ch peak:

| geometry | path | shape | fit slope | intercept | resid% | slope drift |
| --- | --- | --- | ---: | ---: | ---: | ---: |
| 6ch44100 | batch | linear | 13.82 MB/audio-s | 9.6 MB | 0.1% | 1.06x |
| 6ch44100 | stream | linear | 0.354 MB/audio-s | 23.9 MB | 1.0% | 1.54x |
| 2ch48000 | batch | linear | 4.64 MB/audio-s | 14.2 MB | 0.1% | 1.03x |
| 2ch48000 | stream | linear | 0.125 MB/audio-s | 15.8 MB | 2.9% | 1.52x |

| term | MB | share | what it was |
| --- | ---: | ---: | --- |
| materialized frame sequence | 2 556 | 61.5% | `selected_windows` -> `iter_planned_pcm_windows` (`windowing.rs:43`), every frame's `Vec<Vec<f64>>` held at once |
| float rows | 639 | 15.4% | `Pcm16::to_float_rows` (`encoder.rs:218`) |
| conditioned rows | 636 | 15.3% | `condition_pcm`'s `pcm.to_vec()` when the profile has no conditioner (`session.rs:246`) |
| caller PCM (WAV + `Pcm16`) | 318 | 7.6% | the file's bytes plus `to_pcm16`'s copy |
| packets + per-frame scratch | 6 | 0.1% | `encode_pcm`'s loop |

Its explanation of the frame sequence's cost — "2x the f64 payload it holds
(the payload is 8D = 1 270 MB at 300 s, because each window's hop is half its
size, so the sequence covers the source twice)" — is **corrected here**: the
f64 payload of the whole source is 8 x 13 230 000 x 6 = **635 MB**, and each
frame's hop is a *quarter* of its block size (`(previous + current) / 4`, the
same rule `recompute_vorbis_fmt_sizes` applies), so the sequence covers the
source *four* times: 4 x 635 MB = 2 540 MB, plus ~125 MB of per-frame scratch
= the 2 665 MB the current `windows` worker measures. The measured 2 556 MB of
the earlier set stands; only the factor in its explanation was wrong, and it
does not change the conclusion that the term dominated the peak.

### The streaming path is bounded in its ring and linear in its output

The session's ring is bounded — that part of the design holds. What grows is
the emitted packets, and the growth is *at `finish`*. Earlier base:

| geometry | dur | `stream_push` (no `finish`) | `stream` | `finish` adds | output (`wem_bytes`) | adds/output |
| --- | ---: | ---: | ---: | ---: | ---: | ---: |
| 6ch44100 | 300 s | 53.9 | 127.5 | +73.5 | 15.6 | 4.70x |
| 6ch44100 | 600 s | 78.2 | 234.2 | +156.0 | 31.3 | 4.99x |
| 2ch48000 | 300 s | 31.7 | 55.8 | +24.1 | 4.5 | 5.36x |
| 2ch48000 | 600 s | 44.8 | 91.8 | +47.0 | 9.0 | 5.22x |

That is re-measured and decomposed in section (e).

## (c) Wall time, CPU and the multiple of real time — current main

Machine load during this run: **1-minute load average min 18.35, median 24.35,
max 99.73 on 16 logical cores** (sibling lanes), with 452-656% of CPU busy at
the round boundaries. This is a quieter machine than the earlier set had
(median load 206.2), so these wall numbers are less contended *and* still
provisional; the CPU numbers and every RSS number are not.

6ch44100 (`wall` is the whole child process, `xRT` is audio seconds over the
encode-side wall measured inside the process):

| path | dur | wall min | wall med | wall max | user med | sys med | xRT(min) | xRT(med) | cores |
| --- | ---: | ---: | ---: | ---: | ---: | ---: | ---: | ---: | ---: |
| batch | 10 s | 0.408 | 0.422 | 0.429 | 0.668 | 0.533 | 24.90 | 24.06 | 2.82 |
| batch | 30 s | 1.214 | 1.218 | 1.219 | 1.969 | 1.562 | 24.86 | 24.79 | 2.90 |
| batch | 60 s | 2.408 | 2.423 | 2.436 | 3.949 | 3.142 | 25.02 | 24.86 | 2.92 |
| batch | 120 s | 4.764 | 4.800 | 4.834 | 7.750 | 6.472 | 25.29 | 25.10 | 2.95 |
| batch | **300 s** | **12.055** | **12.099** | **12.142** | 19.237 | 16.146 | **24.94** | 24.86 | 2.92 |
| batch | 600 s | 24.266 | 24.310 | 24.355 | 38.322 | 32.758 | 24.80 | 24.74 | 2.92 |
| stream | 10 s | 0.429 | 0.431 | 0.439 | 0.682 | 0.535 | 23.66 | 23.61 | 2.81 |
| stream | 300 s | 12.461 | 12.499 | 12.537 | 19.839 | 16.034 | 24.09 | 24.02 | 2.87 |
| stream | 600 s | 24.612 | 25.245 | 25.879 | 39.362 | 31.746 | 24.39 | 23.79 | 2.82 |

2ch48000:

| path | dur | wall min | wall med | xRT(min) | xRT(med) | cores |
| --- | ---: | ---: | ---: | ---: | ---: | ---: |
| batch | 10 s | 0.183 | 0.185 | 56.43 | 55.55 | 2.18 |
| batch | 60 s | 1.066 | 1.092 | 56.64 | 55.33 | 2.29 |
| batch | 300 s | 5.474 | 5.484 | 55.01 | 54.89 | 2.21 |
| batch | 600 s | 10.864 | 11.051 | 55.36 | 54.45 | 2.26 |
| stream | 600 s | 10.958 | 11.105 | 54.81 | 54.09 | 2.25 |

**The ratio holds to five minutes and past it.** The least-contended samples
run **24.80-25.29x real time at 6 ch/44100** and **53.99-56.75x at 2 ch/48000**
from 10 s to 600 s, against the fixture's 25.8x at 3.161 s. The wall-time fit
to a straight line has a **residual of 0.2-0.4%** with interval slopes drifting
**1.03-1.07x**, so the *minimum* is a reproducible statistic. The earlier set
read 23.5-24.2x at 6 ch on a machine at three times this load; the current
numbers are 3-5% higher, which is the load difference, not a speed-up.

**CPU per audio-second is a constant**, which is the cleaner statement of the
same fact (median of the rounds, ms per audio-second):

| geometry | path | 10 s | 30 s | 60 s | 120 s | 300 s | 600 s | sys share |
| --- | --- | ---: | ---: | ---: | ---: | ---: | ---: | ---: |
| 6ch44100 | batch | 120.2 | 117.7 | 118.3 | 118.7 | 117.9 | 118.5 | 44-46% |
| 6ch44100 | stream | 121.6 | 119.9 | 119.7 | 118.8 | 119.6 | 118.5 | 44-45% |
| 2ch48000 | batch | 41.6 | 41.2 | 40.7 | 40.8 | 40.4 | 41.6 | 36-37% |
| 2ch48000 | stream | 41.0 | 40.8 | 41.1 | 42.0 | 42.0 | 41.7 | 34-37% |

For 6 ch, **~45% of the process's CPU is system time** at every duration — the
page-fault and allocator cost of touching 2.4-4.9 GB — against 34-37% at 2 ch
(1.2-2.4 GB). It is linear like everything else, with no threshold, and the
system share is the same as the earlier set's despite the smaller peak, which
is what you would expect: the same number of pages are allocated and freed,
just not all held at once.

## (d) The per-stage split — current main

Medians of 3 staged runs; shares of the staged total, which is `encode_pcm`'s
call sequence replayed through the public API and guarded by a comparison
(`staged_matches_encode_pcm` held on every run).

6ch44100:

| stage | 10 s | 60 s | 300 s |
| --- | ---: | ---: | ---: |
| sched + windows | 10.7% | 10.4% | 10.4% |
| analyze short | 14.2% | 13.1% | 13.1% |
| analyze long | 28.4% | 28.7% | 28.5% |
| vorbis pack | 46.5% | 47.5% | 47.5% |
| pcm decode + conditioner + container | 0.5% | 0.6% | 0.6% |
| **staged total** | **401.5 ms** | **2 372.5 ms** | **11 809.9 ms** |
| `encode_pcm` | 406.0 ms | 2 396.5 ms | 12 197.4 ms |

2ch48000:

| stage | 10 s | 60 s | 300 s |
| --- | ---: | ---: | ---: |
| sched + windows | 8.5% | 8.0% | 8.5% |
| analyze short | 13.0% | 10.7% | 10.9% |
| analyze long | 40.3% | 42.5% | 40.9% |
| vorbis pack | 36.0% | 36.7% | 37.1% |
| pcm decode + conditioner + container | 2.3% | 2.5% | 2.6% |
| **staged total** | **183.2 ms** | **1 100.8 ms** | **5 379.3 ms** |
| `encode_pcm` | 182.9 ms | 1 082.0 ms | 5 500.6 ms |

**The 6 ch proportions do not move** — packing plus the two analysis stages is
75.4%, 76.0% and 76.6% at 10 s, 60 s and 300 s, against the earlier set's
75.0%, 76.0% and 75.0% on the same input, every share within 1.2 points across
a 30x duration range. The stage *shares* are unchanged by the
one-frame-at-a-time change; what changed is the memory those stages hold, not
their time.

**The 2 ch drift the earlier set reported does *not* reproduce at its
magnitude, and the difference is not attributed.** The earlier set saw long
frames rise 7.4 points and packing fall 2.9 points from 10 s to 300 s; here
long frames move +0.6 points (40.3% -> 40.9%) and packing +1.1 (36.0% ->
37.1%). At the same 300 s point the two sets' 2 ch shares differ by up to 3.1
points (long 44.0% vs 40.9%, packing 34.6% vs 37.1%) while their 6 ch shares
agree within 1.2 points. Stage shares are wall clock, and the two sets ran on
machines whose median load differed by 8x; a share difference at that
separation is not evidence about the code, so the 6 ch agreement is the
stronger reading and the 2 ch difference is left open.

Attribution at 300 s (single-threaded re-runs, not additive with the split
above): 6 ch FFT + log curve 2 404.4 ms (135 942 calls), of which the twiddle
recurrence costs 1 788.9 ms by the cost model; MDCT 422.6 ms; frozen-window
rebuild 103.3 ms; transient detector 119.0 ms. The twiddle recurrence is 15.1%
of the 6 ch staged total, against 14.8% on the earlier base — the FFT finding
does not decay with duration and was not touched by this change.

## (e) The output side: what `finish` adds, and where it goes

This section answers the question a decision was waiting on. All cells are MB
of peak RSS above the process baseline, from a reaped child's `ru_maxrss`, one
fresh child per column (min of 3):

| geometry | dur | output | push | stream | `finish` adds | adds/output | `assembly` | `set` | builder | retain | model |
| --- | ---: | ---: | ---: | ---: | ---: | ---: | ---: | ---: | ---: | ---: | ---: |
| 6ch44100 | **300 s** | **15.6** | 44.2 | 122.6 | **78.4** | **5.01x** | 86.8 | 15.6 | **71.1** (4.56x) | 7.3 | 1.11x |
| 6ch44100 | 600 s | 31.3 | 71.6 | 228.6 | 157.0 | 5.02x | 168.2 | 31.3 | 136.9 (4.37x) | 20.0 | 1.07x |
| 2ch48000 | **300 s** | **4.5** | 26.6 | 53.0 | **26.3** | **5.85x** | 31.3 | 4.5 | 26.8 (5.96x) | -0.5 | 1.19x |
| 2ch48000 | 600 s | 9.0 | 40.6 | 87.4 | 46.8 | 5.20x | 55.9 | 9.0 | 47.0 (5.22x) | -0.2 | 1.20x |

**The earlier 4.70-5.36x is re-confirmed**: 5.01x at five minutes on 6 ch and
5.02x at ten, 5.85x and 5.20x on 2 ch. The earlier set's exact figures were
4.70x / 4.99x (6 ch) and 5.36x / 5.22x (2 ch), so the ratio is stable across
both bases to within 9% (the widest gap is 2 ch at five minutes, 5.36 -> 5.85).

**The container is output-proportional and small.** 6 ch: 0.513 MB at 10 s,
1.554 at 30 s, 3.129 at 60 s, 6.258 at 120 s, **15.639 at 300 s**, 31.279 at
600 s. 2 ch: 0.145, 0.445, 0.901, 1.801, **4.502**, 9.002 MB. Both are exactly
linear in duration (2.00x for a 2x input), and the container is **9.7-9.9% of
the input WAV's size** at 6 ch and **7.6-7.8%** at 2 ch, at every duration. The
stream peak is `push + finish`, and both terms grow with the container: `push`
measures 44.2 MB at five minutes and 71.6 MB at ten (the session retains every
emitted packet from the push side, so the retention moves with the output while
the ring does not), and `finish` adds 5.0x the container at both. That is what
makes the stream peak 122.6-123.6 MB at five minutes and 228.6-230.7 MB at ten
across the two passes.

**How many copies of the container exist at the peak of `finish`, and where.**
Five copies of the packet payload are alive at that moment, at five named
sites. Four of them exist because of `finish`; the first is retained from the
push side, which is why it is already inside the `push` column and not inside
the `finish` addition:

| # | site | what it holds |
| --- | --- | --- |
| 1 | `crates/wem-core/src/stream.rs:143` — `audio_packets: Vec<Vec<u8>>`, filled at `stream.rs:351` and `stream.rs:448` | every emitted packet, retained by the session from the push side (counted in `push`, **not** in `finish`'s addition) |
| 2 | `crates/wem-core/src/stream.rs:643` — `packets.extend(pipeline.audio_packets.iter().cloned())` | a second full copy of every audio packet, made at `finish` |
| 3 | `crates/wem-container/src/packets.rs:114-129` — `build_packet_stream` | the concatenated data payload, a third copy, grown by doubling from a small capacity |
| 4 | `crates/wem-container/src/wem.rs:74-78` — `chunks` holds `data_payload.clone()` (and `fmt_payload.clone()`) | a fourth copy |
| 5 | `crates/wem-container/src/wem.rs:79` — `build_riff(&chunks, …)` | the final `wem_bytes`, allocated in full: a fifth copy |

So `finish` builds four copies of the payload (sites 2-5), which is the right
order of magnitude for the measured 5.01x addition once the doubling overshoot
of sites 3 and 5 is counted — but the measurement says most of them are inside
the container builder, not in the session. Sites 3-5 plus the doubling
overshoot of 3 and 5 are measured by the `assembly` worker at **71.1 MB = 4.56x
the container** (6 ch/300 s), 4.37x at 600 s, 5.96x and 5.22x on 2 ch. The
`assembly` child holds one copy of the packet set and then runs nothing but
`build_vorbis_wem`, so `assembly - set` **is** sites 3-5. It reaches 86.8 MB
against the real `finish`'s 78.4 MB (model ratio 1.11x), which bounds rather
than isolates the session's own share: **the extra copy at `stream.rs:616-618`
is at most one container (15.6 MB) and measures 7.3 MB of the 78.4 MB addition
(9%)**. On 2 ch the residual is negative (-0.5 and -0.2 MB), i.e. the synthetic
route peaks slightly above the real one there too, so the same bound applies
with the sign of the residual not meaning anything. The routes agree within
7-20%, which is the check that the synthetic set stands in for the real one; it
is not exact because the synthetic copy has no allocator history to reuse.

**What the one-shot path does differently.** `encode_pcm` ends with
`packets.extend(audio_packets)` (`encoder.rs:708-710`) — a **move**, not a
clone — so **site 2 does not exist on that path**, and sites 1, 3, 4 and 5 are
the identical code (`encode_pcm` calls the same `build_vorbis_wem`). Nothing
else differs. The consequence is visible only as a share: at five minutes the
whole output side costs the one-shot path 108.3 MB of its 2 445.0 MB peak
(4.4%), against 78.4 MB of the streaming path's 122.6 MB peak (64%). The
one-shot path is not better at building the container; it is simply dominated
by its input.

**What the sink decision turns on.** Three measured facts and one piece of
arithmetic:

1. `finish` adds **5.01x the container** at five minutes (6 ch) and 5.02x at
   ten; the container is 15.6 MB and 31.3 MB, both output-proportional and both
   linear in duration.
2. **4.4-4.6x of that is inside `build_vorbis_wem`** (sites 3-5 and their
   growth), measured by a child that does nothing else. Only **at most one
   container — 7.3 MB measured, 15.6 MB bounded — is the session's own
   `cloned()` copy at `stream.rs:616-618`.**
3. The repository's own bounded-memory check asserts a **150 MiB (= 157.3 MB)
   ceiling** on one streaming encode (`crates/wem-core/tests/streaming.rs`,
   `RSS_CEILING_BYTES`). This program sits at **123.6 MB at five minutes
   (inside, 21% under)** and **230.7 MB at ten (47% over)**; the check passes
   because its own signal is far more compressible and produces a ~3.7x
   smaller container, which is the content dependence the earlier set measured.

*Arithmetic on measured terms*: a finish that only stopped cloning the packet
vector would save **7.3-15.6 MB of a 123.6 MB five-minute peak — 6% to 13%**,
and would not bring the ten-minute peak under the ceiling. A finish that wrote
the container out through a bounded-block sink would save **sites 3-5 as well —
about 4.4x the container, 71 MB at five minutes and 137 MB at ten** — taking
the stream peak to roughly **52 MB and 92 MB**, which is under the ceiling at
both durations. It can be done without knowing the size in advance: the session
already retains every packet, so `dw_data_payload_size` and
`nAvgBytesPerSec` are sums over what it holds (arithmetic, no copies), and the
fmt chunk can be written with final values before the data chunk streams out.
The C ABI already declares that delivery — `include/wem.h`, "MEMORY OWNERSHIP":
*"All output flows through the write callback in bounded blocks"*, and
`wem_session_finish` already calls it (`crates/wem-capi/src/lib.rs:784`,
`emit_bytes(&result.data, …)`).

**So the decision is not "sink or no sink" — the sink exists and is documented.
The decision is whether the *kernel* honours it.** As a signature change alone
(add a sink parameter to `StreamSession::finish`, keep building the container)
the measured saving is at most 13% and the interface change is not worth it.
As a streaming container writer behind the callback `include/wem.h` already
declares, the saving is ~58%, it removes the only remaining duration-linear
term on the streaming path, and it makes an existing documented promise true
rather than adding a new one. The two are different changes with a 4.5x
difference in payoff, and the number that separates them is the 4.56x the
`assembly` worker measures.

## (f) Verdict on current main

1. **The one-shot memory lever the earlier set ranked 2 is taken and
   confirmed.** 4 157 -> 2 445 MB at five minutes 6 ch (-41%); the materialized
   frame sequence is no longer in the peak. It did not fall to the ~1 600 MB
   the earlier set projected, because the replacement brought a 1 376 MB
   detector-stream term that no measurement had named before this one.
2. **The copy lever (ranked 3) is taken for one geometry only.** The 6 ch
   conditioned copy is gone (636 -> 0.6 MB); the 2 ch one is unchanged at
   230.9 MB because that profile selects an input conditioner. The caller-PCM
   term (317.6 MB) is untouched: it is still the file's bytes plus a copy.
3. **The new largest term is the detector stream overlap** — two whole-input
   f64 stream sets alive at once inside `select_modes`, 1 376 MB at five
   minutes 6 ch (56.3%). It is bigger than the conditioned copy it replaced and
   it is the obvious next thing to look at, but nothing here says it can be
   removed: it is a question about `select_modes`'s two-pass structure.
4. **The output side (ranked 4) is re-confirmed and now decomposed.** `finish`
   adds 5.01x the container at five minutes; 4.56x of that is the container
   builder's own materialization, and the session's clone is at most one
   container. See (e) for what that does and does not justify.
5. **Nothing on either curve has a knee, at either base.** Both one-shot curves
   and both streaming curves are straight lines from 10 s to 600 s, with the
   6 ch one-shot curve's interval slopes spanning 0.6%. Five minutes is still
   not a special size.
6. **Not worth changing at these sizes.** The 600 s one-shot encode is 24.31 s
   (median) and 24.80x real time; the stage mix is unchanged from the earlier
   base; the fixture's 25.8x is not flattered by being short. The FFT twiddle
   recurrence is still 15.1% of the split and remains the only arithmetic
   change with a real number behind it.

## Verification

Nothing under `crates/*/src/` changed in this lane: the two Rust files are
integration-test targets (`#[ignore]`d harnesses), and the two Python scripts
are measurement tools. `git status` on the lane's tree shows exactly the five
deliverables plus `scripts/AGENTS.md`, and `corpus/` is untouched. Byte
exactness was re-established on the final tree (tails in the lane report):

Every command below was re-pointed on 2026-09-22 at the target and module names
the test consolidation gave them, and each one still selects what its result
column records. The workspace row is the exception: the consolidation merged 40
targets into 27, so a selection preserving its counts no longer exists and its
numbers are re-taken on today's tree (283 passed / 18 ignored were that tree's).

| check | result |
| --- | --- |
| `make wem-bytes` | ok, 1 test, 0.13 s |
| `cargo test -p wem-core --test encoder reference_bytes` | 9 passed, 0 failed |
| `cargo test -p wem-core --test frame_pipeline_parity` (with `PYTHON` set to the shared venv's interpreter) | 1 passed, 0 failed, 35.29 s |
| `cargo test -p wem-vorbis --test vorbis_codec oracle_values` | 9 passed, 0 failed |
| `cargo test --workspace --all-targets --no-fail-fast` | exit 0: 27 targets, 315 passed, 0 failed, 19 ignored (re-taken; 40/283/18 on the tree this lane measured) |
| `make test-fast` | 104 tests, OK |
| `make fuzz-parity` | exit 0 |
| `make 2ch-stress` | 1 test, OK |
| `make 2ch-long` | 1 test, OK |
| `cargo fmt --all --check` | exit 0 |
| `cargo clippy --workspace --all-targets -- -D warnings` | exit 0 |
| `make lint` (ruff + mypy) | exit 0: "All checks passed!", 77 source files |

The 18 ignored in the lane's run were the two `#[ignore]`d harnesses themselves
plus the pre-existing ones: 12 in `duration_curves.rs`, 2 in `stage_timings.rs`,
4 in `streaming.rs`. No target failed and no test was filtered out to get there.
The count is 19 on today's tree, the extra one being
`crates/wem-core/tests/concurrency_worker.rs`, the instrument the parallelism
change added.

Two things about the harness itself are worth recording, because both were
needed to make the taken instrument land:

* It is `#[ignore]`d and test-only, so it does not run in the suite; but
  `make rust-lint` (`-D warnings`) rejected six `doc_overindented_list_items`
  hits in the taken module doc, and `cargo fmt --all --check` rejected the
  taken file. Both were fixed (comments and whitespace only), which is why the
  instrument differs from `codex/duration-curves` in lines the earlier lane
  wrote. The measurement workers' logic is untouched apart from the two `Cow`
  adaptations and the three additive workers listed under "What was measured".
  The numbers in this document were taken before that reformatting; comments
  and whitespace cannot change RSS or timing, and no measurement was re-run for
  it.
* The two `Cow`-induced signature changes to the phase replays are mechanical
  and make the replay *more* faithful, not less: `condition_pcm` borrows the
  rows when the profile selects no conditioner, so the replay has to bind them
  to a `let` — which is exactly what `encode_pcm` does with its own `pcm_rows`.

`frame_pipeline_parity` needs the native extension importable from the
worktree's `src/wwise_wem/`; the lane built it with the shared venv's `maturin`
into a gitignored wheel directory and extracted `_core.abi3.so` there, so
nothing outside this worktree was installed or replaced.

## Trust boundary

* **The current set's wall-clock numbers are provisional**, though far less so
  than the earlier set's. The 1-minute load average across point boundaries was
  18.35 (min) / 24.35 (median) / 99.73 (max) on 16 logical cores, against
  79.8 / 206.2 / 232.8 for the earlier set. The reported wall statistic is the
  **minimum**, whose fit to a straight line has a residual of 0.2-0.4% and
  whose interval slopes drift 1.03-1.07x. The wall spread of a point reached
  42% of its median on the 2 ch/120 s points (a contaminated round, re-run);
  the RSS spread on the same points is the control.
* **RSS stability.** The earlier set's RSS spread was a median of 0.67% of the
  median value. That **held and improved for the one-shot path**: the median
  spread across the current set's one-shot points is **0.25%**, and every
  6 ch/2 ch one-shot point except one is inside 0.5%. It did **not** hold
  everywhere: pooled across both passes the 6 ch/300 s one-shot point spans
  17.35% (2 021.8 to 2 445.9 MB across four samples) and is bimodal, and the
  600 s one-shot point spans 3.8% (4 692.9 to 4 871.1). The streaming points
  are noisier in absolute terms — 2 ch/60 s spans 11.7% — because at 20-30 MB
  the ring and one 10 s block are being touched for the first time.
* The 6 ch/300 s one-shot value quoted throughout is the **mode of four
  samples** (three within 0.1% of each other), not the `min` the driver prints.
  The deviating sample is reported in (b); the walker that re-runs the driver
  verbatim will see `min = 2 021.8 MB` on a run that reproduces it.
* The RSS numbers, the CPU numbers and the stage shares are load-independent
  and are the ones to act on.
* The stage split is wall clock around shipped calls and inherits the rayon
  partition: `analyze_long` is a wall-clock figure over a per-channel
  partition, while the attribution block is single-threaded by construction.
  They are not additive and are not presented as one number.
* The twiddle entry is a **cost model** — it replays the recurrence, it does
  not prove what removing it would save.
* **The `assembly` worker measures a model, and the model is checked, not
  assumed.** It builds a synthetic packet set with the *measured* packet count
  and total payload, but a uniform per-packet size and a self-consistent
  mode-bit/`dw_total_pcm_frames` pair (the container builder validates that
  relationship, and neither value takes part in what it allocates). Its result
  lands within 7-20% of the real `finish` addition by the other route; the
  residual is the session's own copy, so that copy is **bounded, not
  isolated**.
* *Arithmetic on measured terms* marks the two projections in (e): they
  subtract measured terms from a measured peak and assume the change removes
  exactly those terms. They are not measurements of changed code.
* The `stream_push` diagnostic is one to two runs per point; the paired
  `stream` column is the main run's minimum.
* One machine, one build configuration (release, `parallel` on), one signal
  generator. The generator's cross-check against the existing 2 ch program is
  a byte-for-byte equality and the generated inputs are byte-identical between
  the two sets (all twelve digests match), but the *content* is not the
  fixture, and the fixture's own stage shares are quoted from the earlier
  finding, not re-derived here.
* The two sets were taken on different machine load and, for the 300 s one-shot
  point, with different reproducibility. Every conclusion drawn from the
  comparison is drawn from RSS or CPU, never from wall time.