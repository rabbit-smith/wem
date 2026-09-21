# Domain context

This glossary defines the domain language used by the standalone encoder.

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

## Frame regression record

A checked record for one encoding. It stores stable hashes at stage seams so a
change can identify the first differing frame, channel and stage before the
final WEM hash changes.
