# Profile data as Rust code

Date: 2026-09-22. This page is the evidence record for moving the encoder
profile data out of the tracked JSON tree and into the kernel as generated Rust
tables — what moved, what proved the move exact, where the recorded material
lives now, and how the pure-Python reference reads the same values.

## Decision

Profile data is Rust code in the kernel. The compiled artifact is the single
carrier: another language that needs the values reads them from the artifact,
and nothing ships a resource tree.

Two consequences follow and are the whole of the change:

* the JSON tree left the tracked repository and became untracked development
  material under `corpus/profiles/`;
* the controlled carrier in git became generated Rust source
  (`crates/wem-profiles/src/generated/`), produced by
  `scripts/generate_profile_code.py` from that material — the same status as
  `crates/wem-analysis/tests/x87_parity.rs`, a tracked generated artifact whose
  generator consumes untracked `corpus/` material.

## Result

| Claim | Evidence |
|---|---|
| The generated tables equal the recorded documents, table by table, bit by bit | `crates/wem-profiles/src/carrier_tests.rs` (`every_compiled_profile_matches_the_recorded_tree`, `assembled_resources_are_identical_from_both_sources`) |
| The assembled codec inputs are identical from either source | same file, `assembled_resources_are_identical_from_both_sources` |
| The kernel's dump equals the recorded documents | `tests/parity/test_profile_artifact_bridge.py` |
| Whole-file output is unchanged | `make wem-bytes`, `cargo test -p wem-core --test complete_wem_bytes` |
| Every stage and frame is unchanged | `frame_pipeline_parity`, `stage_parity`, `vorbis_oracle_values`, `tests/parity/` |
| Regeneration is byte-stable | `python3 scripts/generate_profile_code.py` twice, empty diff; `--check` exits 0 |

## The one thing that was not a rename

The kernel read profile documents with `serde_json` **without** its
`float_roundtrip` feature, and that reader does not parse decimal to the nearest
double: it converts the digit run to `f64` and then divides once by a power of
ten. For a 17-significant-digit value such as `1.1164131164550781` that lands one
unit in the last place away from a correctly rounded parse.

The recorded bytes were produced through that arithmetic, so the carrier has to
reproduce it. The first version of the generator used Python's correctly rounded
`float()` and the equivalence suite caught the difference immediately
(`short_profiles[*].mask_curves`); the generator now replicates the reader's step
in `serde_f64` and routes every document number through it
(`json.loads(..., parse_float=serde_f64)`).

This is worth keeping: it is exactly the class of silent value movement the
migration had to be proved free of, and it was found by comparing the carrier
against the documents rather than by comparing the carrier against itself.

## Where the material lives now

`corpus/profiles/` — the untracked development-material tree at the repository
root, next to the paired-build measurements. It is the same tree, byte for byte,
that the package shipped under `src/wwise_wem/data/profiles/`; its content digest
(`f5f97ba673b4b045a1b741b94bbeb8801cf3b357c086221de9bea764b137672f`, over the
sorted `path\0sha256` pairs) is recorded in the header of every generated module.

It was taken from revision 9c73bd4 on `main`. `scripts/generate_profile_code.py
--help` names the location, and `--profiles-dir` overrides it; the generator is
offline and resolves its paths from `Path(__file__)`.

## How the reference oracle reads the values

The oracle needs the profile values to reproduce bytes, and under this decision
it takes them from the compiled artifact rather than from a copy of its own.

* **What the kernel exposes.** `wem_profiles::blob::profile_tables_blob()`
  renders every compiled profile into one canonical little-endian, versioned,
  self-describing stream: identity fields, container geometry, the setup packet
  bytes, and every typed table as named blocks of stored words. Floats travel as
  their IEEE bit patterns; integer tables keep the width the codec reads them at;
  nothing is re-derived on the way out.
* **How the oracle reaches it.** The binding exposes it as
  `wwise_wem._core.profile_tables()` (a data accessor, not a second execution
  path); `wwise_wem_reference.profiles.artifact.decode` reads the stream into
  flat, named bit-pattern tables. `tests/parity/test_profile_artifact_bridge.py`
  holds the result against the recorded documents.
* **Why the specification is not inverted.** The reference implementation owns
  the *algorithm and ordering*: which bank feeds which transform, how the tone
  table nests, how a quality value walks the breakpoint axis, how the transient
  record family materializes, how the recordings are packed. None of that is in
  the dump and none of it moved. The profile values are external facts measured
  from the paired build — the documents were equally just facts, and what made
  the reference the specification was always the code that consumed them, not
  where the numbers came from. The reference is still what the kernel is checked
  against, and reading facts from the artifact is the same relationship the
  oracle had with the recorded documents.

### The precise cost

One half of the oracle's old role does end, and it is worth stating rather than
papering over: the reference's profile *loaders* used to be the thing that
rejected a malformed profile document, and there are no documents to reject at
run time any more. Shape validation of the recorded material now happens once, at
generation time, inside `scripts/generate_profile_code.py`, and its result is
pinned by the equivalence suites. The oracle keeps the half that matters for
byte-exactness — what the numbers mean and in what order they are consumed — and
loses the half that was really a property of the transport.

There is no case where the oracle genuinely cannot work this way. The one place
it *would* break is a dump format that loses information (a width truncation, a
float rendered as text); the format avoids that by carrying stored words
verbatim, and the bridge suite checks the decoded values against the documents
rather than against the kernel's own opinion of them.

### Follow-up not taken here

A C ABI entry point for the same stream (`wem_profile_tables`) is the natural
counterpart so that non-Python shells read the carrier the same way. It is not in
this change: `include/wem.h` is outside this lane's file set, and the Python
binding is the one consumer that exists today.

## Trust boundary

The generated tables are checked in, so a reviewer reads Rust, not JSON. What
that buys and what it costs:

* **Bought** — the values compile into every artifact, so a shipped encoder
  cannot be separated from its profile data; there is no path, no digest chain,
  no index, and no name-keyed lookup left to get wrong at run time.
* **Cost** — the recorded material is no longer visible to `git`. Reproducing a
  generated table from scratch needs `corpus/profiles/`, which is untracked
  development material; the generator says so in `--help`, and the content digest
  in each generated header says which revision of the material produced it.
## What has landed, and what the deletion step still owns

Landed and verified: the carrier, the generator, the kernel reading its profile
data from the carrier, the oracle-access mechanism (dump + binding accessor +
decoder), and the equivalence proofs. The recorded tree is still tracked and the
serialized intake still exists, because the second half of the decision —
deleting it — is a cross-language change with a large fallout surface, and a
half-deleted tree is worse than an explicit hand-off.

The deletion step, with its exact surface, measured from the tree at that point:

**Rust (`crates/wem-profiles`)**

| Remove | Why it exists today |
|---|---|
| `build.rs` | walks `src/wwise_wem/data/profiles` and emits `embedded_profiles.rs` |
| `src/embedded.rs` | the compile-time `include_bytes!` profile bundle |
| `src/bundle.rs` | index/manifest intake, `RuntimeResourceManifest`, `verify_all`, `INDEX_SCHEMA` / `BUNDLE_SCHEMA`, `load_profile_bundle{,_from_bytes,_from_static_bytes}` |
| `src/resources.rs` | `ResourceRef`, `ResourceBackend`, path normalizers, digest checks |
| `src/data.rs` | `DataDir`, the filesystem seam |
| `src/loader_tests.rs`, `src/bytes_loader_tests.rs` | suites for the intake above |
| `src/book_ids.rs`'s `BookTable::load` | JSON row decoding (`from_generated` stays) |
| `src/transform.rs` | base64 MDCT decoding (`make_mdct_look` stays in `wem-analysis`) |
| `src/frozen.rs`, `src/quality.rs`'s loader, `src/transient.rs`'s loaders, `src/psychoacoustics/*`'s loaders | document decoding; the typed value objects and the materialization kernels stay |
| `src/source.rs`'s `impl ProfileSource for ProfileBundle` | the development seam; the trait itself can collapse once one implementation remains |
| `src/model.rs`'s `EncoderProfile` name field | the profile label becomes derived from the key |
| `crates/wem-profiles/tests/{profiles_integration,record_family,two_channel_profile,profile_selection}.rs` | the loader suites |

**Python**

51 modules import the facade profile package. The deletions are
`src/wwise_wem/profiles/{bundle,model,registry,resources}.py`; the rest is
rewiring, in four groups:

* the reference oracle's intake — `reference/wwise_wem_reference/python_engine.py`,
  `profiles/assembly.py`, and the eight `profiles/*` loaders that take a
  `ResourceRef`, which move onto `profiles/artifact.py`;
* `reference/wwise_wem_reference/container/model.py`, which imports
  `EncoderProfile` and the generation label;
* tooling — `scripts/{decode_cdlc_wem,fuzz_diff_parity,generate_frozen_tables,native_smoke,wheel_smoke}.py`;
* test support and suites — `tests/{analysis_resource_support,codebook_resource_support,two_channel_corpus_support}.py`,
  `tests/parity/{oracle_frame_values,stage_records_support}.py`, and about
  twenty test modules, of which `tests/unit/profiles/*` (nine modules) test the
  loader itself and are the ones that must be rewritten against the artifact
  rather than merely repointed.

**Packaging** — `src/wwise_wem/data/profiles/**` (26 files, 1.6 MB) leaves the
tracked tree for `corpus/profiles/`; `tests/parity/distribution_allowlist.json`
loses its resource entries, `pyproject.toml` its package-data glob, and
`scripts/wheel_smoke.py` its digest-chain check.
