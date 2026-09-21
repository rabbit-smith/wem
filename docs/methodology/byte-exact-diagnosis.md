# Diagnosing a byte-exactness gap

The playbook for the class of problem this project keeps meeting: the target is
an external binary, correctness is *exact bytes*, and the two implementations
agree on behaviour but not on bits. It is extracted from the 2ch/48 kHz
diagnosis, whose result is recorded in
[`../findings/2ch-byte-exactness.md`](../findings/2ch-byte-exactness.md).

## Why this is a distinct kind of debugging

The acceptance criterion is a byte string, not an observable behaviour, so every
similarity metric — correlation, PSNR, dB error, file size — is a *proxy*, and a
proxy computed at low resolution is worse than no measurement, because it
manufactures false exclusions. A pipeline with quantizers in it has a threshold
in the middle: a 0.03 dB error in a curve, or one index step in a fit, can flip a
symbol and change thousands of bits.

So the objective is narrower than "make the outputs closer": **find the first
integer or bit where the two implementations diverge, and explain it from an
independent observable.**

## The loop

1. **Establish a deterministic red light.** Pin the input digest, output size,
   output digest, and packet count. Re-run it before and after every change; the
   residuals are your only progress signal. If two failures exist, separate them
   first — a streaming-tail regression will otherwise be charged to the codec
   gap.
2. **Instrument before hypothesising.** Instruments are code and they have bugs.
   Validate a new instrument against a case whose answer you already know, and
   confirm it is measuring what you think (entry point, argument order, units,
   loop bounds).
3. **Close the accounting in the target's own unit.** Bits, not bytes. Attribute
   the excess band by band and make the remainder close exactly. A ledger that
   closes turns "the file is 2.6 % bigger" into "this integer handoff differs in
   these partitions".
4. **Admit two oracles only.** (a) The target's *runtime state*: the actual
   arrays it feeds its own stages. (b) The *published reference implementation*
   that the target embeds. Disassembly, correlation, and file size may propose
   candidates; they may never conclude. Anticipate that the target embeds a
   *different* reference than the one you assumed.
5. **Substitute one array at a time.** Move a single input between the target and
   your implementation and observe which side the divergence follows.
6. **Retract loudly.** Keep a revision ledger inside the finding. Delete
   statistics produced by an instrument later found broken, and mark the
   conclusions that rested on them as withdrawn — an un-retracted wrong number
   re-enters the next session's priority list.
7. **Never fit.** A value read from the target or the published source is
   evidence. A value adjusted until the bytes match is a landmine: it will hold
   for the sample in front of you and fail on the next one. If a rule needs a
   constant you cannot read, that missing observation *is* the next task.

## Instruments that worked

- **Runtime observation of the target's own internals.** Locate the handoff by
  the module's own control flow (the function that rewrites the buffer in place
  is a strong anchor), then read the arrays at that point. One observation of the
  real inputs collapsed a week of inference.
- **The project's own decoder as the symmetric reader.** Any statistic computed
  from a bitstream must come from one reader used on both sides. Hand-written
  readers desynchronise on variable-length codes and produce spectacular,
  meaningless ratios.
- **Per-frame comparisons from the current tree.** The live per-frame and
  per-stage suites (`cargo test -p wem-core --test frame_pipeline_parity`,
  `tests/contract/test_frame_pipeline_parity.py`, `make stage-contract`) exist
  to tell you *where* the first difference is instead of *that* there is one.
- **Boundary inputs.** Deliberate samples — silence, opposed DC, low and high
  tones, isolated impulses, independent-channel noise, alternating-channel
  bursts, a tail change — reach branches a single music sample never touches. One
  such sample exposed a state-role swap that had survived every other test.

## Instrument hazards

Each of these produced a wrong conclusion at least once.

| Hazard | Symptom | Defence |
| --- | --- | --- |
| Probe does float math in the host runtime | The target's own numerics change when the probe is attached | Copy raw bytes; interpret them offline |
| Attach after the target starts work | Your "first frame" is not the first frame; nothing pairs | Suspend at process creation, attach, then resume |
| Interception site shorter than the patch | The script loads and silently never fires | Verify the interception fires before trusting an empty result |
| Hand-rolled bitstream reader | Impossible ratios, unstable counts | Use the project decoder, or validate against it |
| Derived files from an older tree | A phantom "only N values differ" that re-measurement contradicts | Re-derive from the current working tree before prioritising |
| Probe slows the target down | Timeouts, truncated output, half-written records | Shrink the observation window; treat truncated records as absent |

## Substitution algebra

| Observation | Conclusion |
| --- | --- |
| Target input + your implementation reproduces the target output | Your implementation of that stage is correct — look upstream |
| Your input + target implementation reproduces the target output | Your *inputs* to that stage are wrong, not the stage |
| Target input + your implementation still differs | You are misreading the handoff: argument order, units, or which buffer is in place |
| Both sides real, output matches coefficient for coefficient | This stage is closed; the remaining gap lives strictly elsewhere |

## Two traps that cost the most time

**Ablation is order-dependent.** Substituting a known-correct table into a
pipeline that is *still wrong elsewhere* measures that table's marginal
contribution at the current state, not its true contribution. A psychoacoustic
table whose substitution moved 5 bytes in the broken pipeline moved 20 of 142
packets once the residue path was correct. **Never close a mechanism on an
ablation run before the pipeline is known-good.**

**An exclusion needs finer resolution than the effect.** "The curves agree within
0.17 dB" cannot exclude a mechanism whose effect is one index step across a ±0.5
quantisation threshold. "The port is structurally identical to the published
source" cannot exclude a difference in the *input* to that port. When you record
an exclusion, record the instrument's resolution and the threshold next to it, so
the next reader can see whether it was ever entitled to conclude.

## Ordering the work

Bisect along the data flow, not along the file layout:

```
input conditioning → analysis inputs → analysis outputs → quantisation
                   → classification → bitstream packing → container
```

Fix the cheapest *upstream* gap first, because it unblocks the measurement of
everything downstream — but re-measure after every fix. Each closed gap promotes
the next-largest residual to the top of the list, and the residual's *shape*
(which packets, which bands, which frames) is the actual evidence. Watch for
residuals that are not spread proportionally but confined to a specific
population — end-of-stream, one transient variant, one channel, one block size —
because that shape names the mechanism.

## The evidence record

A finding document is the durable artefact, and it has a fixed shape:

1. **Result** — size, digest, packet counts, and the exact input identity.
2. **Root causes** — one per gap, each with the observation that established it
   and the packet-count movement it produced.
3. **Accounting** — the bit ledger that closes against the target.
4. **Retractions** — what was withdrawn, and why: broken instrument, invalid
   resolution, or stale data.
5. **Trust boundary** — what the corpus proves, and explicitly what it does not.
6. **Pointers** — mechanisms belong in the reference docs; the finding links to
   them rather than restating them.

## Checklist before declaring a gap closed

- [ ] Whole-file digest, size, and packet counts identical on at least one paired
      real input.
- [ ] The same result through every implementation path (native, direct core,
      oracle) and under randomized chunking.
- [ ] A boundary corpus covering the branches the fix touches, not just the
      original sample.
- [ ] Every parity and golden suite still passes.
- [ ] Every value introduced by the fix traceable to a runtime read or a
      published reference — no fitted constants.
- [ ] Retractions and the trust boundary written down in the finding.