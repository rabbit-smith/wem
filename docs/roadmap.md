# Roadmap and outstanding work

The two registered encoder profiles are byte-exact for their paired reference
inputs. The final 2ch evidence and its current limits are recorded in
`findings/2ch-active-findings.md`.

## Confirmed state

| Area | Current state | Evidence |
| --- | --- | --- |
| 6ch/44.1 kHz | Whole WEM and every registered frame/stage are byte-exact | `make golden`, `make frame-contract`, `make stage-contract` |
| 2ch/48 kHz | Paired 96,000-frame input is byte-exact: 37,658 B, 142/142 audio packets, SHA-256 `41fe43e…ef629` | `2ch-active-findings.md` |
| 2ch representative corpus | Six real-build cases are whole-file byte-exact: silence, opposed DC, low tone, high tones, isolated impulses, and independent stereo noise | `tests/data/2ch-reference/manifest.json` |
| 2ch stress corpus | Alternating channel bursts and a tail-changing input are whole-file byte-exact through the native batch, native streaming, and Python oracle paths | `make 2ch-stress` |
| Native/oracle parity | Batch, one-chunk streaming and randomized chunking are byte-identical | `make fuzz-parity`, pinned 2ch integration contract |
| Input boundaries | Profile-selected conditioner and Xiph-compatible dynamic EOS LPC training are implemented in batch and streaming paths | conditioner tests, streaming parity, paired tail observation |
| Residue path | aoTuV coupled quantization, type-2 classification, classwords and stage packing reproduce the paired build | `2ch-residue-xiph.md` |

## Maintenance work

1. Keep both paired byte contracts, the representative and stress corpora, and
   the profile digest chain unchanged.
2. Extend the independent real-build evidence with varied-duration, long-form
   music while retaining deterministic synthetic edge and fuzz coverage.
3. Run `make fuzz-parity` with ordinary test changes that touch streaming,
   scheduling, profile assembly, or packet packing.
4. Preserve the clean-room evidence rule: profile data comes from direct reads
   or published reference algorithms, never output fitting.
5. Add a new geometry only after it works end to end and has its own pinned
   whole-file byte contract.

## Acceptance ladder

1. targeted unit or crate test;
2. `make fuzz-parity` and the affected contract;
3. `make frame-contract`, `make stage-contract`, and `make golden`;
4. `cargo test --workspace`;
5. `make wheel-smoke`.
