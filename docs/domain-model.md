# Domain context

This glossary defines the domain language used by the standalone encoder.

## PCM source

Signed 16-bit, channel-interleaved WAV input decoded into equal-length channel
sample sequences. A PCM source has a sample rate, channel layout and frame
count.

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

## Frame regression contract

A checked record for one encoding. It stores stable hashes at stage seams so a
change can identify the first differing frame, channel and stage before the
final WEM hash changes.
