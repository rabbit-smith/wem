# Decoding

The mirror of the encoding flow with the data direction reversed: a WEM is
handed to the kernel in chunks, and interleaved f32 PCM comes back. The product
norms this document's design serves are in [`standards.md`](standards.md); the
vocabulary is in [`domain-model.md`](domain-model.md); the interface every shell
mirrors is [`include/wem.h`](../../include/wem.h) section 5.

## The chain

```text
container framing (incremental)
  -> setup packet (parsed, then checked against the compiled carrier)
  -> per audio packet: mode and mapping -> floor1 posts -> residue
     coefficients -> mapping0 coupling inverse -> floor envelope
  -> inverse MDCT + hybrid synthesis window + overlap-add
  -> interleaved f32 PCM, exactly dw_total_pcm_frames frames
```

`wem_core::decoder::DecodeSession` is the only place the chain is assembled
(`crates/wem-core/src/decoder.rs`); every stage below it belongs to
`wem-vorbis`'s decode-direction segments and `wem-analysis::dsp`'s inverse
transform, and none of them is re-implemented by a shell. The container framing
is the decode session's own, and only the header region is buffered whole: the
data payload's shape (its seek table length) is known only once `fmt ` has been
parsed, so payload bytes are consumed as they arrive and a packet is decoded as
soon as its bytes are complete.

## The surfaces

| Surface | Entry points | Declared by |
|---|---|---|
| Rust kernel | `wem_core::decoder::{DecodeSession, DecodeStep, DecodedHeader}` | `crates/wem-core/src/decoder.rs` |
| C ABI | `wem_decoder_new` / `_push` / `_finish` / `_free`, `WemHeaderCb`, `WemPcmCb` | [`include/wem.h`](../../include/wem.h) section 5 |
| PyO3 (`wwise_wem._core`) | `Decoder`, `DecodeStep`, `DecodedHeader` | `crates/wem-python/src/lib.rs` |
| wasm-bindgen | `WemDecoder` | `crates/wem-wasm/src/lib.rs` |
| Package (Python) | `wwise_wem.decode` | [`public-interface.md`](public-interface.md#decode) |

The C ABI is the authority. Every other shell mirrors its lifecycle, its
delivery framing and its error classes 1:1, owns no numerics and no profile
logic, and gets a new stable C ABI entry point rather than a kernel fork if it
ever needs something the others do not. The two non-C shells do not have
callbacks: a step's output is *returned* (the PyO3 step object, the wasm step
object) instead of delivered through `header_cb` / `pcm_cb`, which is the same
translation the encode shells already make for `wem_session_push`'s packets.

The package facade is the one caller-facing entry: `decode(source)` is a plain
function returning a decode result, and the caller never sees Init, push or
Finish. That shape is deliberate — see [Settled decisions](#settled-decisions).

## Lifecycle

Exactly one `Init`, zero or more WEM chunks, exactly one `Finish`, then release.
There is no profile argument and no selection: the WEM is self-describing, and
the configuration is resolved from the container's own geometry.

- A **header announcement** fires exactly once per session, before any PCM, and
  carries the PCM geometry (`channels`, `sample_rate`) plus the setup packet
  this revision parsed. Both C callbacks are required: the geometry is available
  nowhere else in the output, so a NULL header callback could only hide the
  caller's mistake — unlike the encoder's `packet_cb`, which may be NULL because
  the container bytes the write callback delivers are a superset of it.
- **Chunk boundaries never affect the emitted samples.** Any chunking of the
  same WEM bytes emits the same PCM; an empty chunk is a no-op. The PCM is
  handed over in bounded blocks (1024 frames each at the C ABI, the delivery
  size the PyO3 shell mirrors), so a caller never sees a block whose size
  depends on how it chunked its input.
- **A rejection is not terminal, except when it is a defect.** A push that
  reports any code but `WEM_ERR_INTERNAL` leaves the handle usable: the bytes it
  could not read stay pending, the frames the packets before them completed are
  still delivered, and the next call reports the same rejection again. Nothing
  is skipped and nothing is silently truncated.
- **`WEM_ERR_INTERNAL` is terminal** for the handle it ran in, exactly as for
  the encoder: a defect means this library broke an invariant, and the state it
  left behind is not something a caller may build on. Its output is not
  delivered.
- **`Finish` is terminal whatever it returns**, and a callback that returns
  anything other than `WEM_OK` aborts the decode and marks its session terminal.
  On success the session has delivered exactly the declared frame count.
- A decode session has **single-threaded ownership**, like the encoder's
  `WemSession`: it is never shared between threads.

## What is accepted, and what is refused

A WEM this revision's container reader parses, whose setup packet is the one the
compiled carrier holds for the container's geometry. The error class is decided
by *whether parsing succeeded*, never by guessing which side of a mismatch is
wrong:

| Refusal | Class |
|---|---|
| bytes that are not a container this revision parses, a setup packet that does not parse, an audio packet that does not parse, a container whose block sizes disagree with the resolved profile's, a packet stream that does not cover the declared frame count, a residue that stops on a codeword its codebook does not assign | malformed input |
| a container that parses cleanly but is not a Wwise Vorbis container, names a geometry this build does not hold, or carries a setup packet that is not the one this build holds | unsupported configuration |
| a call outside the lifecycle | state error |
| a defect in this library | internal |

**The generation is not a checked field, and no code looks for one.** There is
no version discriminator in the container: `w_format_tag` is `0xFFFF` on every
sample examined — ours, the paired build's, and real game audio — and the
header's remaining fields track sizes and geometry, not generations. "Wwise
2013.2" therefore names *the configuration the carrier holds*, and a WEM from
another generation is refused because its setup packet is not that
configuration. That is the carrier's authority working as intended, not a
version test: a WEM from a generation that happened to produce a byte-identical
setup would decode, and would decode correctly.

**The container's block sizes are cross-checked against the resolved profile's**
(`u_blocksize0_pow` / `u_blocksize1_pow` against the profile's `block_sizes`).
A disagreement is malformed input, not an attempt to decode with the wrong block
sizes.

**One code per class, no more.** The malformed-input class is
`WEM_ERR_INPUT_MALFORMED` (Python `INPUT_MALFORMED`), the decode surface's own
appended value; no encode entry returns it. The unsupported-configuration class
reuses `WEM_ERR_FORMAT_UNSUPPORTED`, the lifecycle class `WEM_ERR_STATE_ERROR`,
and the defect class `WEM_ERR_INTERNAL`, so a client that already branches on
the encode table keeps working. The Rust-side type is `DecoderError`, its own
enum rather than extra arms on `EncoderError`: the decode direction has failure
modes the encode direction cannot reach and lacks ones it can, and every public
error enum stays exhaustively matchable.

## Output length and alignment

Exactly the container's declared frame count (`dwTotalPCMFrames`) is emitted,
and output sample *i* is the encoder's input sample *i*.

The origin offset is **one long half-block** — `blocksizes[1] / 2`, 1024 samples
for both registered profiles — and it is not a free parameter. The scheduler
timeline's zero precedes PCM sample zero by that half-block, so PCM sample zero
is synthesis-stream position `blocksizes[1] / 2`: the leading span is lead-in to
be discarded, and `wem-analysis::dsp`'s synthesis overlap-add already applies
the offset, so nothing downstream re-derives or fits it. The samples past the
declared count are the encoder's own tail look-ahead and are dropped, never
delivered.

A packet stream that synthesizes fewer frames than the container declares is
malformed input, never a short decode handed back as if it were complete. The
frame count is checked after the last block has landed, so a complete stream is
never reported as short.

## The streaming property

Memory does not grow with the stream. A live decode session holds the bytes of
the packet it is currently reading, one block awaiting its follower's mode
(bounded by the profile's long block size), one overlap-add state per channel
and scratch — nothing proportional to the stream length. The one ordering
exception is stated rather than glossed: a container that put its `data` chunk
*before* its `fmt ` chunk cannot have its payload interpreted until `fmt `
arrives, so that payload is held until then. Every container this repository's
writer and the paired build produce puts `fmt ` first, and the exception is a
memory bound, never a wrong sample.

**This is a claim, so it has a test.**
`crates/wem-core/tests/decode_memory.rs` decodes a 64× stream (202 s of audio)
in a child process and caps that child's peak RSS, the way
`crates/wem-core/tests/streaming.rs` caps a streaming *encode*. It asserts the
child exited 0 as well as the ceiling, so a child that dies early cannot pass by
dying. The bound is not decorative: before the overlap-add was bounded it
retained the whole timeline and this test failed at 230.7 MB. The curve behind
it, its before and after, and the two mechanisms that were repaired are in
[`../findings/decode-performance.md`](../findings/decode-performance.md).

The delivery framing keeps the same shape end to end: the kernel reports what
each step completed, the C ABI hands it over in fixed-size callback blocks, the
PyO3 and wasm shells return it as one step value, and the package facade yields
the bounded blocks one iteration step at a time. A caller that stops iterating
releases a session holding bounded state, which is why release timing is not a
resource hazard.

**How the early end and the defect are told apart.** `Codebook::decode` reports
a failed bit read and an unassigned tree branch as the same error, and the
residue stage folds both into an end-of-packet status. The format allows a
packet to end inside its residue, so an early end is not a decode failure — but
it is classified from the one thing the conflation does not destroy, the
reader's residual bit count at the stop: a read that ran out consumes every
remaining bit, while an unassigned branch is only reached after a successful
read, so bits remain. `bits_left > 0` at the stop is therefore a bitstream
defect and is reported as such. The test is deliberately one-directional — an
unassigned branch taken on the packet's very last bit leaves no bits either and
is accepted as an early end — and the asymmetry is stated in
`crates/wem-core/src/decoder.rs` rather than papered over.

## The declared frame count

The header announcement carries the declared frame count beside the geometry and
the setup packet, so it is readable before any sample arrives and **no shell
reads the container for it**.

It travels there because the alternatives are worse. The session has already
parsed the value — it needs it to clamp its output — so announcing it costs
nothing; leaving it out would make every shell locate the container's `fmt `
chunk and read `dwTotalPCMFrames` at the offset `wem-container`'s
`VorbisFmtFields` names. That is container layout knowledge in a layer the
integration topology keeps free of it, and it would be duplicated once per
shell rather than fixed once. The announcement also keeps the header callback's
own justification intact: the geometry is required because it is available
nowhere else in the output, and the declared count is a container fact of
exactly that kind.

The value is consistent by construction: a session that finishes successfully
has delivered exactly the declared count, and the kernel reports a mismatch
instead of a result when it has not.

## Settled decisions

Recorded with their reasons so none is reopened without one.

- **The facade's `decode` is a plain function returning an iterable result
  object, not a generator function.** A generator function's body does not run
  until the first `next()`, so a rejection would surface at iteration time,
  against the facade's documented call-time error behaviour; the C ABI also has
  a place to announce the geometry before any PCM (`header_cb`) and a bare
  generator has none. The result object holds the native session and hands out
  the blocks, and the geometry is readable before the first block.
- **What releases the native session?** Ownership by value and Rust `Drop`, as
  the extension's `StreamSession` already does: no new protocol, no
  `__enter__`/`__exit__`. `close()` — the method generators already have — gives
  a caller a deterministic point, so `contextlib.closing(result)` works.
- **Is the header callback optional, to match `packet_cb`?** No. The test is
  redundancy: `packet_cb` may be NULL because the container `write_cb` delivers
  is a superset of those packets, so nothing is lost; the geometry is available
  nowhere else in the decoder's output, so a NULL there would silently cost the
  caller the ability to interpret the PCM.
- **What is the block size?** An implementation detail of the delivery, not a
  parameter: the samples do not depend on it, and a caller that needs a
  specific block size regroups what it is handed.
- **Does `decode` accept a path?** Yes, read whole, mirroring how `encode` reads
  its input whole; opening happens at call time, so a file-access error surfaces
  there as its standard `OSError` subclass.
- **The PCM layout is interleaved.** Channel-major is what the decoder produces
  internally; interleaved is what the C ABI boundary gives, matching the
  encoder's interleaved input boundary, so the two directions meet at the same
  representation.
- **The header announcement also carries the setup packet.** It costs nothing
  (the bytes are already parsed), it mirrors the encoder's seq-0 setup delivery,
  and a caller that ignores it loses nothing — the redundancy test passing
  rather than being waived.
- **`Finish` takes no out-parameter.** The only thing a summary could carry is
  the frame count the container already declares, and a digest of bytes the
  caller already holds is the caller's to compute. This is the same rule the
  encoder's `Finish` follows.
- **`decode` takes no `quality`.** Quality is an encoder input; the decoder
  reads whatever the bitstream says.
- **Decode is a mode flag, not a CLI subcommand.** `wwise-wem INPUT.wem
  --decode --output OUTPUT.wav`, the same flag with the same meaning in the Rust
  binary and the Python console script. A `decode` subcommand would take the
  argv slot a file named `decode` already occupies, which is the command line's
  existing shape rather than a preference, and `tests/parity/test_cli_decode.py`
  pins that such a file still encodes.
- **The command line writes signed-16 PCM WAV.** This entry used to record that
  decode was not on the command line at all, on the ground that exposing it
  "would have to choose a PCM output container and sample format, which is a new
  surface rather than a mirror of an existing one". That choice has been made:
  uncompressed signed-16 PCM WAV, because the kernel's own WAV reader accepts
  only that format, so a decode output feeds straight back into either CLI's
  encode. The mapping is the repository's existing normative rule
  (`sample_conversion.float_to_int16`), which is why the two front ends emit
  identical bytes rather than merely similar ones. A refused decode writes
  nothing at all — no file, no directory — and says how much of the declared
  stream it left behind. The shape and the format are normative in
  [`public-interface.md`](public-interface.md).

## What is established

| Claim | Established by |
|---|---|
| The committed paired-build container decodes to its WAV source: geometry, 139 398 frames, and a reconstruction bounded by the source's own peak | `crates/wem-core/tests/decode_reference_wem.rs`; `tests/parity/test_decode_surface.py` through the shipped facade |
| `decode(encode(x))` reconstructs `x` across both registered profiles and the tracked 2ch corpus, and `decode(reference.wem)` equals `decode(encode(input.wav))` exactly | `crates/wem-core/tests/decode_reference_wem.rs`, `crates/wem-core/tests/encoder.rs` |
| Chunk boundaries never move a sample, and empty chunks are no-ops | `crates/wem-core/tests/decode_reference_wem.rs`; `crates/wem-python/src/lib.rs` (the shell's own surface) |
| A rejection does not advance the stream, `Finish` is terminal, and a decoded stream is deterministic | `crates/wem-core/tests/decode_reference_wem.rs` |
| The C ABI decode surface: lifecycle, error-class mapping, callback delivery order, terminal handles | `crates/wem-capi/tests/decode_capi.rs`, `crates/wem-capi/tests/capi_surface.rs` |
| The shells mirror the error table and the step framing 1:1 | `crates/wem-python/src/lib.rs`, `crates/wem-wasm/src/lib.rs` (their unit tests) |
| The shipped wheel decodes through its embedded kernel: the declared geometry and frame count of the container it encoded, in a clean environment | `python3 scripts/wheel_smoke.py` |

The comparison these rows describe is a live one, never a recorded number: the
decoded samples are compared against the source PCM at run time, and the
structural bound (`max|error|` within the source's own peak) is what catches a
decode that is not a reconstruction at all.

## What is not claimed

**No bit-exactness against libvorbis, vgmstream or any other decoder.** The
contract is settled by the determinism norm: a deterministic, self-consistent
decoder verified by round trip against this repository's own encoder. The
reference path that would make an external comparison look bit-exact binds the
host's libvorbis through `ctypes` and compiles a C helper with `cc` on first use
(`scripts/decode_wem.py --libvorbis-exact`), and is documented as not
byte-stable across environments — so it cannot be a test oracle, and no row
above depends on it. The feasibility evidence for the transform, and why that
route was rejected, is in
[`../findings/decode-transform-feasibility.md`](../findings/decode-transform-feasibility.md).