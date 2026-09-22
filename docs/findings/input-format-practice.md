# Input formats: what comparable encoders accept, and what the sources actually require

Date: 2026-09-22. A survey of external practice and of the first-party documents
behind it, written to answer one question about this library. Everything below
was read from the owning source — an API header, a specification, a vendor's own
developer documentation, or a project's own source — on 2026-09-22. Where a claim
could not be traced to such a source it is marked **not confirmed** rather than
softened, and the exact version or branch read is named beside each one. Two
vendor documentation hosts were unreachable from this environment; that is
recorded where it matters and again in [What could not be
confirmed](#7-what-could-not-be-confirmed).

## 1. The question

The owner put it as two options:

> "We support WAV input only. Do we need to support multiple input formats,
> through an adapter or interface of some kind? Or should we leave that to other
> libraries and just provide a standard, compatible converted input? Which do you
> think?"

Option A is an adapter or interface through which the library accepts more input
formats. Option B is to keep the accepted input narrow and standard, and to let
whatever produced it do the converting. The deliverable is not a preference: it is
what comparable software does, which of that is written down as a rule rather than
practised, and what each choice would cost here.

One distinction decides most of the answer, so it is worth stating before the
evidence. "Input format" is three different things, and the ecosystems treat them
differently:

- **the wrapper** — RIFF/WAVE, AIFF, RF64, Wave64. Reading another wrapper changes
  no sample value; it is parsing.
- **the sample representation** — bit depth, endianness, integer or float. Changing
  it changes every sample value, by a rule somebody has to choose.
- **the compression** — PCM, Vorbis, MP3, FLAC, Opus. A lossless decoder is exact
  and a lossy one is not, and either way the decoded samples are whatever that
  decoder produces.

The evidence below is grouped along that split, because the norms are different on
each side of it.

## 2. What each ecosystem does

### 2.1 The codec libraries: what their public API names as input

**libx264** — read from `x264.h` on the `master` branch of the official repository
at `code.videolan.org` (`X264_BUILD` 165; the file's copyright line reads
"2003-2025 x264 project"). The library's input is a picture structure holding raw
planes, and the header says so in the field comments
([x264.h](https://code.videolan.org/videolan/x264/-/raw/master/x264.h)):

> `typedef struct x264_image_t`
> `{`
> `    int     i_csp;       /* Colorspace */`
> `    int     i_plane;     /* Number of image planes */`
> `    int     i_stride[4]; /* Strides for each plane */`
> `    uint8_t *plane[4];   /* Pointers to each plane */`
> `} x264_image_t;`

and the picture structure documents the same field as caller-supplied
("`/* In: raw image data */`" above `x264_image_t img;`, with the `Out:` comment
describing reconstructed data coming back). The encode entry point is documented
in five words — "encode one picture." — and takes that picture plus output NAL
pointers; there is no parameter anywhere in the public header that names a media
source, a container, or a decoder. The file-named parameters in `x264_param_t` are
the two-pass statistics file, the custom quantization matrix file, the zones
string, the rebuilt-frame dump and the OpenCL kernel cache; none of them names a
media source.

**libvpx** — read from `vpx/vpx_encoder.h` on the `main` branch of
[webmproject/libvpx](https://github.com/webmproject/libvpx/blob/main/vpx/vpx_encoder.h).
The frame entry point takes an image:

> "Encodes a video frame at the given "presentation time." […]"
> "\param[in]    img       Image data to encode, NULL to flush.
>                          Encoding sample values outside the range
>                          [0..(1<<img->bit_depth)-1] is undefined behavior."

and where the caller's frames disagree with what the codec wants, the header puts
the conversion on the caller, in the field that exists for exactly that
(`g_input_bit_depth`):

> "This value identifies the actual bit-depth of the input source in bits. It
> must not exceed codec bit-depth. Note that the frames passed as input to the
> encoder must match codec bit-depth. So, if there is a mismatch between source
> bit-depth and codec bit-depth, the application is required to upshift the frame
> to the codec bit-depth before passing it for encoding."

Frame geometry is the same shape: "Note that the frames passed as input to the
encoder must have this resolution."

**libaom** — read from `aom/aom_encoder.h` at upstream version 3.6.0, in the
distribution's copy of the upstream file
([sources.debian.org](https://sources.debian.org/data/main/a/aom/3.6.0-1+deb12u2/aom/aom_encoder.h);
the project's own host did not answer from this environment). Same shape, and the
same sentence about the input's bit depth:

> "\param[in]    img       Image data to encode, NULL to flush.
>                          Encoding sample values outside the range
>                          [0..(1<<img->bit_depth)-1] is undefined behavior."

> "This value identifies the bit_depth of the input frames in bits.
> Note that the frames passed as input to the encoder must have
> this bit-depth."

**LAME** — read from `include/lame.h` and `doc/man/lame.1` on the `master` branch
of [lameproject/lame](https://github.com/lameproject/lame/blob/master/include/lame.h).
The library's several encode entry points all take PCM, and their parameter
comments name the scaling the caller has to apply: `short int pcm[]`, "PCM data
for left and right channel, interleaved"; float variants, "data must still be
scaled to be in the same range as short int, +/- 32768"; the IEEE-float variants,
"data must still be scaled to +/- 1 full scale"; and `long`/`int` variants with
their own stated scaling. The comment above the basic buffer entry point states
where resampling sits:

> "input pcm data, output (maybe) mp3 frames.
> This routine handles all buffering, resampling and filtering for you."

with the input's rate declared separately (`lame_set_in_samplerate`, "input sample
rate in Hz. default = 44100hz") and the output rate restricted to MPEG's set
("MP3 only allows: MPEG1 32, 44.1, 48khz; MPEG2 16, 22.05, 24; MPEG2.5 8, 11.025,
12"). LAME's decoder is explicitly not the library: it is optional at build time
and borrowed — "a simple interface to mpglib, part of mpg123, is also included if
libmp3lame is compiled with HAVE_MPGLIB".

**libopus** — read from `include/opus.h` on the `main` branch of
[xiph/opus](https://github.com/xiph/opus/blob/main/include/opus.h). The encoder's
input is PCM at one of five rates, and a rate outside that set is not the
library's problem:

> "\param [in] Fs <tt>opus_int32</tt>: Sampling rate of input signal (Hz)
>                                      This must be one of 8000, 12000, 16000,
>                                      24000, or 48000."

The payload across its encode entry points is named as "Input signal (interleaved
if 2 channels)" in `opus_int16`, "Input signal (interleaved if 2 channels)
representing (or slightly exceeding) 24-bit values" in `opus_int32`, and "Input in
float format (interleaved if 2 channels), with a normal range of +/-1.0". The
header also notes the encoder "can switch to a lower audio bandwidth or number of
channels if the bitrate selected is too low", which is a decision inside the
encoder about its own output, not about the caller's input.

### 2.2 The front ends that do read containers

Three of those projects ship a command-line program that *does* read containers,
and in each case the reading is in the front end, not in the codec library the
front end links.

- **x264's command line** has a demuxer layer with pluggable input modules. Read
  from `x264.c` in the GitHub mirror of the official tree (the project's own host
  served a bot check for the file; the mirror's `master` carries the same
  `x264_demuxer_names` array):
  `"Specify input container format [\"auto\"]"` with the names
  `"auto", "raw", "y4m"` and, when built in, `"avs"`, `"lavf"`, `"ffms"`; plus
  `--input-fmt` ("requires lavf support"), `--input-csp`, `--input-depth`,
  `--input-res`. The library the CLI links takes the picture struct quoted above.
- **opusenc's manual page** (`man/opusenc.1`, `master`, page header dated
  2019-09-07) states the front end's own accepted set:
  > "reads audio data in Wave, AIFF, FLAC, Ogg/FLAC,
  > or raw PCM (integer or floating point) format
  > and encodes it into an Ogg Opus stream."

  Its raw-input options mirror the library's requirements exactly
  (`--raw`, `--raw-float`, `--raw-bits`, `--raw-rate`, `--raw-chan`,
  `--raw-endianness`) — the headerless path is the library's own input, and the
  header path is the front end's convenience. FLAC input is a build-time option
  there: `configure.ac` declares `--without-flac` to "disable FLAC support" and
  otherwise requires the library ("FLAC 1.1.3 or later is required to build this
  package!").
- **LAME's command line** detects containers itself and carries one decoder:
  > "Without `-r`, LAME will perform several `fseek()`'s on the input file looking
  > for WAV and AIFF headers."

  It also resamples there — "LAME will automatically resample the input file to
  one of the supported MP3 samplerates if necessary" — and decodes MPEG input for
  re-encoding (`--mp1input`, `--mp2input`, `--mp3input`, the last documented as
  "Useful for downsampling from one mp3 to another"). All of that is the front
  end; the library takes PCM.

So the split is consistent: the codec takes PCM; the program that a person runs
reads the file.

### 2.3 The counter-example: an API that owns decoding and conversion

Apple's AudioToolbox is the surveyed API that *does* take a file and convert
internally, and its documentation shows both halves. Read from the AudioToolbox
headers in the local macOS SDK (SDK 27.0; the header copyright lines read
"(c) 1985-2015 by Apple, Inc."). The quotes below are from those headers; the
links name the same symbols' public documentation pages.

The file object converts into a caller-declared format, and that format must be
PCM
([ExtendedAudioFile.h](https://developer.apple.com/documentation/audiotoolbox/extaudiofile)):

> "The format must be linear PCM (kAudioFormatLinearPCM).
>
> You must set this in order to encode or decode a non-PCM file data format.
> You may set this on PCM files to specify the data format used in your calls
> to read/write."

The converter underneath owns the whole matrix, and the same header names what it
can do rather than how
([AudioConverter.h](https://developer.apple.com/documentation/audiotoolbox/audioconverter)):

> "AudioConverters convert between various linear PCM and compressed
> audio formats. Supported transformations include:
>
> - PCM float/integer/bit depth conversions
> - PCM sample rate conversion
> - PCM interleaving and deinterleaving
> - encoding PCM to compressed formats
> - decoding compressed formats to PCM"

and, for a pair of PCM formats, it lists the sample-rate case beside the bit-depth
cases: "sample rate conversion" and "conversion between any pair of the following
formats: […] 16, 24, or 32-bit integer, big- or little-endian […] 32 and 64-bit
float, big- or little-endian."

The API also lets the caller choose *which* implementation does it:
`kExtAudioFileProperty_CodecManufacturer` is documented as choosing "between a
hardware or software encoder".

### 2.4 The API that owns the source file: Wwise's own authoring converter

This is the section that decides the question rather than informing it, so it is
worth being precise about what is and is not readable.

Audiokinetic publishes a set of Wwise SDK headers under an Apache-2.0 dual
license in its own repository,
[audiokinetic/WwiseIncludes](https://github.com/audiokinetic/WwiseIncludes). The
authoring plug-in interface in that repository contains a **media converter**
interface, and the host calls it with a *file* and a *target sample rate* — not
with samples. Read at branch `master` (SDK version `v2017.2.0` build 6500, whose
tip commit is dated 2018-01-25) and at branch `wwise_v2016.1`; the text is
identical in both
([AudioPlugin.h](https://github.com/audiokinetic/WwiseIncludes/blob/master/SDK/include/AK/Wwise/AudioPlugin.h)):

> `class IPluginMediaConverter`
> `{`
> `public:`
> `        virtual ConversionResult ConvertFile(`
> `            const GUID & in_guidPlatform,`
> `            const BasePlatformID & in_basePlatform,`
> `            LPCWSTR in_szSourceFile,   ///< Source File to convert data from.`
> `            LPCWSTR in_szDestFile,     ///< DestinationFile, must be created by the plug-in.`
> `            AkUInt32 in_uSampleRate,   ///< The target sample rate for the converted file, passing 0 will default to the platform default`
> `            AkUInt32 in_uBlockLength,  ///< The block length, passing 0 will default to the platform default`
> `            AK::Wwise::IProgress* in_pProgress,`
> `            IWriteString* io_pError`
> `            ) = 0;`

with a companion `GetCurrentConversionSettingsHash` whose parameters are the same
"target sample rate for the converted file" and the same block length.

Read plainly, that interface makes the converter the component that opens the
source, decides what the source's own format means, and produces the destination
at the target rate. Three things about it are **not confirmed** here and must not
be assumed:

1. **Whether Wwise's Vorbis encoder is such a converter.** The header says the
   interface exists and how the host calls it; it does not say which plug-ins
   implement it, and the public include set in that repository contains the
   decoder-side factory (`AkVorbisDecoderFactory.h`) and no encoder-side one.
2. **Whether the 2013.2 SDK's copy of this header says the same thing.** That
   repository publishes only the 2016.1, 2017.1 and 2017.2 lines; there is no
   2013.2 branch, and the current 2013-era headers are not otherwise published.
3. **What Audiokinetic's own documentation says about accepted source formats and
   about a source whose rate or channel count does not match the target.** Every
   `audiokinetic.com` path tried from this environment — the library pages for the
   conversion settings and the audio file formats, and the release-notes document
   path — answered with an access challenge rather than content. So the vendor's
   documented list of accepted formats, and any sentence saying that Wwise
   converts or does not convert a mismatching source, could not be read at all.
   **Not confirmed**, in both directions.

### 2.5 The PCM containers that are not this repository's

**AIFF and AIFF-C.** Apple's own format table is readable and fixes the
representation: AIFF carries the big-endian integer forms of 8, 16, 24 and 32
bits, and AIFC adds the 32-bit and 64-bit float forms plus compressed types, with
the key "`BE` | Big Endian", "`F` | Floating point", "`I` | Integer", "`UI` |
Unsigned integer" — [Core Audio Overview, Documentation Archive, updated
2017-10-30](https://developer.apple.com/library/archive/documentation/MusicAudio/Conceptual/CoreAudioOverview/SupportedAudioFormatsMacOSX/SupportedAudioFormatsMacOSX.html).
The AIFF-C specification itself is reachable only as a scanned document that this
environment could not read, so whether it states any rule for converting its
samples to another bit depth or to little-endian is **not confirmed** — and
therefore "the specification is silent about conversion" is also not asserted
here.

**IEEE-float and 24-bit WAV.** Microsoft documents the container rules and stops
at the conversion. The bit-level rule that exists is about *layout*, not about
conversion — [WAVEFORMATEXTENSIBLE, ksmedia.h, last updated
2023-03-13](https://learn.microsoft.com/en-us/windows-hardware/drivers/ddi/ksmedia/ns-ksmedia-waveformatextensible):

> "If **wValidBitsPerSample** is less than **Format**.**wBitsPerSample**, the valid
> bits (the actual PCM data) are left-aligned within the container. The unused bits
> in the least-significant portion of the container should be set to zero."

> "Drivers should reject wave formats that violate these rules."

and the same documentation defines the container/valid-bits split
([Extensible Wave-Format Descriptors, last updated
2021-12-15](https://learn.microsoft.com/en-us/windows-hardware/drivers/audio/extensible-wave-format-descriptors)):
"Specify the number of bits per sample separately from the size of the sample
container. For example, a 20-bit sample can be stored left-justified within a
three-byte container." The only conversion sentences found there are permissive,
not rules: "A stream with a sample precision of 24 bits can use a 32-bit container
size for efficient processing, but can be converted to use a 24-bit container to
improve storage efficiency without loss of data", and "a rendering device that
provides only 20 bits of precision can use dithering to improve the fidelity of
its output signal". Where Microsoft states that a conversion *happens*, it states
no rounding rule for it
([Device Formats, last updated 2021-01-06](https://learn.microsoft.com/en-us/windows/win32/coreaudio/device-formats)):
"The audio engine will convert the floating-point samples in the output mix to
16-bit integers before playing them through the device."

The nominal range of IEEE-float WAV samples is **not confirmed** from Microsoft's
documentation: no page consulted states it, and `mmreg.h` itself was not
inspected. The format tags are confirmed from documentation tables:
`WAVE_FORMAT_IEEE_FLOAT` is 0x0003 and `WAVE_FORMAT_PCM` is 0x0001 ([Audio Subtype
GUIDs, last updated
2026-07-08](https://learn.microsoft.com/en-us/windows/win32/medfound/audio-subtype-guids)).
The classic IBM/Microsoft "Multimedia Programming Interface and Data
Specifications 1.0" (August 1991) could not be read at all, so its text is **not
confirmed**.

**RF64 and BW64.** The accessible standards-body text is the ITU's, not the
EBU's; the EBU publication pages answered with an access block from this
environment. ITU-R BS.2088-2 (11/2025), "Long-form file format for the
international exchange of audio programme materials with metadata", states its own
scope
([Summary](https://www.itu.int/dms_pubrec/itu-r/rec/bs/R-REC-BS.2088-2-202511-I!!SUM-HTM-E.htm)):

> "This Recommendation contains the specification of the Broadcast Wave 64Bit
> (BW64) audio file format including the new chunks <ds64>, <axml>, <bxml>, <sxml>
> and <chna>, which enable the file to carry large multichannel files and metadata
> including the Audio Definition Model (ADM) specified in Recommendation
> [ITU-R BS.2076]."

Its table of contents carries "Achieving compatibility between RIFF/WAVE and BW64"
and, in the informative annex on RIFF, only packing, sample-format and storage
sections — no conversion, rounding, dither or resampling clause
([Table of
Contents](https://www.itu.int/dms_pubrec/itu-r/rec/bs/R-REC-BS.2088-2-202511-I!!TOC-HTM-E.htm)).
That is scope-level silence only: the normative body is a document this
environment could not read.

**Wave64 (`.w64`).** No first-party Sony specification was reachable, so nothing
about its layout or its conversion rules is claimed here. **Not confirmed.**

### 2.6 Sample-rate conversion, in the sources that own one

Nothing found in this survey states a bit-exact resampling requirement, and the
documents that own a resampler say the opposite in their own words.

**RFC 6716 (Opus), Standards Track, September 2012**, states both the scope of
codec normativity and the status of resampling
([rfc6716](https://www.rfc-editor.org/rfc/rfc6716.txt)):

> "The primary normative part of this specification is provided by the source code
> in Appendix A. Only the decoder portion of this software is normative, though a
> significant amount of code is shared by both the encoder and decoder."

> "The resampler itself is non-normative, and a decoder can use any method it wants
> to perform the resampling."

The same document's encoder description puts rate conversion inside the encoder's
front end without specifying it — "The input signal's sampling rate is adjusted by
a sample rate conversion module so that it matches the SILK internal sampling
rate" — and even decoder conformance is defined as a tolerance rather than an
equality:

> "Compliance with this specification means that, in addition to following the
> normative keywords in this document, a decoder's output MUST also be within the
> thresholds specified by the opus_compare.c tool (included with the code) when
> compared to the reference implementation for each of the test vectors
> provided."

with the reference implementation itself stating "The normative behavior is
defined as the output using the floating-point configuration."

**The Vorbis I specification** (Xiph.Org Foundation, July 4, 2020,
[xiph.org](https://xiph.org/vorbis/doc/Vorbis_I_spec.html)) is a decode
specification: "This document serves as the top-level reference document for the
bit-by-bit decode specification of Vorbis I." It specifies no encoder output.

**FFmpeg's resampler** is the clearest statement that a rate conversion is a
parameterised choice: it exposes the engine itself as an option
([FFmpeg Resampler Documentation](https://ffmpeg.org/ffmpeg-resampler.html)):

> "resampler — Set resampling engine. Default value is swr. Supported values:
> 'swr' select the native SW Resampler; […] 'soxr' select the SoX Resampler (where
> available)"

and then a page of knobs that each move the output: "filter_size For swr only, set
resampling filter size, default value is 32", "phase_shift For swr only, set
resampling phase shift, default value is 10", "cutoff … Default value is 0.97 with
swr, and 0.91 with soxr", "precision For soxr only, the precision in bits to which
the resampled signal will be calculated. The default value of 20 …", plus named
dither methods for the bit-depth step.

**Apple's converter** documents the same thing as a quality setting rather than a
result: its sample-rate converter complexity values are documented as "Linear
interpolation. lowest quality, cheapest.", "Normal quality sample rate
conversion.", "Mastering quality sample rate conversion. More expensive.", and
"Minimum phase impulse response. Stopband attenuation varies with quality
setting." with noise floors of -96 dB, -144 dB and -160 dB for the three quality
levels.

So the answer to "can bit-exactness survive a resampler" is not that the sources
disagree: it is that **no source makes the claim in the first place**. The
strongest statement found is a specification declaring its own resampler
non-normative.

### 2.7 The Rust ecosystem's decoder options, and what each costs

Read from each crate's own manifest, README and docs pages; the version of each
is named.

| Candidate | What it reads | Runtime dependencies | License | `wasm32-unknown-unknown` |
| --- | --- | --- | --- | --- |
| `symphonia` 0.6.1 | Wave, AIFF, CAF, ISO/MP4, MKV/WebM, OGG containers; PCM, ADPCM, FLAC, MP1/2/3, Vorbis, AAC-LC, ALAC decoders (its README's own status tables; Opus and WavPack are listed with the "in work or not started yet" status) | The meta-crate declares 15 direct dependencies (3 mandatory, 12 optional); across the default feature set, 9 third-party crates are declared directly, and the transitive graph is larger and was not counted | MPL-2.0 | **Not confirmed.** The README lists "Providing a WASM API for web usage" as a *planned* feature, and the project's CI matrix tests four native targets with no wasm target |
| `hound` 3.5.1 | WAVE only: "Hound can read and write the WAVE audio format" (integer PCM and IEEE float, 8/16/24/32 bits) | 0 (the manifest has no `[dependencies]` section; the one dev-dependency is not shipped) | Apache-2.0 | **Not confirmed** (no statement in its README or manifest) |
| `ffmpeg-next` 9.0.0 with `ffmpeg-sys-next` | everything FFmpeg reads | a C toolchain and the system FFmpeg libraries: the project's build notes require "clang, pkg-config and FFmpeg libraries (including development headers)"; build-dependencies are `cc`, `pkg-config`, `bindgen` | bindings WTFPL; the libraries are LGPL-2.1-or-later with GPL parts | **Not confirmed**; nothing in the crate's docs addresses it |
| `ffmpeg-sidecar` 2.5.2 | whatever the `ffmpeg` binary on the machine reads | runs a child process; default feature downloads an FFmpeg build at run time | MIT | **Not confirmed** |
| `claxon` 0.4.3 | FLAC | 0 | Apache-2.0 | **Not confirmed** |
| `lewton` 0.10.2 | Vorbis | 5 declared (3 optional) | MIT OR Apache-2.0 | **Not confirmed** |
| `puremp3` 0.1.0 | MP3 (MPEG-1/2/2.5 Layer III) | 2 | MIT OR CC0-1.0 | **Yes** — the only one of these with a first-party claim: "The motivation for this crate is to create a pure Rust MP3 decoder that easily compiles to the `wasm32-unknown-unknown` target." Its README also says "This project is currently unmaintained and probably not suitable for production" |

`opusenc`'s own treatment of format support is the closest thing to a precedent
for how to price this: it takes FLAC input through a separate library that is
optional at build time, and the whole rest of its input handling is header
parsing.

## 3. The norms, distilled

"Rule" means a specification or a first-party API document states it as required
behaviour. "Documented API behaviour" means a library's own header documentation
states how the library must be called and what it will not do. "Practice" means the sources
document the same behaviour consistently but do not require it. "Silence" means no
source was found that addresses it.

| # | The norm | Support | Who states it |
| --- | --- | --- | --- |
| N1 | An encoder's API takes decoded PCM (or raw frames), not a container or a compressed stream. | **Documented API behaviour**, in all five codec libraries surveyed, with no counter-example at the encoder level | [x264.h](https://code.videolan.org/videolan/x264/-/raw/master/x264.h) (`x264_image_t`, "encode one picture."); [vpx_encoder.h](https://github.com/webmproject/libvpx/blob/main/vpx/vpx_encoder.h) ("Image data to encode"); [aom_encoder.h 3.6.0](https://sources.debian.org/data/main/a/aom/3.6.0-1+deb12u2/aom/aom_encoder.h); [lame.h](https://github.com/lameproject/lame/blob/master/include/lame.h) (`short int pcm[]`); [opus.h](https://github.com/xiph/opus/blob/main/include/opus.h) ("Input signal (interleaved if 2 channels)") |
| N2 | The caller declares the input's geometry, and the encoder neither infers it nor converts it: a mismatch is the caller's to fix. | **Documented API behaviour**, stated in the words "the application is required to" and "must" | libvpx `g_input_bit_depth` ("the application is required to upshift the frame to the codec bit-depth before passing it for encoding"); libaom (`g_input_bit_depth`); libopus (`Fs` "must be one of 8000, 12000, 16000, 24000, or 48000"); libvpx `g_w`/`g_h` ("the frames passed as input to the encoder must have this resolution") |
| N3 | Reading containers and decoding compressed audio live in a separate component from the codec: a front end, or another library the front end links. | **Practice**, documented by each project's own front end — and in one case by the library's own build options | x264's CLI demuxer layer ("Specify input container format", modules `raw`, `y4m`, and optionally `avs`, `lavf`, `ffms`); [opusenc 1](https://github.com/xiph/opus-tools/blob/master/man/opusenc.1) ("reads audio data in Wave, AIFF, FLAC, Ogg/FLAC, or raw PCM … format"), with FLAC as an optional build dependency; LAME's man page ("looking for WAV and AIFF headers", `--mp3input`); LAME's optional mpglib decoder |
| N4 | An API may instead own the file and the conversion — and when it does, it still arranges for the caller's side to be PCM, and it exposes the implementation as a choice. | **Documented API behaviour** (Apple). **First-party authoring API** (Wwise), with the caveats in 2.4 | Apple `kExtAudioFileProperty_ClientDataFormat` ("The format must be linear PCM"), `AudioConverter` ("PCM sample rate conversion", "PCM float/integer/bit depth conversions"), `kExtAudioFileProperty_CodecManufacturer`; Wwise `IPluginMediaConverter::ConvertFile` (`in_szSourceFile`, `in_uSampleRate`) |
| N5 | A sample-rate or bit-depth conversion is an approximation whose output depends on the method chosen; the method is exposed, not fixed. | **Rule** for Opus's resampler ("non-normative … any method it wants"); **documented API behaviour** for FFmpeg (engine and filter options) and Apple (quality levels) | [RFC 6716 §4.2.9](https://www.rfc-editor.org/rfc/rfc6716.txt); [FFmpeg Resampler Documentation](https://ffmpeg.org/ffmpeg-resampler.html); AudioConverter's sample-rate converter complexity values |
| N6 | Where a specification does fix exactness, it fixes it for a **decoder**, and for lossy codecs it fixes it as a tolerance; no specification fixes an **encoder's** output bytes. | **Rule** | RFC 6716 ("Only the decoder portion of this software is normative"; conformance "MUST also be within the thresholds specified by the opus_compare.c tool"); [Vorbis I specification](https://xiph.org/vorbis/doc/Vorbis_I_spec.html) ("the bit-by-bit decode specification of Vorbis I") |
| N7 | No specification requires an encoder to accept more than one input format, and no specification defines a bit-exact resampler. | **Silence** — the survey found no such statement, not a statement against | — |
| N8 | PCM container standards fix **layout** (container size, valid bits, alignment, endianness) and do not fix **conversion** (rounding, dither, clipping) between representations. | **Rule-shaped** for layout ("Drivers should reject wave formats that violate these rules"); **silence** for conversion, where the only sentences found are permissive ("can be converted", "can use dithering") | [ksmedia.h WAVEFORMATEXTENSIBLE](https://learn.microsoft.com/en-us/windows-hardware/drivers/ddi/ksmedia/ns-ksmedia-waveformatextensible); [Extensible Wave-Format Descriptors](https://learn.microsoft.com/en-us/windows-hardware/drivers/audio/extensible-wave-format-descriptors); [Device Formats](https://learn.microsoft.com/en-us/windows/win32/coreaudio/device-formats) |

Two honest notes on the table:

- N1's strength is not that a specification mandates it. It is that five
  independent codec libraries document their input as samples, and that the one
  first-party API which owns decoding (Apple) and the one authoring converter that
  owns a source file (Wwise) both still define a PCM boundary or a PCM client
  format. "An encoder takes PCM" is therefore a **documented API convention across
  the field**, not a written rule of the field.
- N7 is the reason the question is a product decision. Nothing found here obliges
  this repository either way.

## 4. Where this repository stands

### 4.1 What the current shape matches

**N1, at every level.** The kernel's C ABI takes interleaved signed-16 PCM
(`wem_encode_pcm16_interleaved`, `wem_session_push`), the Python root exports
`PcmBuffer` and `RawPcm`, and the command line reads the WAV itself — the same
split x264's, LAME's and opusenc's front ends have
([`public-interface.md`](../reference/public-interface.md),
[`include/wem.h`](../../include/wem.h)).

**N2, more strictly than the surveyed encoders.** A source whose channel count or
sample rate has no installed profile is refused as `GEOMETRY_MISMATCH` rather than
converted or accepted. The surveyed libraries can do that because a caller hands
them the geometry for the stream they want; this repository has two fixed
configurations, so a mismatching source is not a stream to encode at all — it is a
selection that resolves to no profile, and the selection rules already say what
happens then (`docs/reference/profiles.md`: "A selection no installed profile
satisfies is an error that names the installed configurations. It is never
replaced by a default or by a neighbouring geometry").

**N3, for PCM containers.** The facade's adapter layer and the Rust command line
own the WAV read; the kernel never sees a file path, and the reference tree is
never a runtime path. `docs/reference/architecture.md` already places "external
PCM/WAV inputs" as a facade duty under `application`, so an input-format layer
that sits there is the position the architecture already reserves for it.

**N5's honesty, without the machinery.** The repository has no resampler at all,
so it makes no approximation claim: a rate it does not carry is refused.

**N8's split, on its own terms.** The WAV reader already accepts signed 24-bit PCM
and 32-bit IEEE float WAV and converts at the adapter boundary by the
repository's own documented integer rules, and the public interface states the
boundary of the claim:

> "24-bit and float input is deterministically converted into the signed-16
> domain. The bit-exact Wwise guarantee applies to signed-16 input; converted
> input is deterministic and matches the repository's reference encoder after
> conversion."

That is exactly the shape N8 leaves open: the container is parsed, the conversion
is the implementation's own, and the guarantee is narrowed to the representation
the guarantee was established on.

### 4.2 What it does not match

- **Wwise's own converter takes a file.** The reachable vendor interface
  (2.4) hands the conversion routine `in_szSourceFile` and a target
  `in_uSampleRate`. If the 2013.2 encoder that this repository reproduces is
  driven that way, then this repository's public input is a *narrowing* of the
  reproduced component: the codec core without its file-opening and
  format-handling front. Nothing in the reachable headers says the 2013.2 encoder
  is a media converter, and Audiokinetic's own documentation of accepted formats
  and of conversion behaviour could not be read, so this is a gap in the record
  rather than a known mismatch.
- **Apple's API takes a URL and converts.** This repository refuses a geometry it
  has no profile for; `ExtAudioFile` would convert to it. That is a design
  difference, not a norm violation: the repository's two configurations are
  calibrated against one build, and N2 is the shape those libraries use too.
- **The front end's accepted set is narrower than any surveyed front end's.**
  x264's CLI, opusenc and LAME's CLI each read more than one container;
  `wwise-wem` reads RIFF/WAVE, with three sample representations.

### 4.3 What the sources are silent about — and the one that decides it

The sources are silent on N7, which is the licence for the current shape: no
specification or first-party API document says an encoder *must* accept more than
one input format. The surveyed front ends exist because the programs' users have
files in many formats, not because a rule required it.

The deciding question is narrower than that, and it is the one this survey cannot
close:

> **What does Wwise's own authoring and conversion pipeline do when the source's
> sample rate or channel count does not match the target profile's?**

Three readings are possible, and they have different consequences:

1. **The pipeline converts before the encoder runs** (a host-owned resampler). Then
   "same bytes as Wwise" for a non-matching source requires reproducing that
   resampler, because the encoder's input samples would be that resampler's
   output.
2. **The converter plug-in converts internally** (which is what the reachable
   `ConvertFile` signature suggests, since it receives the source file and the
   target rate). Then the same requirement lands on the plug-in, and the
   conversion is inside the component being reproduced.
3. **A non-matching source is not converted at all** — the target geometry is
   whatever the source already has. Then there is nothing to reproduce, and the
   accepted input is exactly "a source at an installed geometry", which is the
   current shape.

Under readings 1 and 2, this repository's claim has to stay where
`public-interface.md` already puts it: byte-exactness is established for input
whose geometry the profile names. Under reading 3, the WAV-only input is not a
limitation at all — it is the whole of what the reproduced component was ever
given. **Which reading is correct was not established here**, because
Audiokinetic's documentation host answered with an access challenge on every path
tried, and the published SDK headers cover 2016.1 and later only. That is the
single largest gap in this survey, and it is named again in section 7.

Two smaller silences matter to the byte-exactness claim:

- **No source defines a bit-exact resampler** (N5, N7). So even a decision to
  accept other formats *and* resample internally could not be checked against a
  specification — it would be checked against one implementation's output, which
  is precisely the position `standards.md` refuses for the decode direction
  ("a comparison against it would be a comparison against the machine's
  libraries, not against a specification").
- **No source defines the conversion between PCM representations** (N8). This
  repository's 24-bit and float rules are its own; they are deterministic and
  documented, and they are already outside the bit-exact claim.

### 4.4 The precedent in this repository

Three precedents bear on the options; each was a decision to remove rather than
add, and each is the kind of change this question would be.

- **A layer that gave a value a second home was removed.** The serialized profile
  tree and its intake existed, and were deleted in favour of generated Rust
  constants, because the identity could address the value and nothing else needed
  to ([`profile-as-code.md`](profile-as-code.md), and
  [`architecture.md`](../reference/architecture.md): "There is no index, no
  manifest, no resource path and no digest chain", with the deletion list for
  `load_profile_bundle`, `ProfileBundle` and `DataDir` in the finding). The
  failure shape to avoid is a second place where the sample domain is decided.
  Today the adapter boundary is the one such place, and the conversion helpers say
  so ("These helpers are the single place where non-16-bit PCM samples are mapped
  into the signed-16 domain").
- **A dependency was removed rather than added.** The repository commits to
  carrying no digest in any result ("No result carries a digest", and the change
  whose subject reads "a hash is the produced artifact's digest and nothing
  else"). The direction of travel is fewer dependencies, not more, which is the
  strongest local argument against option (d).
- **An external decoder was refused as an oracle.** The one route that would make
  a decode-side claim checkable binds the host's libvorbis and is documented as
  not byte-stable across environments. That is the repository's existing position
  on decoder-derived evidence, and it applies with equal force to any decoder
  shipped to accept more input.

The brief for this survey also names a deleted decoder-adjacent piece of the
encode path; that change was not located in the history read for this survey, so
nothing here rests on it.

**Would an input-format layer violate the layer rules?** Not in the position the
architecture already reserves: an adapter under `application` that produces the
typed signed-16 PCM source is the existing shape, and reading one more wrapper or
representation there changes no dependency direction. Three placements would
violate the rules as written:

- decoding inside `scheduling`, `analysis`, `vorbis` or `container` — those are
  pure and resource-free, and `analysis` "receives typed configuration; it does
  not open package resources";
- a kernel or C ABI entry point that takes a path — the library reads no ambient
  state, a configuration is "named by a structured profile selection, never by a
  variable or a path", and the C ABI's shape is PCM in, callbacks out;
- a second integration surface kept in parallel with the C ABI for a decoder —
  "a new language integrates by writing a shim over the C ABI, never by changing
  the kernel for it".

A decoder added to the kernel would also change the portability floor rather than
a layer boundary: it would be the first third-party runtime dependency in a build
that must stay compilable for `wasm32-unknown-unknown`, while no first-party
source found here states that any of the candidate libraries builds for that
target.

## 5. The options

No option is recommended here; the table is the input to the decision. Costs are
stated as the sources document them, and the byte-exactness column says what each
option does to the existing claim — not what a future claim could be.

| Option | Which norm it satisfies or breaks | Dependencies and portability | Effect on the byte-exactness claim | Documented consequence |
| --- | --- | --- | --- | --- |
| **(a) PCM-only; decode elsewhere.** The accepted input stays a signed-16 PCM source; a caller with another format decodes it with something else and hands in samples. | Satisfies N1, N2, N3, N7. Breaks nothing found. | None added. The portability floor is untouched: the kernel, the C ABI and the wasm shell are unchanged. | Unchanged. The claim keeps the exact left-hand side it has today, and the 24-bit/float conversions stay where `public-interface.md` already puts them. | This is what the five surveyed codec libraries all do at their API, and what this repository already does at three levels. A caller that decodes in Python already needs nothing new: `PcmBuffer` and `RawPcm` are the interface, with the geometry stated explicitly because "raw PCM geometry cannot be inferred safely". |
| **(b) PCM-only plus a documented converter path, with the exact command that produces the accepted input.** | Satisfies N1, N2, N3, N7; makes N5 visible to the user instead of hiding it. | None added. Cost is documentation, plus the obligation that the command in the document is a live artifact. | Unchanged, provided the document says plainly that the conversion is outside the claim — the input the library receives is still signed-16 PCM at the profile's geometry. | The surveyed front ends are this shape already: the container reading is one component and the codec another. The documented form for an arbitrary source would be a converter invocation that resamples and reduces depth, e.g. `ffmpeg -i input.mp3 -ac 2 -ar 48000 -c:a pcm_s16le output.wav`, or `sox input.mp3 -c 2 -r 48000 -b 16 output.wav`. The first of those calls the resampler documented in 2.6 — so the document has to say that the resampling method is the converter's choice, not this library's. Neither command was run for this survey. |
| **(c) A pluggable decoder interface the caller implements, with no decoder shipped.** | Satisfies N4's *caller chooses the implementation* half, and N1–N3 if the interface's payload is PCM. Costs a new public surface. | None added to the build; the portability floor is untouched because nothing is linked in. But it is a new surface in three languages and the C ABI, and `standards.md` requires the shells to mirror the C ABI 1:1 — so the interface has to exist at the kernel boundary, not only in Python. | Unchanged, and it is the one format-widening option that leaves it unchanged by construction: whatever the caller decodes, the library still receives a signed-16 PCM source at the profile's geometry. | The minimal form of this already exists and is free: a caller hands in `PcmBuffer` or `RawPcm`, and the library never learns what produced it. What a formal interface adds is that the *library* drives the caller's decoder, which buys the CLI case (a path in, no Python in the loop) and costs the "library calls out to caller code" property the integration topology does not currently have. The two documented precedents for a caller-chosen codec backend are Apple's codec-manufacturer property and Wwise's own plug-in boundary, where the converter is a separate module assembled by the host. |
| **(d) A built-in set of decoders.** | Satisfies a requirement that no source states (N7 is silence, not a rule). N1–N3 still hold if the decoders hand PCM inward. | The first third-party runtime dependency in the project, and the largest cost in this table: `symphonia` 0.6.1 declares 15 direct dependencies in its meta-crate (3 mandatory) and 9 third-party crates directly across its default graph, under MPL-2.0, with `wasm32-unknown-unknown` support **not confirmed** by any first-party source; the FFmpeg bindings require system libraries and a C toolchain and are documented as LGPL-2.1-or-later with GPL parts; only `puremp3` makes an explicit wasm claim, and it is unmaintained. The zero-dependency build and the wasm shell are the properties at risk. | Unchanged for PCM and lossless sources; **changed** for lossy sources and for any decoder that resamples. A decoded lossy stream has no single correct sample value — Opus's own conformance is a tolerance ("MUST also be within the thresholds specified by the opus_compare.c tool"), so two conforming decoders need not agree — and a decoder that converts rates would put an unspecified resampler inside the claim. It also weakens the claim's *shape*: the guarantee would become "same bytes as the paired build, given the PCM that this decoder produced", which is a different sentence. | The nearest published precedent prices it as optional: `opusenc` reads FLAC only when the build has libFLAC, and offers `--without-flac`; the rest of its input handling is header parsing. A built-in set also has to answer which of the decoders' outputs the guarantee is anchored to, and no surveyed document anchors one. |
| **(e) The PCM-container subset only: AIFF/AIFC, IEEE-float and 24-bit WAV, RF64/BW64 (and Wave64 if a first-party specification were obtained).** | Satisfies N8's split: more wrappers and representations, and no new conversion rule beyond the ones already documented. Satisfies N3. | No dependency: parsing code, not a library. Portability is unaffected. The real cost is that every added representation needs its own conversion rule, which is one more place where the sample domain is decided. | Unchanged for signed-16 sources in those containers. The float and 24-bit conversions remain the repository's own rules, already documented as outside the claim. RF64 is a size problem rather than a sample problem, so it adds no conversion at all. | 24-bit WAV and IEEE-float WAV are **already accepted**, so (e) is a smaller change than its name suggests: the delta is AIFF/AIFC (big-endian integer PCM per Apple's format table), and RF64/BW64 (the `<ds64>` chunk, per ITU-R BS.2088-2's scope). Nothing found states a conversion rule for any of them — for AIFF the specification itself could not be read, and for W64 no first-party specification was reachable — so each rule would be this repository's own, which is exactly how the 24-bit and float rules are already described. |

Two remarks that are not recommendations, only properties of the table:

- (a), (b) and (c) are not alternatives to each other. (b) is (a) plus a
  document, and (c) is what a caller can already do informally, promoted to a
  public surface. (e) is orthogonal: it widens the wrapper and representation
  without touching the compression question at all.
- Every option that *keeps the input signed-16 PCM at a profile's geometry*, in
  any wrapper, leaves the byte-exactness claim untouched. The only options that
  can move it are those that put a decoder or a resampler between the caller's
  bytes and the encoder's input — and per 2.6, no source defines what such a
  component must produce.

## 6. What would falsify the current shape

Named, specific statements that — if they exist — would require shipping a
decoder, a converter, or both. Each says where it would have to come from and
what it would change.

1. **A Wwise document stating that the encoder is handed the original source file
   and must produce the target geometry from it.** The reachable half of this
   exists: `IPluginMediaConverter::ConvertFile` takes `in_szSourceFile` and
   `in_uSampleRate`, and the destination "must be created by the plug-in". If a
   2013.2 copy of that interface, or Audiokinetic's conversion-settings
   documentation, says the encoder plug-in is driven that way, then the thing
   this repository reproduces accepts files, and the WAV-only surface is a
   narrowing rather than a faithful subset. That would make accepting a file path
   a conformance question instead of a convenience one.
2. **A Wwise document stating that a source whose sample rate or channel count
   differs from the target's is converted during conversion, together with a
   specification or observable definition of that conversion.** Then "same bytes
   as Wwise" for such a source would be a target this repository could implement
   — but only by reproducing that conversion, which is not specified in any
   document reachable here.
3. **A specification or first-party API document requiring an encoder to accept
   container or compressed input.** None was found: the five surveyed codec APIs
   document the opposite (N1, N2), and the two APIs that do take a file (Apple's
   and Wwise's) are a converter and an authoring host, not codec APIs. If one
   existed, it would be the rule N7 records as absent.
4. **A normative statement anywhere that a sample-rate conversion is bit-exact, or
   that a converter must agree byte for byte with another implementation.** None
   found; the one specification that owns a resampler declares it non-normative
   (N5), and the one standards-track codec surveyed defines decoder conformance as
   a tolerance (N6). If such a statement existed, options that resample would stop
   being automatic disqualifications for a byte-exactness claim.
5. **A first-party statement that a candidate decoder builds for
   `wasm32-unknown-unknown` and is compatible with shipping here.** That would
   remove the portability objection to option (d) — the dependency count and
   license questions would remain, and the lossy-input caveat would not move.
6. **A statement, from the owner or from a documented requirement rather than from
   a norm, that the supported formats are a product requirement.** The sources
   surveyed cannot supply this: it is the one input to the decision that has to
   come from outside them, and it decides the question more directly than anything
   in this file.
7. **Evidence that the paired build itself was driven with a source that needed
   conversion.** If the reference containers behind `docs/findings/` were produced
   from sources whose geometry already matched the target, then the byte-exact
   claim has never covered a converted source, and widening the accepted formats
   would be widening the *unverified* region rather than the verified one.

## 7. What could not be confirmed

- **Audiokinetic's own documentation.** Every `audiokinetic.com` path attempted
  answered with an access challenge: the library pages for the conversion
  settings and the audio file formats, and the release-notes document path. So the
  vendor's documented list of accepted source formats, its documented handling of
  24-bit and float sources, and any documented statement about a source whose
  sample rate or channel count does not match the target are all **not
  confirmed**. Whether Wwise converts such a source is therefore the open question
  of this survey, and section 4.3 states what follows from each possible answer.
- **Whether the Wwise Vorbis encoder implements the media-converter interface**,
  and whether the 2013.2 SDK's copy of that header matches the 2016.1 and 2017.2
  branches read here. The published repository has no 2013.2 line.
- **The AIFF-C specification's text.** It is reachable only as a scanned document
  that this environment could not read, so its wording on byte order, sample size
  and any conversion rule is **not confirmed**, and neither is the claim that it
  is silent about conversion.
- **The classic IBM/Microsoft WAVE specification** ("Multimedia Programming
  Interface and Data Specifications 1.0", August 1991). Not readable here; the
  Microsoft statements quoted in 2.5 come from current `learn.microsoft.com`
  pages, each with its "last updated" date given above.
- **The nominal range of IEEE-float WAV samples.** No Microsoft page consulted
  states it, and `mmreg.h` itself was not inspected. Not confirmed rather than
  proven absent.
- **EBU Tech 3285 and Tech 3306.** The publication pages answered with an access
  block and the documents themselves were unreadable; ITU-R BS.2088-2 was used as
  the standards-body source for the `<ds64>` chunk instead, and its normative body
  remains unread (only its scope and table of contents were).
- **Sony's Wave64 specification.** No first-party document was reachable, so
  nothing about `.w64` is claimed.
- **`wasm32-unknown-unknown` support for every decoder candidate except
  `puremp3`.** No first-party statement was found for `symphonia`, `hound`, the
  FFmpeg bindings, `ffmpeg-sidecar`, `claxon` or `lewton`; `puremp3`'s README
  states it explicitly, and the same README calls the crate unmaintained.
- **The transitive dependency closure of `symphonia`'s default feature set.** The
  counts in 2.7 are direct declarations read from manifests; the resolved graph
  is larger and was not counted.
- **Whether `ffmpeg-next` can be built for wasm at all.** Not addressed by any of
  its own documentation.