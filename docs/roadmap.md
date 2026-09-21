# Roadmap and outstanding work

Both registered profiles are byte-exact for their paired reference inputs. The
2ch evidence, its corpora and its limits are recorded in
[`findings/2ch-byte-exactness.md`](findings/2ch-byte-exactness.md).

## Confirmed state

| Area | Current state | Evidence |
| --- | --- | --- |
| 6ch/44.1 kHz | Whole WEM and every registered frame/stage are byte-exact | `make golden`, `make frame-contract`, `make stage-contract` |
| 2ch/48 kHz | Paired 96,000-frame input is byte-exact: 37,658 B, 142/142 audio packets, SHA-256 `41fe43e…ef629` | [`findings/2ch-byte-exactness.md`](findings/2ch-byte-exactness.md) |
| 2ch representative corpus | Six real-build cases are whole-file byte-exact: silence, opposed DC, low tone, high tones, isolated impulses, independent stereo noise | `tests/data/2ch-reference/manifest.json` |
| 2ch stress corpus | Alternating channel bursts and a tail-changing input are whole-file byte-exact through the native batch, native streaming, and Python oracle paths | `make 2ch-stress` |
| Native/oracle parity | Batch, one-chunk streaming and randomized chunking are byte-identical | `make fuzz-parity`, the pinned 2ch integration contract |
| Input boundaries | Profile-selected conditioner and Xiph-compatible dynamic EOS LPC training are implemented in both batch and streaming paths | conditioner tests, streaming parity, paired tail observation |
| Residue path | aoTuV coupled quantization, type-2 classification, classwords and stage packing reproduce the paired build | [`findings/2ch-byte-exactness.md`](findings/2ch-byte-exactness.md) |

## Maintenance work

1. Keep both paired byte contracts, the representative and stress corpora, and
   the profile digest chain unchanged.
2. Extend the independent real-build evidence with varied-duration, long-form
   material while retaining deterministic synthetic edge and differential
   coverage.
3. Run `make fuzz-parity` with ordinary changes that touch streaming,
   scheduling, profile assembly, or packet packing.
4. Preserve the clean-room evidence rule: profile data comes from direct reads
   or published reference algorithms, never from fitting an output.
5. Add a new geometry only after it works end to end and has its own pinned
   whole-file byte contract.

## Acceptance ladder

The command-by-command ladder is in
[`guides/development.md`](guides/development.md#verification-ladder). The order
is fixed: targeted test → differential parity and the affected contract →
frame/stage/golden contracts → the Rust workspace → the installed-wheel smoke.