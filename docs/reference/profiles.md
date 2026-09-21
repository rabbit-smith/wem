# Profile model

A caller selects a configuration with one structured value — `WwiseProfile`: a
`WwiseVersion`, a channel count and a sample rate. Never with a profile name, a
profile directory, profile index/manifest bytes, or an environment variable.
Inside the kernel, `ProfileKey` carries the full immutable identity
(generation, geometry, channel layout, setup identity) and is an exact lookup,
never a request to synthesize or approximate a configuration.

The two types have one spelling per language and are the same contract
everywhere: Rust `WwiseVersion` / `WwiseProfile`, C `WemVersion` / `WemProfile`
(see [`include/wem.h`](../../include/wem.h), ABI revision 2), Python
`WwiseVersion` / `WwiseProfile`. Version codes are stable and append-only, like
the C ABI error values.

## Selection rules

- The generation participates in resolution. Geometry alone is not an identity:
  once two generations are installed with the same geometry, a geometry-only
  lookup could not tell them apart.
- A selection no installed profile satisfies is an error that names the
  installed configurations. It is never replaced by a default or by a
  neighbouring geometry.
- A selection more than one installed profile satisfies is an error, not a
  first-match pick.
- An unrecognized version code is a contract violation against the current
  revision, not a fallback to the newest known generation.

An installed profile owns its complete calibration set:

- the Wwise Vorbis setup packet and codebooks;
- channel mask, mapping and coupling configuration;
- short/long block geometry;
- psychoacoustic, transient-selector and floor/residue tables;
- default container metadata.

Changing RIFF header values is insufficient to add a geometry. A new
channel/rate pair is registered only after its complete setup and analysis tables
pass packet-level and whole-file regression.

## Installed profiles

| Selection | Name | Status |
| --- | --- | --- |
| `Wwise2013, 6, 44100` | `wwise2013-6ch-44100` | Byte-exact for the paired build; whole-file golden, per-frame and per-stage contracts |
| `Wwise2013, 2, 48000` | `wwise2013-2ch-48000` | Byte-exact for the paired input and both real-build corpora; evidence and limits in [`../findings/2ch-byte-exactness.md`](../findings/2ch-byte-exactness.md) |

### 2ch/48 kHz provenance

Every field of the 2ch analysis resource set is classified against the paired
encoder build through the `(2, 45000, 50000)` descriptor family, whose
30-pointer block is word-identical to the 6ch record of the same build.

- **Statically hosted arrays** — quality curves, transient record family, look
  envelopes and remap tables, frozen window halves, MDCT trig blocks — are
  byte-verified against the paired build.
- **Materialized geometry** — the remaining psychoacoustic surfaces (ATH,
  octave, mask, interval, 1024-bin curves, tone and mode variants) have no static
  host in the build: they are init-time materializations of deterministic
  geometry. The short surfaces are reproduced by the checked geometry
  materializer. The long geometry surfaces and the complete 17×8 tone bank were
  read from the running 48 kHz build, and the directly readable prefixes of both
  long profile records match the registered words.
- Container layout constants are paired-build values, while content-derived size
  and rate fields are recomputed for each encode.

No profile value is fitted to an output; the provenance rule is in
[`../methodology/byte-exact-diagnosis.md`](../methodology/byte-exact-diagnosis.md).

## Exact setup identity

A profile selection resolves to exactly one installed `EncoderProfile`. The
packaged setup packet is checked against its declared SHA-256 whenever it is
loaded; missing geometry, an unsatisfiable or ambiguous selection, or a
checksum mismatch is an error. The built-in path is self-contained: it reads
packaged profile resources and never reads a reference WEM. Golden expected
outputs are test assets, not runtime inputs.

## Frozen transcendental tables

Each profile freezes its complete runtime transcendental domain into
`analysis/frozen-tables.json` (schema `wem.frozen-math.v1`), checksum-addressed
through the manifest. Four enumerable libm sites are table-backed: the 128
short-psy coordinate frequencies, the FFT stage twiddles for lengths 2..2048, and
the patched 256/2048 Vorbis window halves. Assembly injects them as typed
`FrozenMathTables`; a construction path that requests an input outside the frozen
domain fails loudly instead of consulting the host libm.

Two named entries (`floor_fit` dB quantization and the `residue` classification
fallback) do not fire for the installed exact profiles. Should a future geometry
activate them, ports must pin a reference double `log`/`log10` implementation and
verify bit equality against the per-site records under
`tests/data/stage-golden/transcendental/`.

Regenerate the payload with `scripts/generate_frozen_tables.py`, which
cross-checks the live domain recorded by `scripts/record_tmath.py`. The golden
WEM bytes and frame-contract hashes must not change afterwards.