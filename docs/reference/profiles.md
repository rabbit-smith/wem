# Profile model

A caller selects a configuration with one structured value — `WwiseProfile`: a
`WwiseVersion`, a channel count and a sample rate. Never with a profile name, a
profile directory, profile index/manifest bytes, or an environment variable.
Inside the kernel, `ProfileKey` carries the full immutable identity
(generation, geometry, channel layout, setup identity) and is an exact lookup,
never a request to synthesize or approximate a configuration.

The two types have one spelling per language and the same shape
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
- An unrecognized version code is rejected against the current revision, not
  treated as a fallback to the newest known generation.

An installed profile owns its complete calibration set:

- the Wwise Vorbis setup packet and codebooks;
- channel mask, mapping and coupling configuration;
- short/long block geometry;
- psychoacoustic, transient-selector and floor/residue tables;
- default container metadata.

Changing RIFF header values is insufficient to add a geometry. A new
channel/rate pair is registered only after its complete setup and analysis tables
pass packet-level and whole-file regression — and registering it means
generating a new table set into the carrier
(`scripts/generate_profile_code.py`), not shipping a payload.

## Installed profiles

| Selection | Label | Status |
| --- | --- | --- |
| `Wwise2013, 6, 44100` | `6ch/44100Hz/2013.2` | Byte-exact for the paired build; checked by the whole-file, per-frame and per-stage suites |
| `Wwise2013, 2, 48000` | `2ch/48000Hz/2013.2` | Byte-exact for the paired input and both real-build corpora; evidence and limits in [`../findings/2ch-byte-exactness.md`](../findings/2ch-byte-exactness.md) |

The label is derived from the identity (`ProfileKey::label`), never stored: a
profile has no name, no directory and no index entry left to carry one. The
recorded profile tree still exists as untracked development material under
`corpus/profiles/`, which is what the table generator and
`scripts/generate_frozen_tables.py` read.

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

A profile selection resolves to exactly one compiled profile. The setup packet
is a Rust constant and the identity the key carries is the SHA-256 of exactly
those bytes: the value model recomputes the digest from the packet when it
builds an identity from the carrier, so no stored copy can drift from them. An
unsatisfiable or ambiguous selection is an error, and so is a key whose setup
identity does not describe the packet it carries. The built-in path is
self-contained: it reads the compiled carrier and never reads a reference WEM.
Expected outputs are test assets, not runtime inputs.

## Access boundary

There is no intake. Profile data is Rust source in the kernel
(`crates/wem-profiles/src/generated/`), compiled into every artifact; no
library user can name a profile, point at a profile tree, or hand over
index/manifest bytes, because those concepts do not exist.

The one public way to obtain a profile is selection-keyed —
`compiled_profile_for_selection(WwiseProfile) -> Result<CompiledProfile, ProfileError>`
— which resolves against the tables compiled into the library under the same
exactly-one rule as the encoder. It exists because the stage and analysis
parity suites need a profile's *materials*, not its bytes, and they test layers
outside this crate.

The Python side has no loader either: the reference reads the same carrier
through the kernel's data hand-off
(`wwise_wem._core.profile_tables()` → `wwise_wem_reference.profiles.artifact`),
so there is no second copy to drift from it and no resource reader anywhere.
See [`../findings/profile-as-code.md`](../findings/profile-as-code.md) for the
change that removed the serialized layer.

## Frozen transcendental tables

Each profile freezes its complete runtime transcendental domain into the
carrier's `frozen.*` tables. Four enumerable libm sites are table-backed: the
128 short-psy coordinate frequencies, the FFT stage twiddles for lengths
2..2048, and the patched 256/2048 Vorbis window halves. Assembly injects them as
typed `FrozenMathTables`; a construction path that requests an input outside
the frozen domain fails loudly instead of consulting the host libm.

Two named entries (`floor_fit` dB quantization and the `residue` classification
fallback) do not fire for the installed exact profiles. Should a future geometry
activate them, ports must pin a reference double `log`/`log10` implementation and
verify bit equality against the per-site records under
`tests/data/stage-records/transcendental/`.

Regenerate the recorded payload with `scripts/generate_frozen_tables.py` (it
writes into the untracked `corpus/profiles/` tree), which cross-checks the live
domain recorded by `scripts/record_tmath.py`, then recompile it into the
carrier with `scripts/generate_profile_code.py`. Run `make wem-bytes` and the
per-frame parity suites afterwards: the reference WEM bytes and every per-frame
value must come out unchanged.