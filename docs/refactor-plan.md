# Domain restructuring plan

This checklist moves the verified encoder into the package boundaries defined by `architecture.md`. Moves are mechanical unless a step explicitly describes a resource contract change. Internal compatibility wrappers are not retained; the package-root API remains stable.

## Phase 0 — establish the boundary

- [x] Define the target domain packages and dependency direction.
- [x] Record the domain model under `docs/`.
- [x] Provide a cross-platform local artifact cleanup command.
- [x] Lock package-root exports, wheel contents, import DAG, frame contract, and golden output.
- [x] Replace the exact internal edge baseline with package-layer rules as domain packages land.

## Phase 1 — move implementation into domains

Each batch must update imports, tests, wheel inventory, and documentation in the same commit. Old internal module paths are deleted.

### 1.1 Application and adapters

- [x] `encoder.py` → `application/encoder.py`
- [x] `encode_wem.py` → `application/compat.py`; remove its duplicate CLI entry point
- [x] `wav_adapter.py` → `adapters/wav.py`
- [x] Move application-owned result/stat models next to the application; re-export public names at package root

### 1.2 Scheduling

- [x] `frame_scheduler.py` → `scheduling/planner.py` and `scheduling/model.py`
- [x] Move immutable frame planning into `scheduling/`; PCM preprocessing belongs to analysis
- [x] `mode_selector.py` → `scheduling/selector.py`
- [x] Add package-layer tests that keep scheduling independent from analysis and encoding

### 1.3 Analysis

- [x] `frame_model.py` → `analysis/model.py`
- [x] `stream_state.py` → `analysis/session.py`
- [x] `transform.py`, `log_spectrum.py`, `lpc.py` → `analysis/dsp/`
- [x] `transient_detector.py` → `analysis/transient/detector.py`
- [x] Move PCM windowing and detector feeding into `analysis/preprocessing/`
- [x] Move psychoacoustic modules into `analysis/psychoacoustics/`
- [x] Move resource loading out of analysis modules and leave typed configuration inputs

### 1.4 Vorbis

- [x] `floor1.py` and `floor1_fit.py` → `vorbis/floor.py` and `vorbis/floor_fit.py`
- [x] `residue.py` → `vorbis/residue.py`
- [x] Consolidate `encode_packet.py` and `audio_packet.py` under `vorbis/packet_encoder.py`
- [x] `parse_audio_packet.py` → `vorbis/packet_decoder.py`
- [x] Move codebook resource assembly behind the profile resource interface

### 1.5 Profiles

- [x] `wem_profiles.py` → `profiles/registry.py` and `profiles/model.py`
- [x] `profile_bundle.py` → `profiles/bundle.py`
- [x] `resources.py`, codebook/table loaders → `profiles/resources.py` and domain-specific assemblers
- [x] Ensure caches include profile/resource identity in their keys

## Phase 2 — organize tests by responsibility

- [x] Move pure behavior tests to `tests/unit/<domain>/`.
- [x] Move encoder/profile/CLI flow tests to `tests/integration/`.
- [x] Move API, architecture, distribution, and frame contracts to `tests/contract/`.
- [x] Move full WEM identity to `tests/golden/`.
- [x] Keep PCM/WEM fixtures under `tests/fixtures/` and frame data under `tests/data/frame-contract/`.
- [x] Replace the manually enumerated fast-test command with directory-based unittest discovery.

## Phase 3 — create self-contained profile bundles

- [x] Add `data/profiles/index.json`.
- [x] Add one profile `manifest.json` containing identity, geometry, container defaults, and the complete logical resource inventory.
- [x] Move setup and codebooks under the profile directory.
- [x] Move transform and transient calibration under the profile directory.
- [x] Move short and long psychoacoustic calibration under the profile directory.
- [x] Make manifest resource SHA-256 values the only resource digest declarations.

## Phase 4 — inject profile resources

- [x] Assemble the immutable analysis portion of `EncoderProfileResources` at encoder construction.
- [x] Pass typed transform, transient, and psychoacoustic tables into analysis owners.
- [x] Pass setup/codebooks into Vorbis owners.
- [x] Remove loader default paths and module-level resource selection.
- [x] Remove the global runtime manifest and duplicate bundle fields.
  - [x] Drop `EncoderProfile.runtime_manifest_path` / `runtime_manifest_sha256`; `runtime_manifest()` reads the installed profile bundle.
  - [x] Stop gating `Encoder` on `WWISE2013_RUNTIME_MANIFEST_SHA256`; validate against the selected profile's installed bundle instead.
  - [x] Collapse `ProfileBundle.setup` into the manifest `vorbis.setup` resource (no duplicate field).
  - [x] Remove legacy aliases such as `DEFAULT_BUNDLE` and unused path→digest duplication where callers can use logical resources.

## Phase 5 — remove transitional structure

- [x] Delete `data/tables/`, the old global manifest, and the old bundle file.
- [x] Remove any empty leftover `data/tables/` directories from the source tree.
- [x] Remove hard-coded resource paths and SHA values from Python modules.
- [x] Remove the exact internal edge allowlist after layer rules cover the final graph.
- [x] Confirm no old internal module or resource path enters the wheel (`make wheel-smoke` + distribution allowlist).
- [x] Rewrite architecture/public docs for the final layout only (no "target/migration" wording; lazy-import notes use current module paths).
- [x] Ignore local analysis artifacts (`.understand-anything/`) so they never enter VCS or wheels.
- [x] Run the complete acceptance suite (`make check`) and leave `git status` clean after `make clean`.

## Acceptance for every batch

Final gate for Phase 5 (also used after each earlier batch):

- [x] Focused unit and boundary tests pass.
- [x] Fast suite passes.
- [x] Internal import graph is acyclic and layer-compliant.
- [x] 205-frame contract passes.
- [x] Golden WEM bytes and SHA-256 are unchanged.
- [x] Wheel inventory and installed/ZIP smoke tests pass.
- [x] `git status` is clean after `make clean`.
