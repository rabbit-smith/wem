# Architecture

This document defines the package boundaries for the encoder. The package root is a stable API shell; implementation code belongs to a named domain.

## Package layout

```text
wwise2013_wem/
├── __init__.py, __main__.py, api.py, cli.py
├── application/       # use-case orchestration and compatibility adapters
├── adapters/          # external PCM/WAV inputs
├── scheduling/        # immutable frame plans and mode-selection policy
├── analysis/
│   ├── dsp/            # MDCT, spectrum and LPC primitives
│   ├── preprocessing/  # LPC-padded detector input and window materialization
│   ├── transient/      # transient detector algorithm
│   └── psychoacoustics/# remap, seed, envelope and temporal analysis
├── vorbis/             # bitstream, setup, floor, residue and packet codecs
├── container/          # RIFF/WEM models and codecs
└── profiles/           # profile identity, registry, resources, codebooks, and calibrated tables
```

Public DTOs remain importable from the package root. Their implementation may live next to the domain that owns them; root exports are the compatibility boundary. Internal module paths are not compatibility surfaces.

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

## Acceptance boundaries

Structural changes must preserve:

- the exact package-root public exports;
- an acyclic layer-compliant import graph;
- the exact wheel inventory;
- the 205-frame regression contract;
- the golden WEM byte stream and SHA-256;
- installed-wheel and ZIP-import resource verification.

Migration history is tracked in [`refactor-plan.md`](refactor-plan.md). Domain terminology is defined in [`domain-model.md`](domain-model.md).
