# Decode surface — design proposal

Status: **proposal**, written 2026-09-22 against `main` at `b4bc9c6`. Nothing here
describes behaviour the repository has today; there is no decoder. This document
exists to be the interface contract the implementation lanes write against, so
that two lanes working on disjoint files do not have to invent matching shapes.

On implementation this document **decomposes** and should not survive as-is:
the norms move into [`reference/standards.md`](reference/standards.md) (which
stops being encoder-scoped), the design into a new `reference/decoding.md`, the
API into [`reference/public-interface.md`](reference/public-interface.md), and
the vocabulary into [`reference/domain-model.md`](reference/domain-model.md).
The feasibility evidence behind the transform is already recorded in
[`findings/decode-transform-feasibility.md`](findings/decode-transform-feasibility.md).

## 1. The decisions this document encodes

| # | Decision |
|---|---|
| D1 | **Input scope.** Any WEM this revision's container reader parses whose setup packet is the one the compiled carrier holds for the container's geometry. "Other generations are rejected" is a consequence of that, not a separate check — see §3. |
| D2 | **No profile argument.** The WEM is self-describing; the decoder derives its configuration from the container. The caller passes no selection. |
| D3 | **Streaming only.** There is one entry shape, not two. A one-shot collector is not part of the surface. |
| D4 | **Push in, PCM out.** `Init → push(WEM bytes)* → Finish`, PCM delivered through a callback. Structurally the mirror of the encoder's lifecycle with the data direction reversed. |
| D5 | **f32 output only**, normalized so ±1.0 is full scale. The library owns no integer rounding and no clipping policy. |
| D6 | **The inverse transform lives in `wem-analysis::dsp`**, paired with `mdct_forward` and sharing its injected typed configuration. |
| D7 | **One appended `WemError` value**; every other outcome reuses an existing code. Rust-side error type is a new `DecoderError`, not an extension of `EncoderError`. |
| D8 | **Exactly `dw_total_pcm_frames` samples**, sample *i* corresponding to the encoder's input sample *i*. |
| D9 | **The facade exports `decode(...)` as a generator**, not a session object and not a collector. |
| D10 | **Three orthogonal verification angles plus a robustness sweep**, each rated by a live relative comparison rather than any recorded number. |
| D11 | **Documentation is part of the deliverable**, spread the way the tree already spreads facts. |

Two premises fixed outside this document: the encode chain is losing SHA-256
wholesale (another lane), so **decode introduces no digest of any kind**; and
`include/wem.h` is shared with that work, so the ABI edit here is sequenced
after it.

## 2. Surface

### C ABI (`include/wem.h`, appended — nothing renumbered or reordered)

```c
/* One-time header announcement: the geometry, plus the setup packet this
 * revision parsed (the mirror of encode's seq-0 setup delivery). Required:
 * without it the caller cannot interpret the PCM. */
typedef WemError (*WemHeaderCb)(uint32_t channels, uint32_t sample_rate,
                                const uint8_t *setup, size_t setup_len,
                                void *user_data);

/* PCM delivery: interleaved f32, ±1.0 full scale. Bounded blocks. */
typedef WemError (*WemPcmCb)(const float *interleaved, size_t frames,
                             void *user_data);

WemError wem_decoder_new(WemHeaderCb header_cb, WemPcmCb pcm_cb,
                         void *user_data, WemDecoder **out_decoder);
WemError wem_decoder_push(WemDecoder *decoder, const uint8_t *data, size_t len);
WemError wem_decoder_finish(WemDecoder *decoder);
void     wem_decoder_free(WemDecoder *decoder);
```

Notes on the shape:

- **Both callbacks are required**, unlike `wem_session_new`'s optional
  `packet_cb`. The reason is a redundancy test, not symmetry: an optional
  callback is safe exactly when its data is also reachable through a required
  one, and here the geometry is not. See §3.
- **The geometry is announced once, through its own callback**, rather than
  repeated as constant arguments on every PCM callback. That makes "exactly
  once" a structural guarantee instead of a convention, and it costs no
  per-call redundancy.
- **`wem_decoder_new` takes no `WemProfile`.** With the geometry announced on
  the way out, the selection the encoder needs has no counterpart here: the
  encoder's names calibration data absent from its output, the decoder's is in
  its input.
- **`wem_decoder_finish` returns no summary struct.** With no digest anywhere,
  the only thing a summary could carry is a frame count the caller already has
  from the container's own `dw_total_pcm_frames`. A summary that restates the
  input is not worth an out-parameter with its own "written when and only when"
  rule.

### Rust

- `wem_analysis::dsp` gains the inverse MDCT and OLA, receiving the carrier's
  frozen trig bank and window halves as injected typed configuration exactly as
  `mdct_forward` does today.
- `wem_vorbis` gains the decode-direction bitstream work (D6's crate does not
  own it; see §4).
- `wem_core::decoder` owns the session and is the only place the chain is
  assembled, matching `wem-core`'s role as sole orchestration layer.
- `wem_core::error::DecoderError` is a new public enum, exhaustively matchable,
  with `source()` chains, per the errors norm.

### Python facade

```python
from wwise_wem import decode

result = decode(wem_bytes)      # a rejection surfaces HERE, not at first iteration
result.channels                 # geometry is available before iterating
result.sample_rate
result.total_frames             # the container's dw_total_pcm_frames
for chunk in result:            # f32, interleaved, ±1.0 full scale
    ...
```

**`decode` is a plain function returning an iterable result object, not a
generator function.** The reason is the required `header_cb`: the C ABI has a
place to announce the geometry before any PCM, and a bare generator has none.
The two ways to force one into a generator are both rejected elsewhere in this
document — a type-varying first item (the caller who forgets `next()` treats the
header as audio) and per-chunk redundancy (the thing the ABI decision rejected).
A plain function also gets the error timing right: a rejection is raised at call
time, matching `encode`'s documented behaviour, because a generator function's
body does not run until the first `next()`.

This adds one public result type, which is the same kind of thing
`EncodeResult` already is. D9's constraint was that no *session* type enters the
package root, and none does: the caller never sees Init, push or Finish.

**`decode` accepts `bytes`, `bytearray`, `memoryview`, or a path.** The path
form reads the file whole, mirroring how `encode` reads its input whole
(`_read_wav_pcm16_bytes`), rather than reading incrementally — incremental input
would be a capability `encode` does not have, not a mirror of one. Opening
happens at call time, so a file-access error surfaces there with its standard
`OSError` subclass.

## 3. Derived rules (no further decision needed)

These follow from the decisions above and are stated so lanes do not re-derive
them differently:

- **The origin offset is one long half-block, and it is not a free parameter.**
  The scheduler timeline's zero precedes PCM sample zero by one long half-block
  (`wem-scheduling/src/planner.rs:163-166`), so output sample *i* is OLA
  position `i + blocksizes[1]/2` and the leading `blocksizes[1]/2` samples of
  the synthesis stream are lead-in to be discarded. For both registered
  profiles that is 1024 samples. The development-tree decoder applies the same
  offset independently (`source_origin = blocksizes[1] // 2`), and the
  planner's own test pins the geometry
  (`plan_short_sequence_geometry`: `plans[0].sample_start == 896`,
  `advance == 128` for a short first block — `0 + 1024 − 128`, covering PCM
  `[-128, 128)` and thus overlapping sample zero). D8's length half is
  established empirically; this closes its alignment half from the encoder's
  own scheduler rather than by fitting.
- **An optional callback is legitimate only when its data is redundant with a
  required one's.** `wem_session_new`'s `packet_cb` may be NULL because the
  container `write_cb` delivers is a superset of those packets — passing NULL
  discards nothing. The same shape on the decode side would be a trap, because
  the geometry is available nowhere else in the output, and a NULL that
  silently costs the caller the ability to interpret the PCM is exactly the
  "default hides a caller mistake" that
  [`reference/standards.md`](reference/standards.md#errors) forbids. Hence
  `header_cb` is **required**, and the decode surface is deliberately *not*
  symmetric with the encode surface here: the asymmetry is in the shape,
  while the requiredness now matches — encode takes the geometry as a required
  argument, decode emits it through a required callback.
- **The generation is not a checked field, and no code should look for one.**
  There is no version discriminator in the container: `w_format_tag` is
  `0xFFFF` on every sample examined — ours, the paired build's, and real
  game audio — and the header's remaining fields track sizes and geometry, not
  generations. "Wwise 2013.2" therefore names *the configuration the carrier
  holds*, and a WEM from another generation is rejected because its setup
  packet is not that configuration. That is the carrier's authority working as
  intended, not a version test. A WEM from a generation that happened to
  produce a byte-identical setup would decode, and would decode correctly.
- **The error split is by whether parsing succeeded.** Container or setup packet
  fails to parse → the new code (`INPUT_MALFORMED`). Parses cleanly but names a
  configuration the carrier does not hold → the existing `FORMAT_UNSUPPORTED`.
  This separates "the file is bad" from "this revision cannot decode that kind"
  without guessing which one a mismatch is.
- **The container's block sizes are cross-checked against the resolved
  profile's.** `u_blocksize0_pow`/`u_blocksize1_pow` must agree with the
  profile's `block_sizes`; disagreement is malformed input, not a decode
  attempt. The trig bank is looked up from the resolved profile's `mdct_banks`.
- **Lifecycle rules are the encoder's, unchanged**: exactly one Init, zero or
  more pushes, exactly one Finish; Finish is terminal whatever it returns;
  chunk boundaries never affect the output samples; an empty push is a no-op;
  a defect that runs through a handle makes it terminal.
- **`decode` takes no `quality`.** Quality is an encoder input; the decoder
  reads whatever the bitstream says.
- **Session release follows the existing pattern: ownership by value, Rust
  `Drop`, no new protocol.** The generator holds the native session as a local,
  so finalizing the generator releases it — the same lifecycle the extension's
  `StreamSession` already has (`PyStreamSession { inner: WemStreamSession }`,
  no `impl Drop`, no `__enter__`/`__exit__`, no explicit `close`). A caller that
  wants a deterministic release point already has one without any new surface:
  generators have `close()`, so `contextlib.closing(decode(...))` works. The
  severity is bounded by the fact that a live decode session holds only the
  bytes of a not-yet-complete packet (capped by the container's
  `u_max_packet_size`), one block of OLA state and scratch — **nothing
  proportional to the stream length**, unlike a file handle or a collected
  buffer.

## 4. Chain decomposition and lane ownership

Ten segments. Five already exist in the kernel (the encoder needed them for its
own closed-loop checks), three are mirrors of code that exists, one is a
promotion out of a test module, one is genuinely new.

| Wave | Lane | Owns | Must not touch |
|---|---|---|---|
| 1 | **A — transform** | `crates/wem-analysis/src/dsp/` (inverse MDCT, OLA, its unit tests) | `wem-vorbis`, `wem-core`, `wem-capi`, `include/wem.h` |
| 1 | **B — bitstream** | `crates/wem-vorbis/src/{floor,residue,packet_encoder}.rs`, a new `packet_decoder.rs`, and `wem-vorbis/src/lib.rs` | `wem-analysis`, `wem-core`, `wem-capi`, `include/wem.h` |
| 2 | **C — orchestration + ABI** | `crates/wem-core/src/decoder.rs`, `error.rs`, `include/wem.h`, `crates/wem-capi/src/lib.rs` | `wem-analysis`, `wem-vorbis`, the shell crates |
| 3 | **D — shells + registration** | `crates/wem-python/src/lib.rs`, `crates/wem-wasm/src/lib.rs`, `src/wwise_wem/*`, `docs/*`, `tests/parity/distribution_allowlist.json`, `scripts/wheel_smoke.py` | the kernel crates |

Wave 1 is **two** lanes, not five. Segments 6–9 all live in `wem-vorbis` and
share its error type and its `lib.rs`; splitting them by topic would put four
lanes on one file, which the split-by-file rule forbids. Waves 2 and 3 are
dependency-ordered, not parallel.

The `wem-vorbis` work in lane B, item by item:

| Segment | Work | Mirror of |
|---|---|---|
| 6 | floor1 unwrap | `floor1_wrap` |
| 7 | residue decode | `pack_residue_vq`, `classify_partition_type2` |
| 8 | audio packet header parse | `parse_audio_packet` (reference tree only; no Rust counterpart today) |
| 9 | stereo coupling inverse | `decode_branches`, currently inside `#[cfg(test)]` in `packet_encoder.rs` |

## 5. Interfaces between lanes

Stated as promises, because these are the seams that would otherwise be
negotiated at merge time:

- **A promises C**: a synthesis entry that takes the frozen trig bank, the
  frozen window halves, a block-size pair and a mode sequence, and produces
  samples; and its own unit tests closing the transform round trip at the f32
  noise floor with unit scale.
- **B promises C**: decode-direction parsing to typed intermediate values
  (packet header, floor posts, residue coefficients) plus the coupling inverse,
  all returning `wem-vorbis`'s own error type and importing neither profiles nor
  container code.
- **C promises D**: the C ABI symbols above, the `DecoderError` → `WemError`
  mapping, and the lifecycle guarantees of §3.
- **D promises nothing to the kernel**; it is the last wave and owns every
  registration duty the repository requires.

## 6. Verification

Three orthogonal angles plus a robustness sweep — deliberately **not** a ranking,
because they answer different questions. Each states what it isolates and what it
has to assume.

| Angle | Isolates | Assumes | Material |
|---|---|---|---|
| **A — independent artifact** | the decoder, with no reliance on any result of ours | nothing | `decode(tests/fixtures/reference.wem)` against `tests/fixtures/input.wav` — real paired-build 6ch/44100 output, 139 398 frames |
| **B — coverage** | the decoder across both registered profiles and many inputs | the encode-side bit-exactness result, which makes `encode(x)` byte-identical to the paired build's output | `decode(encode(x))` against `x` over the tracked 2ch corpus and the `samples-r2b`/`samples-r3` pairs (`dw_total_pcm_frames` equals the source frame count on every pair checked) |
| **C — independent implementation** | the decoder against a different decode implementation | nothing about our numerics | our decoder against `scripts/decode_wem.py` on the same WEMs |
| **D — robustness** | no panic, no silent truncation | — | the 21 corpus WEMs, including the game-audio set in `corpus/paired-build/probe2/` |

**A is not "better" than B, and B is not a smoke test.** Because the encoder is
bit-exact, B's input is byte-identical to what the paired build would have
produced, so B measures the same thing as A over a far wider corpus; A's
distinct value is that it never asks anyone to accept the encode-side result
first. Neither ordering nor the older "tier" framing should be read into the
table.

**The bar is a live comparison, not a recorded number.** The repository's own
rule decides this: *"A recorded expectation is not a comparison: when it
disagrees with a run it hides the change behind a re-record step"*
([`reference/standards.md`](reference/standards.md#bit-exactness)). So A and B do
not compare against a fixed correlation floor. (`CORRELATION_FLOOR = 0.98` in
`tests/integration/test_two_channel_e2e.py` is exactly such a recorded
expectation, and was only ever labelled a decoder *quality smoke* bar.) The
requirement is relative and computed at run time:

> our decoder's error against the source, on a given WEM, is **no worse than**
> the reference decoder's error on that same WEM.

**C's bound is derived, not chosen — which removes the last recorded number.**
Writing ε_ref for the reference decoder's error against the source, requiring
our error to be at most ε_ref gives, by the triangle inequality,

```
|ours − reference|  ≤  |ours − source| + |source − reference|  ≤  2·ε_ref
```

so C's tolerance is `2·ε_ref` computed live from material A/B/C already need.
No number is picked, nothing is recorded, and the comparison stays a comparison.

The derivation is **conditional on A/B's requirement holding**: it uses
`|ours − source| ≤ ε_ref` as a premise, not as a fact. If our error against the
source exceeds the reference's, `2·ε_ref` is not a bound on C and the thing to
chase is A/B, not C. C is therefore orthogonal in what it *assumes about our
numerics* — nothing — but not in what it *depends on*.

**Which reference path is the comparison target.** `scripts/decode_wem.py` has
two synthesis paths: a deterministic f64 numpy default, and `--libvorbis-exact`,
which binds the host's libvorbis through `ctypes` and compiles a C helper with
`cc` on first use into a temporary directory. The second is documented as "not
byte-stable across environments" and **cannot be a test oracle**; the
deterministic default path is the only comparison target. This is recorded so
that nobody later "strengthens" the suite by reaching for the exact path.

**What A/B do not establish.** They compare against the source audio, so they
cannot separate "our decoder is wrong" from "both our decoder and the reference
share a wrong assumption about the container or the setup packet" — C exists for
that, and its independence is implementation-level, not assumption-level. The
only genuinely assumption-free artifact in the repository is A's, and it covers
one geometry.

## 7. Registration duties (lane D)

Unchanged from the standing rules, listed because decode is easy to forget:

- `tests/parity/distribution_allowlist.json` for any new module,
- `reference/public-interface.md` for the new export,
- `make wheel-smoke` for the installed and zip-import paths,
- `reference/standards.md` for the decode contract and its "established by"
  rows,
- and no profile data changes: the decoder consumes the carrier the encoder
  already ships.

## 8. Settled during review

Every question that was open when this document was first drafted, and what
settled it. Recorded rather than deleted so none is reopened without its reason.

- **What releases the native session?** Ownership by value and Rust `Drop`, as
  the extension's `StreamSession` already does. The session holds only the bytes
  of a not-yet-complete packet, one block of OLA state and scratch — nothing
  proportional to the stream length — so release timing is not a resource
  hazard, and `contextlib.closing(decode(...))` already gives a caller a
  deterministic point if they want one. §3.
- **Is `header_cb` optional, to match `packet_cb`?** No. The test is redundancy:
  `packet_cb` may be NULL because the container `write_cb` delivers is a superset
  of those packets, so nothing is lost; the geometry is available nowhere else
  in the decoder's output, so a NULL there would silently cost the caller the
  ability to interpret the PCM. §3.
- **What is the generator's chunk size?** An implementation detail, not a
  parameter: the result object owns the pacing, and a caller who needs a
  specific block size can regroup what it is handed.
- **Does `decode` accept a path?** Yes, reading it whole, mirroring `encode`.
  This question was first settled as "no" on the argument that a generator
  function defers file errors to iteration time; `decode` becoming a plain
  function returning an iterable result object dissolved that argument, so the
  decision was re-made rather than left standing on a dead premise. §2.
- **The PCM layout: interleaved.** Channel-major is what the decoder produces
  internally and what `PcmBuffer` speaks; interleaved is what the ABI boundary
  gives, matching the encoder's interleaved s16 *input* boundary, so the two
  directions meet at the same representation.
- **Does `header_cb` also carry the setup packet bytes?** Yes. It costs nothing
  — the bytes are already parsed — and it is the mirror of encode's seq-0 setup
  delivery. A caller that ignores them loses nothing, which is the redundancy
  test passing rather than being waived.

`wem_decoder_finish` taking no out-parameter is deliberately *not* listed here
as a question. It is only asymmetric with encode while `WemMeta` exists; that
struct is being removed, which leaves `wem_session_finish` param-less too and the
two lifecycles aligned. If `WemMeta` survives in some other form, this decision
should be revisited against whatever replaces it.