# Decode performance: the measured split, and the memory the session keeps

**Result.** The release build decodes the tracked paired-build fixture
(`tests/fixtures/reference.wem`, 6 ch/44100, 139 398 frames = 3.161 s) in a
median **30.4 ms** — **104× real time** — and a 1 s stereo WEM in 2.7-9.1 ms
(110-374×). The time is not spread evenly: **the residue stage is 56.8% of it
and the overlap-add another 22.6%**; the rest is the floor envelope, the floor
decode, the one-time setup and the interleave, none of them above 7%.

The headline measurement of this lane is not throughput. Decode expands: a WEM
becomes 24-31× its size in f32 PCM, so the question a caller has to be able to
answer is *what does a live session cost while the stream grows?*
`scripts/measure_decode_perf.py` measures that as **peak RSS against stream
length, one child process per point reading that child's own `wait4` rusage**,
and the curve **rises**: 22.89 MB at 3.161 s, 233.16 MB at 202.3 s — **10.19×
for a 64× longer stream**, where the surface's contract says the curve is flat.

That is not a performance characteristic; it is a **contract defect**.
`SynthesisOla` never releases the timeline it synthesizes into
(`crates/wem-analysis/src/dsp/transform.rs:711-716` grows `samples` and nothing
drains it), so a live session holds **8 bytes per decoded sample per channel —
twice the f32 PCM it hands the caller** — for as long as it lives.
`crates/wem-core/src/decoder.rs:87-102` states the opposite ("Nothing
proportional to the stream length is held") and
[`../reference/decoding.md`](../reference/decoding.md) states it as the design
decision ("nothing proportional to the stream length"). See
[finding 1](#1-the-overlap-add-retains-the-whole-timeline--contract-defect-the-headline).

**Fixed, and re-measured.** Both mechanisms were repaired in `f64c871` and the
curve was taken again with the same tool: **256×** the stream now grows peak RSS
**1.68×** (10.76 → 18.04 MB, 0.17 bytes/frame over four points), where 64× grew
it 10.19× before, and the whole-WEM-push penalty fell from +51.0% to +0.6%. The
before curve below is kept as recorded on its own base; the after curve, the
fixes and the release frontier they turn on are in
[the fix](#the-fix-and-what-it-measured). The two claims quoted above are true
again.

This document is the evidence record. It is a measurement, not a budget: there
is no threshold here, and `scripts/measure_decode_perf.py` has none either (see
[Trust boundary](#trust-boundary)).

## What was measured, and how

Reproduce with:

```sh
cargo build --release --manifest-path crates/Cargo.toml -p wem-core
python3 scripts/measure_decode_perf.py            # numbers + machine identity
```

Three things are measured, all of them from outside the kernel:

1. **Every tracked WEM** — the fixture and the eight 2-channel corpus files —
   through one **child process per repetition**: the `decode_process_point`
   entry of `crates/wem-core/tests/decode_stage_timings.rs`, spawned directly
   (not through cargo) and reaped with `os.wait4`, so each sample carries the
   decode process's own exit instant, its own `ru_utime`/`ru_stime` and its own
   `ru_maxrss`. The child pushes the WEM in bounded reads and discards every
   sample it receives, so nothing it holds grows with the stream.
2. **The kernel's per-stage split for the fixture**, from the
   `decode_stage_timings_report` entry of the same file — an `#[ignore]`d,
   test-only harness (driven with `--ignored --nocapture`). It replays the
   session's call sequence through the **public** kernel API
   (`parse_chunks`, `parse_setup` + the compiled carrier's codebooks/looks/
   windows, `parse_audio_header`, `decode_floors_for_packet`,
   `decode_residue_coeffs`, `apply_mapping_coupling_inverse`, `floor1_unwrap`,
   `floor1_curve_from_posts`, `SynthesisOla::push`/`finish`) with
   `std::time::Instant` around each existing call. No shipping code carries
   instrumentation for it.
3. **Peak RSS against stream length**, one child process per point, over
   streams generated deterministically from the tracked fixture — its own
   PCM16 frames repeated N times and encoded by the release encoder binary into
   the gitignored `crates/target/decode-perf/` tree. Nothing under the ignored
   `corpus/` is read by the harness, the driver or the figure, and no number
   here depends on it.

The harness's one assertion is a **comparison**, not a threshold: the staged
replay's interleaved f32 PCM must equal what `DecodeSession` delivers, **bit
for bit** (`f32::to_bits`), both when the session is fed the WEM in one chunk
and when it is fed in 64 KiB chunks. It held for every repetition (`yes (every
repetition)`, n = 9). Remove that guard and every number in the split becomes
unfalsifiable.

### Machine identity (numbers are meaningless without it)

| | |
| --- | --- |
| CPU | Apple M3 Max, 16 logical cores (12 P + 4 E) |
| Memory | 64 GB (68 719 476 736 bytes) |
| OS | macOS 27.0, Darwin 27.0.0, arm64 |
| Toolchain | rustc 1.96.0 (ac68faa20), host aarch64-apple-darwin, LLVM 22.1.2 |
| Python | 3.14.6 |
| Build | `cargo build --release --manifest-path crates/Cargo.toml -p wem-core`, default features |
| Load average at start | 24.93, 21.43, 28.42 (1, 5, 15 min) |

The machine was **busy** for the whole record (1-minute load 24.7-28.4 on 16
cores, sampled before and after every series and printed beside it). Every
wall-clock number below is therefore an upper bound on a quiet machine; the
resident figures are far less load-sensitive, and their per-point spreads are
0.7-8.7% (printed with each point in the table below).

## The measured split (fixture, release, median of 9)

| Stage | Call site | median ms | share |
| --- | --- | ---: | ---: |
| Container framing (`parse_chunks` + the packet walk) | `crates/wem-core/src/decoder.rs:246-294`, `:320-365` | 0.008 | 0.03% |
| Setup: setup packet, carrier, codebooks, looks, windows, OLA | `crates/wem-core/src/decoder.rs:658-786` | 1.188 | 4.41% |
| Audio packet header | `crates/wem-vorbis/src/packet_decoder.rs:173` | 0.017 | 0.06% |
| Floor decode (per packet) | `crates/wem-vorbis/src/packet_decoder.rs:245` | 1.229 | 4.56% |
| Residue coefficients (per submap) | `crates/wem-vorbis/src/residue.rs:1283` | 15.319 | 56.80% |
| Coupling: dirty flags + mapping0 inverse | `packet_decoder.rs:437`, `packet_encoder.rs:518` | 0.008 | 0.03% |
| Floor envelope (`postlist` + unwrap + curve + multiply) | `floor.rs:395`, `:470`, `:741` | 1.813 | 6.72% |
| Overlap-add (`SynthesisOla::push`, 6 channels/block) | `crates/wem-analysis/src/dsp/transform.rs:682` | 6.089 | 22.58% |
| Interleave and clamp (`append_completed`) | `crates/wem-core/src/decoder.rs:477` | 0.985 | 3.65% |
| **staged total** (sum of the above) | | **26.968** | |
| **session, one push of the whole WEM** | `DecodeSession::push_bytes` | **29.252** | |
| ↳ of which `finish` | `DecodeSession::finish` | 0.047 | |
| **session, 64 KiB pushes** | `DecodeSession::push_bytes` | **28.434** | |

205 audio packets, 139 398 frames, 108 771 WEM bytes.

`staged_total` (26.968 ms) is **2.28 ms below** the session's own one-chunk
total (29.252 ms), and that difference is the shipped framing, not a stage the
replay omits: `decode_stage_timings.rs` reproduces the container walk from
`parse_chunks` over a whole buffer, while `ContainerStream::commit_packet`
(`decoder.rs:357-365`) drains each consumed packet from the front of the
*pending* buffer, which moves every unconsumed byte behind it. The chunked
session run measures exactly that: **feeding the same WEM in 64 KiB chunks is
0.82 ms (2.8%) faster than feeding it in one 108 771-byte push** — the smaller
the pending buffer, the less there is to move per packet (finding 3).

## Every tracked WEM (9 child processes each)

| Case | Geometry | Frames | Audio s | WEM bytes | median ms | p95 ms | spread ms | real time | median peak RSS MB | child CPU ms |
| --- | --- | ---: | ---: | ---: | ---: | ---: | ---: | ---: | ---: | ---: |
| `fixture` | 6/44100 | 139 398 | 3.161 | 108 771 | 30.388 | 31.195 | 1.984 | 104.0× | 22.32 | 32.98 |
| `dc_opposed` | 2/48000 | 48 000 | 1.000 | 1 423 | 2.883 | 3.003 | 0.216 | 346.9× | 9.70 | 5.40 |
| `impulse` | 2/48000 | 48 000 | 1.000 | 2 169 | 3.042 | 3.097 | 0.203 | 328.7× | 9.68 | 5.47 |
| `silence` | 2/48000 | 48 000 | 1.000 | 458 | 2.677 | 2.824 | 0.269 | 373.6× | 9.60 | 5.03 |
| `stereo_noise` | 2/48000 | 48 000 | 1.000 | 17 165 | 5.765 | 5.973 | 0.491 | 173.5× | 10.04 | 8.35 |
| `tone_high` | 2/48000 | 48 000 | 1.000 | 4 829 | 3.634 | 3.734 | 0.279 | 275.2× | 10.12 | 5.93 |
| `tone_low` | 2/48000 | 48 000 | 1.000 | 3 572 | 3.459 | 3.572 | 0.278 | 289.1× | 10.01 | 5.91 |
| `channel_bursts` | 2/48000 | 48 000 | 1.000 | 35 172 | 9.079 | 9.324 | 1.350 | 110.1× | 9.90 | 11.46 |
| `tail_change` | 2/48000 | 48 000 | 1.000 | 4 909 | 3.706 | 3.837 | 0.242 | 269.8× | 9.80 | 6.10 |

Two things this table already shows. **The cost tracks the packet payload, not
the audio**: `stereo_noise` (17 KB of WEM for 1 s) takes 5.77 ms and
`channel_bursts` (35 KB) 9.08 ms, while `silence` (458 bytes) takes 2.68 ms —
so the residue work, which is what the extra bytes carry, is the decode's cost
centre, exactly as the fixture split says. And **a process baseline of about
9.5 MB** sits under every stereo case: the test binary plus the compiled
carrier's tables, which is what a 458-byte WEM's 9.60 MB peak is made of.

## The headline: peak RSS against stream length

Generated points are the fixture's own PCM16 frames repeated N times
(`tests/fixtures/input.wav` → WAV → the release encoder), each decoded by a
fresh child that discards every delivered sample. Median of 3 children per
point:

| Point | Frames | Audio s | WEM MB | median peak RSS MB | min..max MB | child total ms |
| ---: | ---: | ---: | ---: | ---: | ---: | ---: |
| 1× | 139 398 | 3.161 | 0.109 | 22.89 | 22.02..23.61 | 29.94 |
| 2× | 278 796 | 6.322 | 0.216 | 39.81 | 39.63..43.09 | 58.50 |
| 4× | 557 592 | 12.644 | 0.436 | 52.66 | 50.50..52.71 | 113.11 |
| 8× | 1 115 184 | 25.288 | 0.869 | 53.00 | 52.90..57.21 | 222.03 |
| 16× | 2 230 368 | 50.575 | 1.748 | 80.07 | 78.38..81.67 | 439.29 |
| 32× | 4 460 736 | 101.150 | 3.493 | 128.84 | 128.65..130.81 | 873.39 |
| 64× | 8 921 472 | 202.301 | 6.990 | **233.16** | 232.18..233.85 | 1729.18 |

- **The stream grows 64.0×; the peak RSS grows 10.19×.** A flat curve is
  1.00×. Least-squares slope over all seven points: **22.70 bytes per frame**
  (3.78 bytes per decoded f32 sample; 6 channels → 3.78 bytes per sample per
  channel).
- **The rise is not uniform, and the record says so.** Steps between adjacent
  points: +16.92 (1×→2×), +12.85 (2×→4×), **+0.34 (4×→8×)**, +27.07 (8×→16×),
  +48.77 (16×→32×), +104.32 MB (32×→64×). The 4×→8× plateau is inside the
  per-point spread (4.31 MB at 8×, the largest of the seven), so three children
  per point do not resolve the shape there; the steps above 8× are 6-60× the
  spreads and do not depend on that resolution. The endpoint comparison — the
  number this lane's headline rests on — is 22.89 against 233.16 MB.
- **The mechanism is not the caller.** The child discards every sample
  (`step.pcm` is dropped each push) so the delivered PCM never accumulates, and
  the input is read in 64 KiB chunks so the WEM bytes do not either. The only
  stream-proportional state a session keeps is
  `Codec::ola` (`decoder.rs:770-771`): one `SynthesisOla` per channel whose
  `samples` vector only ever grows.
- **The slope is the right order for that mechanism, and the measured figure
  understates it.** Logical retention is 8 bytes × channels × frames: 53.5 MB at
  the 8× point and 428.2 MB at the 64× point. The measured resident figures are
  modulated by two allocator-level effects — growing a `Vec` by reallocation
  briefly holds the old and the new buffer at once (which is why the 4× point's
  43.7 MB above baseline exceeds its 26.8 MB logical timeline), while macOS
  reclaims pages of a buffer that is appended to once and never re-read (which
  is why the 64× point's 223 MB is 52% of its 428 MB). **Resident is therefore a
  lower bound on what the session holds at length, and an over-count at small
  sizes; the fitted slope across all seven points is the honest single
  number.**
- **The same mechanism in the other geometry.** A separate 2ch/48000 probe on
  `tests/data/2ch-reference/stereo_noise.wem` (3 children each point): 10.06 MB
  at 48 000 frames, 25.05 MB at 768 000, 35.78 MB at 3 072 000 — a slope of
  **8.51 bytes per frame** against a logical 16 bytes per frame (2 channels ×
  8 bytes), the same ~53% resident share. The slope scales with the channel
  count, which is what a per-channel timeline predicts and what a fixed
  overhead would not.

### What the session's memory is supposed to be

- `crates/wem-core/src/decoder.rs:87-102` ("Incrementality and memory"): *"Data
  payload bytes that arrive after that are consumed as they arrive … the
  session retains one block awaiting its follower's mode (bounded by the
  profile's long block size) plus the bytes of the packet it is currently
  reading. **Nothing proportional to the stream length is held**"* — with one
  stated exception, a container that puts `data` before `fmt `, which is not the
  shape of any of the nine tracked WEMs (each walks `fmt ` then `data`, checked
  from the files themselves).
- `docs/reference/decoding.md`: *"a live decode session holds only the
  bytes of a not-yet-complete packet (capped by the container's
  `u_max_packet_size`), one block of OLA state and scratch — **nothing
  proportional to the stream length**"*, which is the "streaming only"
  decision the whole decode surface rests on.

Both were false as written when this page was recorded, and the measurement in
this section is what falsified them; the code read in finding 1 says why. They
are true again as of `f64c871` — see
[the fix](#the-fix-and-what-it-measured).

## Findings

### 1. The overlap-add retains the whole timeline — contract defect (the headline)

**Where.** `crates/wem-analysis/src/dsp/transform.rs:611-629` (`SynthesisOla`)
and `:682-731` (`push`). Every push:

```rust
let position = self.base + self.origin - (n / 2) as usize;
let end = position + n as usize;
if self.samples.len() < end {
    self.samples.resize(end, 0.0f64);
}
```

`base` advances by `(n + following_size) / 4` per block, so `end` — and with it
`samples.len()` — grows for every block of the stream. `pcm()`
(`transform.rs:666`) hands out `samples[origin..origin + completed]`, `finish`
(`:733`) releases the tail, and **nothing anywhere truncates, drains or reuses
the front**. The session creates one such state per channel for its whole life
(`decoder.rs:770-771`) and pushes every block into it
(`decoder.rs:420-450`, `:453-472`).

**Cost.** 8 bytes per decoded sample per channel, i.e. **twice the f32 PCM the
caller is handed**, retained until the session is dropped. The measured peak
RSS is what that costs at the process level: 22.89 MB → 233.16 MB over the
curve above (finding 1's curve), and the brief's own scale — a 46.7 MB WEM that
decodes to ~1.4 GB of f32 — implies roughly **2.8 GB** of retained timeline for
a 2-channel stream.

**Why it matters more than a number.** The decode surface was designed around
D3, "streaming only": there is no one-shot collector and no profile argument, on
the explicit promise that memory is bounded by one packet and one block. A
caller that streams a long file through the C ABI (`wem_decoder_push` in a
loop) or the Rust session gets that promise in the input direction and loses it
in the output direction. The failure is silent: the samples are correct, the
delivered frames are exactly `dw_total_pcm_frames`, and only the process's
resident set shows it.

**The tree already has the check this surface is missing, on the other
direction.** `crates/wem-core/tests/streaming.rs:205-222` asserts a peak-RSS
ceiling for the *encoding* session (150 MB while driving a 600 s stream whose
input alone is ~310 MB of i16 PCM, and 100 MB for the one-shot batch path) and
reads it from a forked child's `wait4().ru_maxrss` — the same child-process
measurement this page uses. Nothing on the decode side asserts anything about
memory: `grep -rl 'maxrss\|ru_maxrss\|wait4' crates/*/tests/` finds
`streaming.rs`, the duration-curve driver and this lane's harness, and no decode
suite. So this is not a case of a bound that was checked and held; it is a bound
that was documented and never measured.

**Remedy (for the lane that owns `wem-analysis`).** The overlap-add does not
need history: a block's window support reaches back only to the previous
block's centre, so `SynthesisOla` can release the finalized prefix and rebase
its own indices (`origin`, `completed`, `base`, `support_end`) — a ring buffer
over the last two blocks' spans, or a `drain` plus a rebase, both local to this
struct. Byte-exactness is immediate: the values, their order and every rounding
point are unchanged, only where they live. Prove it with the comparison this
lane's harness already makes (`staged replay == session PCM`, bit for bit) plus
`crates/wem-core/tests/decode_reference_wem.rs`, which pins the reference
WEM's PCM, length and chunk-boundary invariance.

### 2. Where the decode's time goes: residue 56.8%, overlap-add 22.6% — observation

The split above is the answer for the fixture. The residue stage
(`decode_residue_coeffs`, one call per submap per packet) is **15.32 ms of
26.97 ms**, and the overlap-add (`SynthesisOla::push`, six channels per block)
**6.09 ms**; the floor envelope (1.81 ms), the floor decode (1.23 ms), the
one-time setup (1.19 ms) and the interleave (0.99 ms) follow, and the framing
walk (0.008 ms) and coupling (0.008 ms) are free.

This is reported, not proposed. The encode side's warning applies here for the
same reason: `Codebook::best_vq`'s comparison is a plain `f64` L2 sum whose
accumulation order decides the argmin on ties, and the residue path is where
that book is read — any reformulation changes the sum and therefore could move
samples. A decode-optimization lane needs its own byte-exactness argument
first.

Two smaller observations from the same table:

- **Setup is a fixed per-session cost.** `open_codec` (1.19 ms here — the
  carrier lookup, the codebook load, the MDCT look clones and the frozen-window
  handles) is 4.4% of a 3.16 s decode, but **20-44% of the entire decode of a
  1 s stereo file** (2.68-5.77 ms total). A caller decoding many short WEMs
  pays it per WEM.
- **The overlap-add is where the memory defect and the time cost meet.** It is
  the second-largest stage (22.6%): 1230 channel-block calls over the fixture
  (205 blocks × 6 channels), 4.95 µs each. Those calls are also what grows the
  buffer finding 1 is about, so a remedy that bounds the timeline touches the
  same code that spends a fifth of the decode's time.

### 3. The container walk is quadratic in the pending buffer — algorithmic, act

**Where.** `ContainerStream::commit_packet` (`crates/wem-core/src/decoder.rs:357-365`):

```rust
self.pending.drain(..2 + packet.payload.len());
```

`pending` holds every byte pushed but not yet consumed. Draining from the front
of a `Vec` moves all remaining bytes down, once per packet, so one push of `B`
bytes costs `O(packets × B)` in `memmove` — quadratic in the push size, linear
in the stream.

**Cost, measured** (same 6ch fixture streams as the curve, three children each,
64 KiB pushes against one push of the whole file):

| Point | WEM MB | 64 KiB pushes | one push | difference | RSS 64 KiB | RSS one push |
| ---: | ---: | ---: | ---: | ---: | ---: | ---: |
| 1× | 0.109 | 28.67 ms | 28.57 ms | −0.10 ms (−0.3%) | 21.94 MB | 22.28 MB |
| 8× | 0.869 | 212.66 ms | 227.66 ms | **+15.00 ms (+7.1%)** | 52.35 MB | 63.42 MB |
| 64× | 6.990 | 1693.13 ms | 2538.48 ms | **+845.35 ms (+49.9%)** | 233.01 MB | 372.62 MB |

The excess grows ~57× when the push grows 8× (8² ≈ 64): that is the quadratic
term, and at 7 MB it is half the decode again. The resident difference is a
second, independent push-size effect: `DecodeStep::pcm` carries everything one
call completed, so a single push of a whole stream materializes that stream's
whole PCM in the reply buffer before the caller ever sees it (214 MB of f32 at
the 64× point).

The kernel's own contract already says the right thing about *chunking* — "any
chunking of the same WEM bytes emits the same PCM" (`decoder.rs:541-548`, and
re-verified on this material by the harness's `chunked_matches_session`
comparison, n = 9). What is missing is that chunking is also what keeps both
memory and the framing walk bounded: **a caller that pushes the whole file at
once pays 50% more time and 1.6× the resident memory than one that pushes
64 KiB at a time** on a 7 MB WEM. A `VecDeque`, or draining only when the
consumed prefix is worth the move, removes the time term; the reply-buffer term
is the API shape and belongs in the C ABI's documentation rather than in a
kernel change.

### 4. Refuted: superlinear decode time in stream length

The curve's `total_ms` column is linear to within 10%: 29.94 → 1729.18 ms for a
64× longer stream is **57.8×**, and 222.03 ms at 8× is 7.4× the 1× figure. There
is no quadratic term in the *stream*: the residue, floor, envelope and OLA work
are all per-packet or per-sample, and the one term that is quadratic —
finding 3 — is quadratic in the push size, not in the stream. A decoder fed in
bounded chunks therefore scales linearly in time and (finding 1) **linearly in
memory**, which is the defect.

### 5. Chunking costs nothing in time — measured

The harness runs the fixture twice per repetition, once as a single push and
once in 64 KiB chunks, and compares both against the staged replay. Time:
29.252 ms (one push) against 28.434 ms (chunked) — the chunked path is **2.8%
faster**, not slower, because of finding 3's drain. Chunk boundaries changed
nothing about the samples in any of the 9 repetitions. So there is no
time-versus-memory trade to make here: bounded pushes are both.

## The fix, and what it measured

Both defects were repaired in `f64c871`, and the curve was taken again with
`scripts/measure_decode_perf.py`, the same tool and the same command that found
them. The before column is this page's own record, on its base; the after column
is the fixed tree.

| point | before (base `b3a6bf7`) | after (base `f64c871`) |
|---|---:|---:|
| 1× (139 398 frames) | 21.97 MB | **10.73 MB** |
| 64× (8 921 472) | 232.90 MB | **16.09 MB** |
| 256× (35 685 888) | — | **17.86 MB** |
| growth, 1×→64× | 10.60× | **1.50×** |
| fitted slope | 24.02 bytes/frame | **0.61 bytes/frame** |

Over four points to 256× the slope is 0.17 bytes/frame and the marginal
increment *shrinks* (+0.75, +4.70, +1.74 MB while each step lengthens the stream
4–8×): what is left is a heap plateau, not stream state. Independently
re-measured by the orchestrator on landed `main`, which is where the 1.68× and
0.17 bytes/frame figures above come from. The mechanism that was removed would
have added about 1.29 GB logical at 256×.

**Defect 1 — a sliding window, released to the planner's `base`.** `SynthesisOla`
drops everything below `min(base, origin + completed)` at the top of every push,
and `pcm()` returns the retained finalized window with `pcm_from()`/`pcm_end()`
for absolute coordinates; `push`/`finish` return exactly the newly finalized
samples, which is what the decoder now consumes.

The frontier is **`base`, not the previous block's centre** — and this page
previously said centre, which is wrong in a way that panics rather than merely
wasting memory. `position` is not monotone: a long block following a short one
starts *before* it, so a release to the finalization frontier lets a later block
write below the window's start. The centre bound is what makes a sample *final*
(safe to hand out); `base` is what makes it safe to *release*, because every
block's first touched sample is `base + origin − n/2 ≥ base` and `base` only
grows. The code's `release()` doc states both bounds.

**Defect 2 — a cursor instead of a front-drain.** `ContainerStream` releases the
consumed prefix only when it is at least as long as what remains, which is
amortised O(1) per byte. Batched drains were rejected by measurement, not
preference: draining every T bytes still moves the remainder once per drain, so
a whole-stream push stays O(push²/T). One push against 64 KiB pushes, same child
driver: +0.30 ms (+1.0%) at 1×, +14.36 ms (+6.6%) at 8×, **+889.15 ms (+51.0%)**
at 64× before; −0.14 ms, +1.15 ms and **+10.61 ms (+0.6%)** after. The 64×
one-push resident figure fell 374.36 → 244.01 MB, and the remainder is
`DecodeStep::pcm` (214 MB of f32) — API shape, unchanged by design.

**The check whose absence let a documented bound drift.**
`crates/wem-core/tests/decode_memory.rs` puts a child-process `ru_maxrss` ceiling
of 64 MiB on a 64× stream byte-identical (SHA-256) to the one this page's script
generates, so the ceiling and the finding describe the same stream. It was
demonstrated trippable rather than assumed: on the unfixed tree the child reports
230.7 MB and the test exits 101; on the fixed tree it reports 16.1 MB, 24% of the
ceiling. It also asserts the child exited 0, which the encode check it mirrors
does not — a child that dies early must not be able to pass a ceiling.

No sample moved. The staged replay's bit-for-bit comparison against the session
held on every repetition, one push and chunked.

## Trust boundary

- These numbers are one machine (Apple M3 Max, macOS 27, rustc 1.96.0), one
  build configuration (release, default features) and one tree, measured at a
  1-minute load average of 24.7-28.4 on 16 cores. They are not a budget and
  nothing in the tree compares against them. The load average is printed with
  every series because two readings taken at different loads are not
  comparable; the *paired* quantities here (one push against chunked pushes,
  64 KiB against one push, staged against session) are the load-insensitive
  ones. The **resident** figures are load-insensitive in the strong sense: a
  partial re-measurement at a 1-minute load of **60.7** — 2.4× the recorded
  load — gave 22.05 MB at 1× and 41.01 MB at 2× against the record's 22.89 and
  39.81 MB, while its wall-clock figures moved with the load.
- **The RSS curve is a lower bound at length, not a noisy point estimate.**
  `ru_maxrss` counts resident pages, and two allocator-level effects move the
  figure in opposite directions at different sizes: growing a `Vec` by
  reallocation briefly holds both buffers (over-counting at small sizes — the 4×
  point's 43.7 MB over baseline against a 26.8 MB logical timeline), while macOS
  reclaims pages of a buffer written once and never read again (under-counting
  at length — the 64× point's 223 MB against 428 MB). The endpoint comparison
  (22.89 → 233.16 MB over a 64× stream) rests on the three largest steps
  (+27.07, +48.77, +104.32 MB), each more than 8× the spread of the point it
  leads to (3.29, 2.16, 1.67 MB respectively); the 4×→8× plateau is inside its
  own spread and is reported as measured rather than smoothed. Together these
  mean the measured resident figure must not be read as a cap on what the
  session holds at length.
- The stage split is wall clock around shipped calls, so it inherits the
  process's own scheduling: the fixture's `residue_ms` varies by 9.9% run to
  run and `staged_total_ms` by 15.0%, which is the resolution of a 27 ms
  measurement on a shared machine. No call the replay makes is parallel — there
  is no `rayon` in `crates/wem-vorbis/src`, in `wem-analysis`'s `dsp`
  transform/spectrum modules or in `decoder.rs`, and the session drives the
  channels sequentially — so every stage figure here is CPU-bound work rather
  than a partition's wall clock.
- The staged replay is a **timing** replay, not a memory measurement: it
  materializes the whole decoded PCM and walks the whole container buffer, which
  the session does not. Every memory statement in this document comes from the
  child-process RSS measurement, never from the replay.
- The generated long streams are not a committed artifact and are not the
  fixture: they are the fixture's PCM tiled N times, which makes the encoder
  emit the same packets repeatedly. That is deliberate — the measured quantity
  is memory against length, not content coverage — and it is reproducible by
  any reader from the tracked fixture alone. No number here depends on the
  ignored `corpus/` tree.
- The comparison that guards the split is an equality between two
  independently-driven decodes (the replay's and the session's), bit for bit,
  on the tracked fixture. It says the split describes the shipped pipeline; it
  does not say the pipeline is right — that is what
  `crates/wem-core/tests/decode_reference_wem.rs` establishes against the
  source WAV.