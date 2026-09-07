# Profile model

The codec generation is fixed to Wwise 2013.2. `ProfileKey` selects immutable
encoder configuration by channel count and sample rate.

An installed profile owns:

- Wwise Vorbis setup packet and codebooks;
- channel mask, mapping and coupling configuration;
- short/long block geometry;
- psychoacoustic, transient-selector and floor/residue tables;
- default container metadata.

Changing only RIFF header values is insufficient. A new channel/rate pair is
registered only after its complete setup and analysis tables pass packet and
whole-file regression.

Currently installed: `ProfileKey(channels=6, sample_rate=44100)`.

## Exact setup identity

Profile resolution is exact. A channel-count/sample-rate key selects one
installed `WwiseVorbisProfile`; it is not a request to synthesize or approximate
configuration. The profile's packaged setup packet is checked against its
declared SHA-256 whenever it is loaded. Missing geometry, an unknown profile
name, or a packaged setup checksum mismatch is an error.

The built-in path is self-contained: it reads packaged profile resources and
does not read a reference WEM. Golden expected outputs are test assets, not
runtime profile inputs.

## Template adapter

Compatibility `template=PATH` support adapts WEM container metadata to the same
installed-profile model. A template may contribute RIFF endianness, `fmt`
metadata, seek-table bytes and extra chunks before `data`; fields derived from
the new stream are recomputed.

The template's setup packet is an identity assertion. Its `fmt` geometry must
resolve to an installed exact profile, and its setup must equal that profile's
packaged setup byte-for-byte and by SHA-256. A geometry-only match is
insufficient. An unknown or modified setup is rejected before mode selection
and audio analysis, even if the WEM is otherwise structurally valid. Template
audio packets do not participate in encoding.

## Frozen transcendental tables

The exact profile freezes its complete runtime transcendental domain into
`analysis/frozen-tables.json` (schema `wem.frozen-math.v1`), checksum-addressed
through the manifest. Four enumerable libm sites are table-backed: the 128
short-psy coordinate frequencies, the FFT stage twiddles for lengths 2..2048,
and the patched 256/2048 Vorbis window halves. Assembly injects them as typed
`FrozenMathTables`; an encoder construction path that requests an input outside
the frozen domain fails loudly instead of consulting the host libm.

The two remaining named entries (`floor_fit` dB quantization and the `residue`
classification fallback) do not fire for the installed exact profile. Should a
future geometry activate them, ports must pin a reference double `log`/`log10`
implementation (musl-derived) and verify bit equality against the per-site
input/output records under `tests/data/stage-golden/transcendental/`.

Regenerate the payload with `scripts/generate_frozen_tables.py` (which
cross-checks the live domain recorded by `scripts/record_tmath.py`); the golden
WEM bytes and frame-contract hashes must not change afterwards.
