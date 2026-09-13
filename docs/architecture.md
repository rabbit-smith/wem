# Architecture

This document defines the package boundaries for the encoder. The distribution
facade (package root `wwise_wem`) is a stable API shell; the bit-exact
implementation domains live in the development-tree reference package
(`wwise_wem_reference`, under `reference/`), which the test suites use as the
reference oracle. The facade has a single execution path — the in-package
native extension `wwise_wem._core` (built from `crates/wem-python`) — and
never imports the reference tree at runtime. The reference implementation's
byte-for-byte outputs are the project's source of truth; the Rust kernel is
verified against it, never the reverse.

## Package layout

```text
wwise_wem/                      # distribution facade (wheel)
├── __init__.py, __main__.py, api.py, cli.py   # root API shell + CLI
├── model.py                     # root DTOs
├── adapters/                    # external PCM/WAV inputs (facade duty)
├── application/                 # Encoder facade (single path: native core), compat, result DTOs
├── profiles/                    # profile identity, registry, and resource loaders
└── _core.abi3.so                # in-package native extension (built artifact)

wwise_wem_reference/            # development-tree reference oracle (not in wheel, test-time only)
├── python_engine.py             # direct-import oracle encode pipeline (tests only)
├── analysis/                    # MDCT, spectrum, LPC, transient, psychoacoustics, session
├── vorbis/                      # bitstream, setup, floor, residue and packet codecs
├── container/                   # RIFF/WEM models and codecs
├── scheduling/                  # immutable frame plans and mode-selection policy
└── profiles/                    # table loaders, codebook assembly, resource assembly
```

Public DTOs remain importable from the package root. Their implementation may
live next to the domain that owns them; root exports are the compatibility
boundary. Internal module paths are not compatibility surfaces. The reference
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
- `profiles` is the only owner of packaged calibration resources, checksum verification, codebook assembly, and calibrated table loaders.
- The complete immutable `EncoderProfileResources` aggregate is assembled once from manifest logical names at encoder construction. Its nested analysis resources are injected into each analysis session, while its verified setup packet, parsed setup, and codebooks are injected into Vorbis packet encoding. Resource-loader cache keys include their explicit `ResourceRef` identities.
- `application` is the only layer allowed to assemble profiles, analysis, packet encoding, and containers into the complete WAV-to-WEM use case.
- The internal import graph is acyclic.

## Encoding flow

`application.encoder.Encoder` owns one deterministic conversion:

1. Resolve the installed profile bundle for the selected `EncoderProfile` identity.
2. Plan short/long frames and materialize PCM windows.
3. Run transforms, transient detection, and psychoacoustic analysis.
4. Fit floor1 and encode residue into Vorbis packets.
5. Compose the packet stream into a RIFF/WEM container.

Mutable cross-frame state is owned by one analysis session. A `FramePlan` is the sole scheduling truth and every `WindowedFrame` delegates its scheduling fields to that plan.

## Profile ownership

A profile owns its complete calibration set: setup, codebooks, transforms, transient tables, psychoacoustic tables, block geometry, and container defaults. Algorithms receive already validated typed tables. They never select a default file path.

Each installed profile is a self-contained directory:

```text
data/profiles/
├── index.json
└── wwise2013-6ch-44100/
    ├── manifest.json
    ├── vorbis/{setup.bin,codebooks/}
    ├── transform/mdct.json
    ├── analysis/transient.json
    └── psychoacoustics/{short-profiles.json,short-seed.json,long-base.json,long-modes.json}
```

The profile manifest is the single source for identity, geometry, logical resource names, paths, schemas, and SHA-256 values. Payload files contain data and shape metadata, not resource-selection policy.

`data/profiles/index.json` selects the default profile and checksum-addresses its
manifest. The loader resolves both files with package traversables, so the same
profile inventory works from a source tree, an installed wheel, or a ZIP import.
The manifest's `resources` object maps stable logical names (for example
`vorbis.setup` and `transform.mdct`) to paths relative to that profile directory.

`EncoderProfile` is the public identity value (name, key, setup digest, container
defaults). Runtime tables are loaded only through that profile's installed
bundle; there is no separate global runtime-manifest gate.

## Geometry materializer parity

Psychoacoustic surfaces are init-time materialized geometry of the paired
build (2013.2 conversion plug-in, geometry materializer at module offset
0x15ee0). The port lives on three surfaces that must stay byte-identical:

- reference builder: `reference/wwise_wem_reference/geometry_materializer/`
  (per-instruction port; development/test asset, never a runtime path);
- profile data: `src/wwise_wem/data/profiles/*/psychoacoustics/*` (registered
  bytes on the manifest/index digest chain);
- kernel: `wem-analysis::dsp::{x87, crt90, psy_geom, psy_geom_long}`.

Locks: `tests/contract/test_geometry_materializer_contract.py` asserts
builder == registered bytes on the 6ch authority, and the generated Rust
suites (`crates/wem-analysis/tests/*_parity.rs`) assert kernel == builder bit
for bit on both geometries. 2ch registered values are provisional operating
points: they may change only together with real-hardware revalidation and a
regenerated contract in one window. Provenance detail lives in
docs/profiles.md and the corpus adjudication ledger.

## Acceptance boundaries

Structural changes must preserve:

- the exact package-root public exports;
- an acyclic layer-compliant import graph;
- the exact wheel inventory;
- the 205-frame regression contract;
- the golden WEM byte stream and SHA-256;
- installed-wheel and ZIP-import resource verification.

Domain terminology is defined in [`domain-model.md`](domain-model.md); migration history lives in the git log.
