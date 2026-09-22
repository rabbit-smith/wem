# Architecture

This document defines the package boundaries for the encoder. The distribution
facade (package root `wwise_wem`) is a stable API shell; the bit-exact
implementation domains live in the development-tree reference package
(`wwise_wem_reference`, under `reference/`), which the test suites use as the
reference oracle. The facade has a single execution path — the in-package
native extension `wwise_wem._core` (built from `crates/wem-python`) — and
never imports the reference tree at runtime. The Rust kernel is checked against
the reference implementation's byte-for-byte output, never the reverse.

## Package layout

```text
wwise_wem/                      # distribution facade (wheel)
├── __init__.py, __main__.py, api.py, cli.py   # root API shell + CLI
├── model.py                     # root DTOs and the public error type
├── adapters/                    # external PCM/WAV inputs (facade duty)
├── application/                 # native-core orchestration and result DTOs
├── profiles/                    # profile identity (ProfileKey)
└── _core.abi3.so                # in-package native extension (built artifact)

wwise_wem_reference/            # development-tree reference oracle (not in wheel, test-time only)
├── python_engine.py             # direct-import oracle encode pipeline (tests only)
├── analysis/                    # MDCT, spectrum, LPC, transient, psychoacoustics, session
├── vorbis/                      # bitstream, setup, floor, residue and packet codecs
├── container/                   # RIFF/WEM models and codecs
├── scheduling/                  # immutable frame plans and mode-selection policy
└── profiles/                    # carrier reader (artifact.py), typed assembly
```

The package root exports one `encode` function plus its input/result value
types. Internal module paths are not part of that surface. The reference
package may import facade DTOs and profile-metadata types; the facade never
imports the reference tree (oracle parity is test-only, locked by the
parity suites).

## Dependency direction

Dependencies flow in one direction:

```text
root API / CLI
       │
       v
 application
   ├── adapters
   ├── profiles ──> typed analysis/Vorbis/container models
   ├── scheduling
   ├── analysis ──> scheduling + analysis.dsp
   ├── vorbis.packet_encoder ──> analysis models + Vorbis primitives
   └── container
```

Mandatory rules:

- `scheduling` does not import `analysis`, `application`, `profiles`, `vorbis`, or `container`.
- `analysis.dsp` is resource-free and does not import detector or profile loaders.
- `analysis` receives typed configuration; it does not open package resources.
- `vorbis` primitives do not import `application`, `profiles`, or `container`.
- `container` does not import `application`, `profiles`, or `analysis`.
- `profiles` is the only owner of the compiled profile carrier, codebook assembly, and calibrated table readers.
- The complete immutable `EncoderProfileResources` aggregate is assembled once from the compiled profile at encoder construction. Its nested analysis resources are injected into each analysis session, while its setup packet, parsed setup, and codebooks are injected into Vorbis packet encoding. Reader caches key on the resolved profile (its identity and the shapes of the blocks it carries).
- `application` is the only layer allowed to assemble profiles, analysis, packet encoding, and containers into the complete WAV-to-WEM use case.
- The internal import graph is acyclic.

## Encoding flow

`application.encoder` owns one deterministic conversion:

1. Resolve the compiled profile for the selected `WwiseProfile` identity.
2. Plan short/long frames and materialize PCM windows.
3. Run transforms, transient detection, and psychoacoustic analysis.
4. Fit floor1 and encode residue into Vorbis packets.
5. Compose the packet stream into a RIFF/WEM container.

Mutable cross-frame state is owned by one analysis session. A `FramePlan` is the sole scheduling truth and every `WindowedFrame` delegates its scheduling fields to that plan.

## Profile ownership

A profile owns its complete calibration set: setup, codebooks, transforms, transient tables, psychoacoustic tables, block geometry, and container defaults. Algorithms receive already validated typed tables. They never select a default file path.

Profile data is Rust source, generated once per installed configuration:

```text
crates/wem-profiles/src/
├── generated/
│   ├── mod.rs                     # PROFILES: every compiled profile
│   ├── codebooks/{t97,t219,t282}.rs
│   └── wwise2013_{2ch_48000,6ch_44100}.rs
└── tables.rs                      # the shape of the carrier
```

Each generated module is the profile's identity (`ProfileKeyParts`: generation,
channels, sample rate, channel layout, setup identity), its container geometry,
its setup packet bytes and SHA-256, and every typed table — MDCT banks, codebook
rows, frozen twiddles, transient mechanism, quality curves, short/long
psychoacoustic surfaces. Floats are stored as their IEEE bit patterns, never as
decimal literals, so the carrier cannot move a value.

There is no index, no manifest, no resource path and no digest chain: the
identity is the addressing, and the only way to reach a profile is a
`WwiseProfile` selection. The recorded JSON tree still exists as untracked
development material under `corpus/profiles/`; `scripts/generate_profile_code.py`
is the only path from it into the repository, and the generated source is what
ships.

`EncoderProfile` is the internal identity value (key, setup digest, container
defaults). The human label is derived from the key, never stored. Another
language reads the same tables through the kernel's data hand-off
(`wem_profiles::blob::profile_tables_blob`, exposed as
`wwise_wem._core.profile_tables()`), which is a transfer of the carrier's words,
not a second source.

## Geometry materializer parity

Psychoacoustic surfaces are init-time materialized geometry of the paired
build (2013.2 conversion plug-in, geometry materializer at module offset
0x15ee0). The port lives on three surfaces that must stay byte-identical:

- reference builder: `reference/wwise_wem_reference/geometry_materializer/`
  (per-instruction port; development/test asset, never a runtime path);
- profile data: the compiled carrier's `short_seed.*` / `short_profiles.*` /
  `long_base.*` / `long_variants.*` blocks
  (`crates/wem-profiles/src/generated/*_*.rs`);
- kernel: `wem-analysis::dsp::{x87, crt90, psy_geom, psy_geom_long}`.

Checked by `tests/parity/test_geometry_materializer_parity.py`, which
asserts builder == the carrier's words on the 6ch profile, and by the generated
Rust suites (`crates/wem-analysis/tests/*_parity.rs`), which assert kernel ==
builder bit for bit on both geometries. On 2ch the five short surfaces are
registered from that mechanism, with the geometry read from the paired build
running at 48000 Hz; the twelve long surfaces are likewise registered from the
mechanism and read-verified. Any change to either set means regenerating the
carrier (from `corpus/profiles/`) and re-running both suites.
Per-field provenance is in [`profiles.md`](profiles.md).

## Acceptance

A structural change is accepted when every claim in
[`standards.md`](standards.md#what-is-established) still holds. Domain
terminology is defined in [`domain-model.md`](domain-model.md); migration
history lives in the git log.
