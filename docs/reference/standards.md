# Standards

The product norms for the encoder: what the system must be, and the test that
establishes each one. How to work in this repository — git and commit
discipline, lanes and the shared checkout, the verification ladder, what to read
first — is in the layered [`AGENTS.md`](../../AGENTS.md) set, because an agent
reads those automatically. Task instructions are in [`../guides/`](../guides/),
evidence for a completed result is in [`../findings/`](../findings/), and the
terms to use are in [`domain-model.md`](domain-model.md).

## Bit-exactness

Correctness is defined by exact bytes, not by behaviour similarity: the encoder's
output must equal the paired build's output byte for byte. The pure-Python
reference under `reference/` is the specification the kernel is ported from,
statement by statement — each `_f32(...)` in Python is one `f32` operation
boundary in Rust — and the kernel is checked against the oracle's output, never
the reverse.

A comparison runs two implementations against each other (the oracle against the
kernel, the C ABI against the kernel, the built wheel against its allowlist), or
a run against an external reference (the paired build's WEM for the fixture and
for each corpus case). A recorded expectation is not a comparison: when it
disagrees with a run it hides the change behind a re-record step.

When a comparison reports a difference, the difference says what moved: either
the code is wrong, or the expected bytes have changed as part of the work. Those
are two different changes, and a case is never satisfied by re-recording it.

Every stage lands with its comparison against the oracle's live per-frame
values (`crates/wem-core/tests/frame_pipeline_parity.rs` for the crate surfaces,
`tests/parity/test_frame_pipeline_parity.py` through the shipped binding): both
implementations run at test time over all 205 frames of the fixture, and every
float32 word of the eight analysis stages, every floor post, every quantized
residue integer and every packet byte is compared value against value. The diff
names the first mismatching frame index, stage, channel and bin and prints both
words; a stage is not done until it comes back zero.

## What is established

| Claim | Established by |
|---|---|
| The reference WEM for `tests/fixtures/input.wav`, byte for byte — SHA-256 `17851d26c6210b85e498ae0452d2562d7b9e2c3e9e795c459656b9c9d8d35247` | `make wem-bytes` (`tests/whole_file/test_whole_file.py`); `crates/wem-core/tests/complete_wem_bytes.rs` |
| Per-frame values: scheduling fields, eight analysis stages, floor posts, residue rows, packet bytes, all 205 frames | `crates/wem-core/tests/frame_pipeline_parity.rs`, `tests/parity/test_frame_pipeline_parity.py` |
| The package-root public exports and the wheel inventory | `tests/parity/test_public_api.py`, `tests/parity/test_distribution.py`, `make wheel-smoke`; the export list is in [`public-interface.md`](public-interface.md) |
| Geometry-materializer parity: the ported builder == the carrier's registered words == the kernel's `psy_geom*` surfaces | `tests/parity/test_geometry_materializer_parity.py`; `cargo test -p wem-analysis` |
| The compiled profile carrier: the kernel's tables equal the recorded material, table by table | `crates/wem-profiles/src/carrier_tests.rs` (stage 1, retired with the recorded tree), `tests/parity/test_geometry_materializer_parity.py`, `cargo test -p wem-profiles` |
| The 2ch/48 kHz result, its corpora, and the limits of that evidence | `tests/parity/test_2ch_reference_corpus.py`, `tests/parity/test_2ch_stress_corpus.py`, [`../findings/2ch-byte-exactness.md`](../findings/2ch-byte-exactness.md) |

Running those suites is what establishes each claim. The local targets that run
them are listed in
[`../guides/development.md`](../guides/development.md#what-each-target-runs).

## Determinism

- No runtime transcendental (`sin`, `cos`, `log`, `log10`, `pow`, `exp`) in an
  encoder path — in Python (`math.sin/cos/log/log10/pow/exp` outside the
  site recorder in `reference/wwise_wem_reference/_tmath.py`) or in Rust
  (`f32::sin/cos/ln/log/exp/powf`). Every such value comes from profile data
  (`FrozenMathTables`, the MDCT trig bank, the static trig banks); a ported
  function that would need one is a design error, not a TODO. A new geometry
  that needs a new transcendental input is derived by
  `scripts/generate_frozen_tables.py` and ships as a checksummed profile
  resource; encoder code may only table-read it.
  `tests/unit/profiles/test_frozen_tables.py` checks the frozen domain against a
  fresh derivation and that an encode through the installed profile fires zero
  live calls at the four enumerable sites.
- Float semantics: values are float32 at every assignment point. The Python
  `_f32` call sites mark those points, and Rust rounds at the same statements —
  an intermediate Python keeps as float64-wrapped-float32 is not promoted or
  fused. Summation order and the MDCT butterfly/bit-reverse order are fixed: the
  per-frame comparisons read the resulting values, so reordering them changes
  what the comparison reports. SIMD may vectorize element-wise operations only,
  and `fma` and `fast`-math intrinsics are prohibited because they change
  rounding.
- Cross-frame mutable state lives in exactly one analysis session (domain model:
  *Analysis session*); everything else stays immutable.
- Generated artifacts (the frozen-table payloads, the generated profile code,
  the 2ch corpus inputs) come out byte-identical when regenerated: deterministic
  ordering (`sort_keys`, sorted file walks), explicit little-endian packing, no
  timestamps, absolute paths or other ambient values in an artifact, and every
  generator script safe to run twice with an empty diff.

## Bit patterns

Bit patterns travel as integers or little-endian bytes, never as decimal strings.
A float crosses a boundary through `to_bits`/`from_bits` or its little-endian
bytes; a digest or a word is carried as a value or as bytes, not as text to be
re-parsed. This includes code that carries profile data.

The live comparison follows the same rule: the oracle's value stream carries
every row as 8-hex-digit bit-pattern words
(`tests/parity/oracle_frame_values.py`), so a consumer compares value against
value and can name the first differing channel and bin — never text floats,
never `repr`. A file that has to carry raw words names the endianness in its
suffix.

## Layers and dependency direction

The import graph is one-directional and acyclic, and the same split holds for
Python (oracle) and Rust (kernel). `scheduling` imports nothing downstream;
`analysis`, `vorbis` and `container` never open package resources; `profiles` is
the sole owner of the compiled calibration carrier and its readers; one
application layer alone assembles the WAV-to-WEM use case. [`architecture.md`](architecture.md) holds the complete rule list;
`tests/parity/test_runtime_boundary.py` checks the graph for the oracle tree —
acyclicity, and that `scheduling`, `analysis` and the DSP modules import no
downstream domain.

Rust crates follow the same direction, with the same ownership:

```text
wem-profiles   → (types of wem-vorbis/wem-analysis/wem-container; sole resource owner)
wem-scheduling → (none below it)
wem-analysis   → wem-scheduling (+ own dsp)
wem-vorbis     → (pure codec primitives; never profiles/container/application)
wem-container  → (pure container codecs)
wem-core       → all of the above (sole orchestration layer)
wem-capi       → wem-core + wem-profiles (C ABI surface; no numerics)
wem-python     → wem-core (PyO3 binding; no numerics)
wem-wasm       → wem-core with `default-features = false` (wasm binding; no numerics)
```

A crate may not `use` a sibling outside that direction; the edges are held by
the crate manifests and by review, not by a runtime check.

`analysis` receives typed configuration and never selects a file path or a
default. The reference package may import facade DTOs and profile-metadata types
by absolute name (`wwise_wem.model`, `wwise_wem.profiles.key`); everything else
in it resolves by in-package relative imports, and the facade never imports the
reference tree.

## Errors

**No implicit handling.** The library surfaces a failure; it does not absorb it.
Nothing is swallowed (`unwrap_or`, `.ok()`, `let _ =`, `except: pass`), no
default hides a caller mistake, and nothing is clipped or truncated in silence.
A failure the caller cannot see is a defect.

The same rule decides the facade's execution path: byte-producing code calls the
in-package native extension `wwise_wem._core` unconditionally — no engine
probing, no availability check, no fallback, no switch, no environment variable.
A missing extension surfaces as the ordinary `ImportError` the import machinery
raises, and production code never imports the reference oracle. There is no
runtime switch that selects an implementation; parity between the oracle and the
kernel is asserted by the tests. The extension's module name is owned by the
packaging surface (`tests/parity/distribution_allowlist.json`,
`pyproject.toml`, the `wem-python` library name, locked by the distribution
tests) and is not restated in facade code.

**Errors carry enough.** A public error says what failed and where. An error that
wraps another implements `Error::source()` — or, in Python, `raise … from …` —
so the cause is reachable from the value the caller holds, and the message
carries the observed values rather than a category name. A panic means an
invariant broke, not that the caller passed something bad.

`crates/wem-core/tests/error_source_chain.rs` walks `EncoderError` →
`InternalError` → the stage error (`ProfileError`, `AnalysisError`,
`PacketError`, `ContainerError`) through `source()` and pins that the innermost
cause is reachable. At the Python boundary,
`tests/parity/test_error_surface.py` pins that `WwiseWemError.code` is the
kernel's own class, that `str(error)` is the kernel's diagnostic unchanged, and
that the kernel error stays reachable as `__cause__`.

**Error types are exhaustively matchable.** A public error enum carries no
`#[non_exhaustive]`, on purpose: adding a variant is a breaking change, and the
compiler must tell every caller that a new failure mode exists.
`crates/wem-core/tests/error_variants.rs` matches every public error enum with
no `_` arm, from a caller's position, so the rule is compiler-enforced: a new
variant fails that build until the caller's arm is written. Adding an error
*code* on the C ABI, or a code string in Python, is not a breaking change —
those surfaces have no exhaustive matching, which is why
[`include/wem.h`](../../include/wem.h) can promise that `WemError` values are
stable and append-only, never renumbered or reused, and why the Python code
names are the same stable classes.

## Panics

A panic is a defect. Input-derived paths are panic-free: no `unwrap()`,
`expect()` or `panic!()` on a path a caller can reach with input, in either
implementation, and a caller's bad input is reported as one of the error classes
above rather than by unwinding.

Where a shell can catch a panic it reports an internal-defect code —
`WEM_ERR_INTERNAL` at the C ABI, `INTERNAL` in the Python classes. Every kernel
call behind the C ABI is wrapped in `catch_unwind` for exactly that reason; the
wasm shell builds with `panic = "abort"`, where an invariant violation
terminates the instance instead of unwinding. A handle that carries state across
calls is terminal after a failure: `push` and `finish` on a finished or failed
session are rejected, never silently resumed, while a one-shot call may be
retried (`crates/wem-capi/tests/capi_e2e.rs` covers the finished-session
rejection and two encodes through one shared handle).

## Caller streams and ambient state

The library does not write to the caller's streams and does not read ambient
state behind the caller's back. Output leaves through the caller's callback — the
C ABI write and packet callbacks — or as a returned value; the library prints
nothing of its own. No environment variable, working directory, clock or locale
decides what gets encoded: a configuration is named by a structured profile
selection, never by a variable or a path, and the native runtime carries its
profile data compiled into the library rather than locating any at run time.
The caller-facing consequences are in [`public-interface.md`](public-interface.md)
(execution path) and [`profiles.md`](profiles.md) (access boundary).

## Profile data ownership

A profile owns its complete calibration set: the Wwise Vorbis setup packet and
codebooks, channel mask and mapping, short/long block geometry, psychoacoustic,
transient-selector and floor/residue tables, and default container metadata.
Algorithms receive already validated typed tables; they never select a default
file path.

A selection names one installed configuration — a Wwise generation plus the PCM
geometry — and resolves to exactly one profile, or is rejected: never replaced
by a default, never resolved by geometry alone, never a first-match pick when
more than one installed profile satisfies it. The selection rules, the
provenance classification of the 2ch/48 kHz resource set, the crate-private
resource intake and the frozen transcendental tables are in
[`profiles.md`](profiles.md).

A profile's values are Rust constants in the same artifact as the setup packet
they belong to, so there is nothing to locate, address or verify at run time:
the identity the key carries is the digest of exactly the packet the carrier
holds, recomputed from those bytes when the value model is built from the
carrier (`crates/wem-profiles/src/model.rs`), so no stored digest copy can drift
from them; the carrier itself is proved value-for-value against the recorded
material. A construction path that requests a value outside the frozen domain
fails loudly instead of consulting the host libm. The carrier layout is in
[`architecture.md`](architecture.md#profile-ownership), and provenance — no
profile value is fitted to an output — is in [`profiles.md`](profiles.md).

## Integration topology

The Rust kernel is the sole integration point. Its cross-language surface is the
C ABI declared in [`include/wem.h`](../../include/wem.h) and implemented 1:1 by
`crates/wem-capi`: the lifecycle (`Init` → `push*` → `Finish`), the reply framing
(seq 0 carries the setup packet, then the audio packets), memory ownership, and
the error codes. Every language binding is a parallel shell over the kernel —
PyO3 (`wwise_wem._core`), wasm for the browser and Node (`crates/wem-wasm`,
`js/`), Go via cgo (`examples/go-cgo`), C — never a parallel implementation.
Shells mirror that interface 1:1, map errors 1:1 without inventing variants, and
own no numerics and no profile logic; a new language integrates by writing a
shim over the C ABI, never by changing the kernel for it, and a shell that needs
something the others do not gets a new stable C ABI entry point rather than a
kernel fork. `crates/wem-capi/tests/capi_e2e.rs` runs the C surface against the
kernel — reference bytes through the FFI, error-code mapping, lifecycle
violations, shareable handles.

A same-process consumer binds the kernel directly: never through RPC or a
serialization hop, and never through a second integration surface kept in
parallel with the C ABI.

Streaming chunk boundaries must not affect the output bytes: any chunking of the
input yields the same container.
`crates/wem-core/tests/stream_session.rs` and
`tests/parity/test_2ch_stress_corpus.py` compare the batch, one-chunk and uneven
chunking paths, and `make fuzz-parity` (`scripts/fuzz_diff_parity.py`) runs the
oracle against the native kernel over a fixed seed set of random PCM streams and
random frame-aligned chunk splits.

The reference package under `reference/` is a development-tree oracle: not part
of the wheel, not listed in `tests/parity/distribution_allowlist.json`, imported
by the test suites, and never imported by runtime code.

## Portability floor

The kernel stays compilable for `wasm32-unknown-unknown` in a scalar
configuration. `wem-core`'s `parallel` feature — per-channel rayon partitioning
inside `wem-analysis` — is default-on for native builds and can be dropped by a
threadless consumer, which is how `crates/wem-wasm` builds it; the scalar path
is the one the parity comparisons read.

Profile bytes are consumable without filesystem I/O: the tables are compiled
into the library and resolved by selection
(`compiled_profile_for_selection(WwiseProfile) -> Result<CompiledProfile, ProfileError>`),
so no data directory, profile name or manifest bytes exist to appear on the
runtime surface.

## Source rules

- Names come from [`domain-model.md`](domain-model.md) (`FramePlan`,
  `AnalysisSession`, `AudioPacket`, `WemContainer`, `FrozenMathTables`); no
  parallel vocabularies.
- Data transfer objects are immutable — frozen dataclasses and
  `MappingProxyType` in Python.
- `unsafe` needs a comment proving that bit-exactness or FFI requires it;
  "not needed" is the default until proven otherwise.
- Hot loops allocate nothing per sample and return no `Vec` from an inner stage
  where a scratch buffer exists; scratch ownership stays at the session
  boundary.
