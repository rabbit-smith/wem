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

Currently installed:
- `ProfileKey(channels=6, sample_rate=44100)` — `wwise2013-6ch-44100`,
  structure and psychoacoustics complete; encodes.
- `ProfileKey(channels=2, sample_rate=48000)` — `wwise2013-2ch-48000`,
  its Vorbis setup packet and codebooks (t97 floor plus the t282 residue
  table) are registered and framing-verified, and its full analysis
  resource set is registered, so it resolves by geometry/setup digest
  and encodes. Calibration basis: representative stream / behavior
  pairing. The psychoacoustic four-set (short seed/profiles, long
  base/modes) is geometry-derived (representative 44.1k curve arrays
  frequency-mapped to 48k); the transient detector is behavior-fitted
  on a representative 2ch/48k stream; the quality-curves semantics are
  partial (the `desc31.psy_double` curve is identified as pointing at
  the transient upper-band thresholds, and the assembly wiring of
  curve values into psychoacoustic fields is pending); the container
  aux fields stay partial pending behavior pairing.

## Exact setup identity

Profile resolution is exact. A channel-count/sample-rate key selects one
installed `WwiseVorbisProfile`; it is not a request to synthesize or approximate
configuration. The profile's packaged setup packet is checked against its
declared SHA-256 whenever it is loaded. Missing geometry, an unknown profile
name, or a packaged setup checksum mismatch is an error.

The built-in path is self-contained: it reads packaged profile resources and
does not read a reference WEM. Golden expected outputs are test assets, not
runtime profile inputs.

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
