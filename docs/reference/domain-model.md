# Domain context

This glossary defines the domain language used by the standalone encoder and its
decoder.

## PCM source

Signed 16-bit, channel-interleaved WAV input decoded into equal-length channel
sample sequences. A PCM source has a sample rate, channel layout and frame
count.

The input adapters also accept 24-bit PCM, 32-bit IEEE-float PCM WAV files and
raw PCM bytes with explicit geometry. Those forms are deterministically
converted into the signed-16 domain at the adapter boundary; from that point
on the pipeline (and the native kernel) sees only the signed-16 domain. Converted
inputs are deterministic but outside the signed-16 bit-exact guarantee.

## Wwise version

The Wwise generation an encoder configuration belongs to (`WwiseVersion`; the
one installed generation is Wwise 2013.2). It is part of a profile selection, so
two generations that share a geometry stay distinguishable. Version codes are
stable and append-only across every language binding, exactly like the C ABI
error values.

## Profile selection

The caller-facing way to name an encoder configuration: a Wwise version plus the
PCM channel count and sample rate (`WwiseProfile`). It resolves to exactly one
installed encoder profile, or is rejected — never substituted by a default and
never resolved by geometry alone. It is the only profile selector any binding
exposes.

## Encoder profile

The complete immutable configuration required to reproduce one Wwise 2013.2
encoder configuration. It includes geometry, setup, codebooks, channel mapping,
analysis tables, floor/residue configuration and container defaults. A profile
is more than WEM header metadata.

## Block plan

One scheduled audio frame: previous/current/following block modes, transition
variant, centre position and window geometry. A block plan contains no PCM or
encoded bits.

## Analysis session

The sole owner of mutable state that crosses block boundaries: transient
detector history, mode-selection history, psychoacoustic temporal curves and
global spectral maxima.

## Spectrum frame

The transform output for one block, including MDCT coefficients and the log
spectral surfaces required by psychoacoustic analysis.

## Psychoacoustic frame

The analysis output consumed by floor fitting and residue quantization. It
contains remapped, seeded, post-mask and side-channel surfaces for every
channel.

## Floor posts

The floor1 post values selected for one channel and serialized into an audio
packet. Their rendered curve normalizes the residue input.

## Residue integers

The quantized spectral values classified and vector-quantized after floor
normalization.

## Audio packet

One packed Wwise Vorbis block containing its mode, floor data and residue data.
It has no RIFF framing.

## WEM container

The RIFF/WAVE envelope containing Wwise fmt metadata, one setup packet and the
framed audio packets.

## Decode session

The streaming decode lifecycle: Init with no selection, zero or more WEM byte
chunks, exactly one Finish, then release. One session decodes one stream: it
owns the container framing and the codec state, resolves its configuration from
the WEM's own geometry rather than from a caller's selection, and delivers PCM
through a one-time header announcement and decoded blocks. A decode session has
single-threaded ownership, and nothing it holds is proportional to the stream
length.

## Header announcement

The decode session's one-time delivery, before any PCM, of the PCM geometry the
container declares — channel count and sample rate — together with the setup
packet this revision parsed. It is the mirror of an encode session's
setup-packet delivery, and it is the only place the geometry appears: without it
the decoded blocks cannot be interpreted.

## Decoded block

The unit a decode session delivers PCM in: interleaved f32 samples at ±1.0 full
scale for a bounded number of frames, so no block's size depends on how the
caller chunked its input. A decoded sample outside ±1.0 is passed through,
never clipped.

## Declared frame count

`dwTotalPCMFrames`: the frame count a WEM container declares, and the exact
number of frames a decode session that finishes successfully delivers. It is the
decode direction's length contract — a packet stream that does not cover it is
malformed input, and a short decode is never reported as a complete one. Output
sample *i* is the encoder's input sample *i*: the leading long half-block of the
synthesis stream is lead-in, not output, and the samples past the declared count
are the encoder's own tail look-ahead and are dropped.

## Decode result

The package's value for one decoded WEM: the geometry the header announcement
carried and the container's declared frame count, readable before iteration,
plus a one-shot iterator of decoded blocks. It owns one decode session, so
iterating consumes the decode, and releasing it — by collection or by `close()`
— releases the session.

## Frame regression record

A checked record for one encoding. It stores stable hashes at stage seams so a
change can identify the first differing frame, channel and stage before the
final WEM hash changes.
