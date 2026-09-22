# Decode transform feasibility

Date: 2026-09-22. This page is the evidence record for one question: **can a
deterministic inverse MDCT + OLA be built from the profile data this encoder
already carries, and at what reconstruction accuracy?** It answers the
transform stage only. It is not a decode design, and it is not a plan: what it
establishes is which part of a `decode` surface is bounded work and which part
was, until now, unknown.

## Why this was the blocking question

The repository can already decode most of a WEM's structure, because the encoder
needed the decoding direction for its own closed-loop checks. Ten segments make
the chain; five exist in the Rust kernel today:

| # | Segment | Status in `crates/` | Surface |
|---|---|---|---|
| 1 | Container read | exists | `load_wem_parts_bytes`, `extract_packets`, `parse_chunks` (`wem-container`) |
| 2 | Setup parse | exists | `parse_setup` (`wem-vorbis/src/setup.rs`) |
| 3 | Codebook Huffman decode | exists | `Codebook::decode` |
| 4 | VQ unquantize | exists | `Codebook::decode_vq` |
| 5 | Floor1 curve render | exists | `floor1_curve_from_posts`, `render_point`, `floor1_neighbor_tables` |
| 6 | Floor1 unwrap | missing | mirror of `floor1_wrap` |
| 7 | Residue decode | missing | mirror of `pack_residue_vq` / `classify_partition_type2` |
| 8 | Audio packet header parse | missing | reference has `parse_audio_packet`; no Rust counterpart |
| 9 | Stereo coupling inverse | test-only | `decode_branches`, inside `#[cfg(test)]` in `packet_encoder.rs` |
| 10 | **IMDCT + OLA** | **missing** | nothing |

Segments 6–8 are mirrors of code that exists, and 9 is a promotion out of a
test module. Segment 10 is the only stage with no counterpart in either
direction, so it is the only one whose cost was unknown. That is what this page
measures.

## What is already carried

The inverse needs two kinds of data, and both are frozen and shipped already.
Read through the built extension
(`wwise_wem._core.profile_tables()` → `wwise_wem_reference.profiles.artifact.decode`):

| Needed by the inverse | Table | Shape |
|---|---|---|
| MDCT twiddles | `mdct.{256,2048}.trig` | `n + n/4` f32, per size |
| Synthesis window | `frozen.window_halves` | `n/2` f32 per size |

`n + n/4` is not an arbitrary length: it is the array libvorbis builds once in
`mdct_init` and reads in **both** `mdct_forward` and `mdct_backward`. The
kernel's own fallback constructor reproduces that initializer line for line
(`crates/wem-analysis/src/dsp/transform.rs:42-61` builds exactly libvorbis's
three trig regions — `cos/sin(π/n·4i)`, `cos/sin(π/(2n)·(2i+1))`, and the
half-scaled `cos/sin(π/n·(4i+2))·0.5`).

So the twiddle data for the inverse is **not missing**. It is carried, bit-exact,
for both registered profiles. What was missing is only the routine that reads it
in the other direction.

## Method

The transform was isolated as a lapped-transform round trip with quantization
removed:

```
x → window → MDCT_forward → IMDCT_backward → window → OLA → x'
```

By TDAC, a lapped transform with a Princen-Bradley window reconstructs `x` to
float precision. Vorbis is lossy, but the *transform* is not — so any systematic
error here is a transform or geometry defect, never format loss. That is what
makes the test decisive rather than suggestive.

Three deliberate choices:

- **The forward half is the existing implementation**
  (`wwise_wem_reference.analysis.dsp.transform.mdct_forward`), so the spike
  tests only the new half and any disagreement is attributable to it.
- **The inverse is evaluated from the mathematical definition**, not from
  libvorbis's fast butterfly structure. The definition isolates the question
  "do the carried twiddles and the carried window pair correctly"; the fast
  algorithm is a separate, bounded porting question on top. Mixing the two would
  make a failure unattributable between "wrong data" and "wrong routine".
- **One global scalar is fitted and reported.** Normalization is a free
  parameter and legitimately absorbs a convention mismatch, so what is being
  read is the *shape* of the residual, not its absolute scale. Evaluation is
  over the interior of the covered region; the first and last window halves are
  incomplete by construction.

Harness: `/tmp/wem-decode-spike/roundtrip.py` (scratch, outside the tree).
Data read through the already-built extension — no Rust build was needed.

## Result

Nine overlapping blocks, deterministic input, interior evaluation, both
registered profiles:

| Profile | `n` | `max|w² + w'² − 1|` | rel. error (c=1) | rel. error (fitted c) | fitted `c` |
|---|---:|---:|---:|---:|---:|
| 2ch/48000 | 256 | 7.032e-08 | 1.573e-07 | 1.566e-07 | 1.000000 |
| 2ch/48000 | 2048 | 8.033e-08 | 2.280e-07 | 2.274e-07 | 1.000000 |
| 6ch/44100 | 256 | 7.032e-08 | 1.573e-07 | 1.566e-07 | 1.000000 |
| 6ch/44100 | 2048 | 8.033e-08 | 2.280e-07 | 2.274e-07 | 1.000000 |

Three facts hold at once:

1. **The frozen window is a TDAC window.** `w² + w'² = 1` to 7–8e-08, which is
   float32 rounding, not a window defect.
2. **The round trip closes at the float32 noise floor** — 1.6e-07 to 2.3e-07
   relative, with no structure left in the residual to localize.
3. **Normalization is already consistent.** The fitted scalar is `1.000000`
   exactly; the definition's natural normalization matches the forward's baked-in
   `4/n` scale (`MdctLook.scale = 4.0 / n`) with no correction. The `c=1` column
   being equal to the fitted column is the same statement.

Together these say the contract between the carried twiddles, the carried window,
and the overlap geometry is sound. The transform stage is therefore **bounded
porting work, not an open question**.

The two profiles produce identical numbers because the Vorbis window and the
trig bank are functions of the block size alone; the registered profiles share
both block sizes, so they share these tables.

## The fast inverse, verified against the frozen bank

The definition IMDCT above is `O(n²)` and is not what a decoder would ship. The
structure a decoder would port is libvorbis's `mdct_backward` — the same routine
whose twiddle reads the `n + n/4` bank. It was run against the **carrier's frozen
trig**, with libvorbis's own butterfly, bitreverse and rotation structure:

| `n` | bank vs analytic `mdct_init` | fast vs definition | definition↔fast scalar | round trip through the fast inverse |
|---:|---:|---:|---:|---:|
| 256 | 1.882e-07 | 1.182e-07 | 1.000000000 | 1.974e-07 (scalar 1.000000014) |
| 2048 | 2.033e-07 | 1.917e-07 | 1.000000006 | 3.025e-07 (scalar 1.000000019) |

Three further facts, all at float32 rounding:

1. **The frozen bank is libvorbis's `mdct_init` array.** Compare the carrier's
   `mdct.{n}.trig` against the analytic construction: they agree to 1.9e-07 /
   2.0e-07, which is the f32 storage of the carrier against an f64 evaluation.
   The earlier claim that the inverse's twiddles are already carried is therefore
   not an inference from the array's length — it is a value-for-value match.
2. **The fast structure and the definition compute the same transform**, to
   1.2e-07 / 1.9e-07 relative, with **no scale correction between them**
   (`c = 1.000000000` / `1.000000006`).
3. **The complete chain closes through the fast inverse** at 2.0e-07 / 3.0e-07,
   again with unit scalar. Forward, fast inverse, window and OLA compose to the
   identity on the interior without a fudge factor anywhere.

**The provenance of that transcription is a caveat, not a hidden dependency.**
The `mdct_backward` used here is a local transcription in
`corpus/paired-build/libv_mdct.py`, and `corpus/` is ignored by git
(`.gitignore:33`) — it is development material that does not ship and is not
present in a fresh worktree. The numbers above are therefore reproducible from
this evidence record only by re-deriving the transcription from libvorbis
`mdct.c`, which is exactly what the Rust port will have to do anyway. Nothing in
the shipped path may reference that file.

## What this does not establish

- **Operation-order reproducibility across targets is still open**, though the
  *structure* is now settled — see the last section. The comparison above used
  float64 throughout; a port fixes f32 rounding at each assignment, and that
  ordering has to be pinned and compared, not assumed.
- **No claim about any external decoder.** See the contract section below.
- **The rest of the chain is untested.** Segments 6–9 were assessed by reading
  the kernel, not by running a decode.
- **No geometry beyond the two registered profiles.** Both happen to use blocks
  256/2048, so this result covers two sizes, not a family.

## The contract question this settles

The reference decoder's bit-exact path binds to the host's libvorbis dylib
through `ctypes` and compiles a C helper with `cc` on first use, into
`tempfile.gettempdir()`; it documents itself as "not byte-stable across
environments" (`scripts/decode_wem.py`). The repository's determinism and
no-ambient-state norms
([`../reference/standards.md`](../reference/standards.md#determinism),
[#caller-streams-and-ambient-state](../reference/standards.md#caller-streams-and-ambient-state))
exclude that from anything shippable.

So a shipped `decode` **cannot** claim bit-exactness against libvorbis or
vgmstream — not because it is hard, but because the claim would contradict a
product norm. What it can claim, and what this spike shows is achievable, is a
**deterministic, self-consistent** decoder: fixed operation order, frozen
twiddles, frozen window, no host dependency.

The natural correctness bar follows: round-trip closure against this
repository's own encoder, which is itself bit-exact against the paired build.
That is verifiable with no external tool at all, and it is the bar
`tests/integration/test_two_channel_e2e.py` already gestures at with its
`CORRELATION_FLOOR = 0.98` and its own note that "byte identity is enforced
separately".

## A correction worth recording

The first run of this harness reported a Princen-Bradley violation of exactly
`1.0` and an all-zero reconstruction. The cause was the harness, not the data:
`frozen.window_halves.words` is returned by `artifact.decode` as floats, and
reading it as `uint32` bit patterns truncated every value to zero. The shape of
that failure — a window that satisfies nothing — is exactly what a genuinely
broken window would look like, which is why it is recorded here: a porter who
reads the carrier through a different path can reproduce it.

## What would make this harder than it looks

- **If the port's operation order cannot be made reproducible.** The transform
  *structure* is settled and verified above; what a port still has to do is fix
  f32 rounding at every assignment and pin the order. If that cannot be made
  deterministic across the shipped targets, the determinism bar fails and the
  contract has to weaken. This is a property of the routine to be written, not
  of the data, and the frozen twiddle bank removes the largest source of
  cross-target drift.
- **If a future geometry needs a bank the carrier does not hold.** Both current
  profiles carry 256 and 2048. A geometry with another block size would need its
  bank generated by the existing table-generation path before the inverse could
  run at all.