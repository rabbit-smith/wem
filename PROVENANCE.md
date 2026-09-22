# Provenance

How this implementation was derived, what it claims, what it does not, and which
of its artifacts are measured from the paired build, generated here, or authored
here. Claims are pointed at rather than restated: the product norms and the test
behind each are in [`docs/reference/standards.md`](docs/reference/standards.md),
and the evidence record for the 2ch/48 kHz result, with its limits, is in
[`docs/findings/2ch-byte-exactness.md`](docs/findings/2ch-byte-exactness.md).

## Purpose

Interoperability. The project produces and reads Wwise Vorbis containers — the
`.wem` files a Wwise build emits and a Wwise runtime loads — so that audio assets
can be authored, inspected, verified and converted without the vendor's
authoring tool in the loop. It is an implementation of a format, not a
replacement for the tool that produced it.

## Method

**Runtime observation of a licensed build.** The implementation was developed by
observing the runtime behaviour of a licensed Wwise 2013.2 build (2013.2.10
build 4884): reading the arrays its stages actually consume and the values its
functions take in and hand back, and reproducing those semantics here. Static
disassembly of that build served to locate where the runtime values live and to
propose candidate algorithms; it is not the source of the semantics, because
every candidate was confirmed again from observed inputs and outputs, from the
produced bitstream, or from published source. No profile value is fitted to the
target output — the rule and its method are in
[`docs/methodology/byte-exact-diagnosis.md`](docs/methodology/byte-exact-diagnosis.md).

**Porting from public, permissively licensed source.** The codec lineage is
Xiph's OggVorbis reference encoder, and Audiokinetic's own Wwise 2011.2.2 release
notes — public, at
<https://www.audiokinetic.com/download/documents/Wwise_v2011.2.2_ReleaseNotes.pdf>
— record that the encoder was updated to aoTuV beta6.03, Aoyumi's fork of it
under the same Xiph BSD-style licence. The analysis here independently confirmed
the 2ch residue path matches aoTuV beta6.03's `_vp_couple_quantize_normalize`,
and the end-of-stream LPC training length follows the public
`vorbis_analysis_wrote` path. The static codebook bodies correspond to tables
vgmstream has published since 2012. Which upstream contributes what, under which
licence, is in [`THIRD_PARTY_NOTICES.md`](THIRD_PARTY_NOTICES.md).

**What is not in the tree.** No Audiokinetic source code, SDK source, header, or
material from a leaked source tree was used or consulted, and none is present in
this repository. The rules that keep it that way for future contributions are in
[`CONTRIBUTING.md`](CONTRIBUTING.md).

## What is claimed

Byte identity with the paired build for the two registered configurations:
Wwise 2013.2 at 6 channels/44.1 kHz and at 2 channels/48 kHz. Not behavioural
similarity — the exact bytes.

What establishes it, all of it in this repository:

- the committed reference containers and their SHA-256 digests:
  `tests/fixtures/reference.wem`, and the `tests/data/2ch-reference/` and
  `tests/data/2ch-stress/` cases with the digests recorded per case in their
  `manifest.json`;
- the whole-file comparison against the reference container (`make wem-bytes`);
- the per-frame and per-stage parity suites, which run both implementations at
  test time and compare every float word, floor post, quantized residue integer
  and packet byte;
- the corpus suite over the representative and stress inputs.

The claim list and the test behind each are in
[`docs/reference/standards.md`](docs/reference/standards.md#what-is-established);
the evidence, its accounting and its limits are in
[`docs/findings/2ch-byte-exactness.md`](docs/findings/2ch-byte-exactness.md).

In the decode direction the claim is different and deliberately weaker:
determinism, exact geometry, bounded memory, and a round trip against this
repository's own encoder — never byte identity with another decoder
([`docs/reference/decoding.md`](docs/reference/decoding.md)).

## What is not claimed

- **No byte identity with any other decoder**, and none with any other Wwise
  version or PCM geometry. The two registered configurations are the two that
  were paired and proven. A selection no installed profile satisfies is refused,
  never approximated by a neighbouring geometry, a default, or a first-match
  pick ([`docs/reference/profiles.md`](docs/reference/profiles.md)).
- **Not an exhaustive proof.** Real-build evidence covers the named inputs.
  Boundary lengths are covered by agreement between this repository's own
  implementations, which is not independent real-build evidence. The limits are
  stated in
  [`docs/findings/2ch-byte-exactness.md`](docs/findings/2ch-byte-exactness.md).
- **No encryption or DRM support of any kind.** This is a deliberate design
  boundary, not an unimplemented feature. The repository contains no key
  handling and no circumvention of any technical protection measure, and the
  decoder refuses any container it does not hold a compiled profile for.
- **No official status.** See
  [Relationship to Audiokinetic](#relationship-to-audiokinetic).

## Derived data

| Artifact | Origin |
|---|---|
| Setup packets, one per installed profile | read from the paired build's runtime; carried as byte constants in `crates/wem-profiles/src/generated/` |
| Psychoacoustic and analysis calibration tables | observed from the paired build: statically hosted arrays byte-verified against it, materialized geometry reproduced by the checked geometry materializer, and the long-geometry surfaces and the 2ch tone bank read from the running 48 kHz build |
| Frozen transcendental tables | generated by `scripts/generate_frozen_tables.py` from the recorded material, compiled into the carrier by `scripts/generate_profile_code.py` |
| Codebook bodies, 598 books | generated Rust tables; their contents correspond to the publicly published vgmstream set ([`THIRD_PARTY_NOTICES.md`](THIRD_PARTY_NOTICES.md)) |
| Reference containers: `tests/fixtures/reference.wem`, `tests/data/2ch-*/` | the paired build's own output for inputs held in this repository; test assets, never read at run time; per-case digests in the `manifest.json` files |
| 2ch reference and stress inputs | generated deterministically by `scripts/generate_2ch_reference_inputs.py` and `scripts/generate_2ch_stress_inputs.py` |
| Implementation: Python reference, Rust kernel, C ABI, language shells | authored here, ported from the permissively licensed public sources named above |
| Recorded development material under `corpus/` | untracked; the recorded material the carrier is generated from, not part of the tracked repository |

The classification of the 2ch/48 kHz resource set, surface by surface, is in
[`docs/reference/profiles.md`](docs/reference/profiles.md#2ch48-khz-provenance).

## Relationship to Audiokinetic

This project is not affiliated with, authorised by, or endorsed by Audiokinetic
Inc. "Wwise" is a trademark of Audiokinetic Inc. and is used in this repository
only to describe the formats this project interoperates with. No Audiokinetic
code is included.