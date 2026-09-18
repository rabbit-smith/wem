# Roadmap and outstanding work

This file tracks what is confirmed, what is still approximated, and what each
remaining item needs. It is a plan, not a contract; the contracts live in
`tests/`. Two rules govern everything below:

1. A profile byte is either **read** from the paired build or **computed** by a
   per-instruction port of it. Resampling, fitting and interpolation are not
   admissible sources, so an item stays open until a read or a port exists.
2. Every gate listed in the root `AGENTS.md` must keep passing unchanged.

## Confirmed state

| Area | State | Locked by |
| --- | --- | --- |
| 6ch/44.1k, all surfaces | bit-exact against the paired build | `make golden`, `make frame-contract`, `make stage-contract` |
| `container nAvgBytesPerSec` | derived at pack time (`floor(data_bytes * rate / frames)`), no longer a registered constant | `tests/contract/test_pcm_edge_contract.py` |
| 2ch/48k SHORT surfaces: `ath`, `octave`, `look.interval_table`, `look.mask_curve` | mechanism-registered (promoted from operating points) | `tests/contract/test_geometry_materializer_contract.py` |
| 2ch/48k SHORT profile rows, both profiles (`profiles[0]`, `profiles[1]`) | mechanism-registered, one knot bank each | same contract (`test_short_profile_curves_are_mechanism_derived`) |
| 2ch/48k `geometry.first_octave` | `-34`, read from the running build | invariant test in the same contract |
| 2ch/48k LONG surfaces: `analysis.curves[0..2]`, `analysis.field_19_curve`, `analysis.interval_u32`, `seed.base_curve`, `seed.group_labels_u32`, both `long-modes` variants | mechanism-registered and read-verified against the running build | same contract (`test_long_surfaces_2ch`) |
| 2ch/48k LONG geometry words: `seed.outer_u32[7] = -226`, `[10] = 776` | read from the running build (was the carried 6ch `-234`/`777`) | invariant assertion in the promotion; `docs/roadmap.md` below |
| 2ch/48k LONG `analysis.profile_u32`, `seed.profile_u32`, `seed.tone_banks` | structurally identical to 6ch (verified) — **not** a gap | — |
| 2ch/48k `seed_outer_u32` rate-dependent scalars | **corrected** from the live read, both record copies (inert for the the round signal) | `tests/contract/test_paired_2ch_scalars_contract.py` |
| 2ch/48k frame plan (transient detector, mode selector) | **matches the reference exactly** — same mode sequence and packet count | `tests/unit/analysis/test_mode_selection_tail.py`, crate test `mode_selection_tail` |
| 2ch/48k aux fmt fields `0x34`/`0x38`/`0x3C` | **pinned** from the read; every self-describing fmt field matches the reference on non-silent input | `tests/contract/test_paired_2ch_scalars_contract.py` |
| `uMaxPacketSize` derivation | **fixed**: it is the largest *audio* packet, the setup packet is excluded | `tests/contract/test_packet_framing_contract.py`, crate test in `packets.rs` |

The remaining LONG profile fields above have no mechanism path yet; everything
else in the 2ch LONG surface set now comes from the port.

## The authoritative 2ch geometry (read, not inferred)

| Table | `first_octave` | `shift_octave` | `eighth_octave_lines` | `total_octave_lines` | `sample_rate` |
| --- | ---: | ---: | ---: | ---: | ---: |
| SHORT, 44.1k | -42 | 5 | 8 | 585 | 44100 |
| SHORT, 48k | **-34** | 5 | 8 | 585 | 48000 |
| LONG, 44.1k | -234 | 5 | 8 | 777 | 44100 |
| LONG, 48k | **-226** | 5 | 8 | 776 | 48000 |

The 44.1k SHORT and LONG rows are the registered 6ch values; the 48k rows were
read from the running build and are registered for the 2ch profile.

`first_octave` shifts with the sample rate by `64 * log2(rate / 44100)` (about
+8), while `total_octave_lines` stays rate-invariant. That is what keeps the
seed floor walk's invariant

```
end_max = octave_max - first_octave = 583 < total_octave_lines = 585
```

identical at both rates, with the same margin of 2. Carrying `first_octave`
over from 6ch is what previously pushed the walk past the grid.

### How to repeat the read

`WwiseCLI.exe` is a small launcher; the process that actually loads the
conversion plug-in and materializes the geometry is a process named `Wwise` (a
few tens of MB, in the order of a hundred modules). The read is:

1. Start a generation run (`WwiseCLI.exe <project> -GenerateSoundBanks`) with a
   source at the geometry of interest.
2. Enumerate processes and keep the one whose module list contains the
   conversion plug-in.
3. Walk its committed readable memory and search for the dword pattern
   `(first_octave, shift_octave, eighth_octave_lines, total_octave_lines, sample_rate)`.

Records appear repeatedly at a fixed stride, which is what distinguishes a real
geometry record from a coincidental match.

## Outstanding work

### O-1: `short_look_1` (`profiles[1].mask_curves`)

Done. The two profiles read two knot banks of the same descriptor family:
`profiles[0]` from `mask_pool_0` and `profiles[1]` from `mask_pool_1`, both at
the default quality index and a zero bias. Each reproduces the registered 6ch
authority 384/384 bytes, and the two banks are byte-equal in the build's two
descriptor copies, so the sample rate enters only through the curve lerp. Both
sets are registered for 2ch/48k (174 of 384 `profiles[1]` values moved, by up to
0.478 dB) and locked by the contract.

Note for later: the surface *is* reached (the encoder selects `profiles[1]` on
short transition frames, and perturbing its rows changes output bytes), but the
corrected rows do not bind on the current 2ch/48k test signal, so this fix does
not by itself move the paired-encode divergence.

### O-2: 2ch/48k LONG surfaces

Done. Two facts closed it, both read from the live build rather than argued:

* **The surfaces.** A scan of the running 2ch/48k process found every predicted
  surface, eight copies each, and found the previously registered
  (frequency-resampled) values nowhere. Five *full* 4096-byte surfaces were then
  read back and compared: `analysis.curves[0]`, `analysis.curves[1]`,
  `analysis.field_19_curve`, `analysis.interval_u32` and `seed.base_curve` all
  matched this port's 48000 output 4096/4096 bytes. The mode-3 variant rows and
  `seed.group_labels_u32` were confirmed by 64-byte slice (8 hits each).
* **The geometry words.** The registered 2ch seed record still carried the 6ch
  geometry (`first_octave` -234, `total_octave_lines` 777). The scan hit
  `(-226, 5, 8, 776, 48000)` three times and the carried variant zero times, so
  `outer_u32[7]`/`[10]` are now `-226`/`776`.

Why the geometry mattered: with `octave(1024, 48000)` peaking at 549, the stale
`first_octave` gave `end_max = 549 + 234 = 783`, past the 777-entry grid, and the
seed floor walk panicked on the first 2ch encode after the surface promotion.
The read values give `549 + 226 = 775 < 776`, the same margin of 1 the 6ch LONG
family has. The mechanism's inputs are shared (the LONG knot banks sit at the
same pointer-block indices in both descriptor copies), so the rate is the only
parameter that differs.

### O-5: LONG fields with no builder

`analysis.profile_u32`, `seed.profile_u32` and `seed.tone_banks` are still
carried from the 6ch registration (and were frequency-resampled for 2ch) and no
builder covers them. They are now the only 2ch LONG values that are not
authoritative, so they are the first place to look for the remaining paired
encode divergence.

### O-3: seed floor bound

Done: the floor walk now reports an out-of-range cursor in both the kernel and
the reference instead of indexing past the seed surface
(`crates/wem-analysis/src/psychoacoustics/seed.rs`,
`reference/wwise_wem_reference/analysis/psychoacoustics/seed.py`), with
regressions in `tests/unit/analysis/test_seed_floor_bounds.py`.

### O-4: documentation

Done: `README.md`, `docs/profiles.md` and `docs/architecture.md` describe the
promoted 2ch SHORT surfaces — including both profile row sets — as
mechanism-materialized, and attribute only the 2ch LONG tables to operating
points.

## The paired-encode divergence, localised

Reading the reference stream's own framing (the Wwise audio header writes only
the mode value, so the mode is bit 0 of the payload) gives a precise split:

* The **frame plan is correct**. The reference 2ch/48k stream is 48 short, then
  87 long, then 7 short frames — exactly our plan's prefix, with an identical
  215-byte setup packet. The transient detector and mode selector agree
  frame-for-frame.
* **We emit 7 extra trailing short frames** (149 vs 142). Their hop positions
  are 96000..96896: the reference's last frame starts exactly at the source
  length, ours runs on. This is *not* fixable by a constant stop rule — see
  O-6.
* **The coded content diverges from packet 1** while the mode matches
  (106 vs 102 bytes), so something feeding the analysis is still not
  authoritative — see O-7.
* **Five aux fmt fields are zero for 2ch** while the reference carries
  `0x34=16080`, `0x38=16560`, `0x3C=0xec69cb18` (the 6ch profile already pins
  18180 / 18636 / 0xb3dea448). See O-8.

### O-6: tail frame count (closed)

Closed. No constant bound fits the references, but a self-referential one does:
the build emits a frame while the *previous* frame's center still lies inside
the PCM, so the plan ends one hop past the source length -- and that hop is the
frame's own (1024 for a long tail, 128 for a short tail).

Evidence: thirteen 2ch/48k reference streams read back from the host -- four
from the bank route, one from `-ConvertExternalSources`, different sources and
lengths -- all end with the last frame's center equal to `dwTotalPCMFrames`
(one at +64). The old fixed bound `center < source_len + 1024` matched only the
6ch golden, and emitted seven extra trailing short frames on the the round target.

With the corrected rule the same input now yields the reference's plan exactly:
48 short, 87 long, 7 short, 143 packets, identical in the oracle and the kernel,
while the 6ch golden stays byte-identical. The synthetic PCM boundary contract
was re-locked in the same change; its plans now end exactly on the source length
(4096 -> 4096, 8192 -> 8192) and the new stream is a packet prefix of the old
one, so only trailing frames moved. Regressions on both sides:
`tests/unit/analysis/test_mode_selection_tail.py` and
`crates/wem-analysis/tests/mode_selection_tail.rs`. If the routes differ, the
converter needs an explicit route/trim rule, which is a contract decision.

### O-7: content divergence from packet 1

With the plan verified, the remaining payload difference is upstream of the
packet stage. The 2ch inputs that are still not read-verified, in the order
they feed a short frame, are: the materialised `analysis.transient` record, the
`analysis.frozen-tables` twiddles/coordinate domain, and the O-5 LONG fields.
All three are reachable with the live-read method that closed O-1/O-2.

### O-8: aux fmt fields

Partly closed, and the earlier reading was too strong.  `0x34`, `0x38` and
`0x3C` are per-layout constants: thirteen 2ch/48k reference streams (different
sources, lengths, both routes) carry the identical triple.  But
`dwUnknown_0x24` / `uUnknown_0x32` are **encoder-derived, not constant** -- see
O-11.  The other values are layout constants rather than
per-sample data: all thirteen 2ch/48k reference streams read back from the host
-- different sources, lengths, and both conversion routes -- carry the identical
`0x34 = 16080`, `0x38 = 16560`, `0x3C = 0xec69cb18` (with `0x24 = 0`,
`0x32 = 0`), while the 6ch golden carries its own triple
(18180 / 18636 / 0xb3dea448) that the 6ch profile already pins. They are not a
CRC or adler of the PCM, WAV or payload, not packet-start offsets and not
payload byte offsets; the values are exact multiples of the block align (4020
and 4140 frames for 2ch, 1515 and 1553 for 6ch), so they read as a frame-range
record.  Pinned in the container metadata; on every non-silent reference the 2ch
output's fmt then matches field for field.  The remaining header fields are all
content-derived (`dwDataPayloadSize`, `nAvgBytesPerSec`, `uMaxPacketSize`) plus
the O-11 pair.

### O-10: kernel hardcoded the short-path sample rate (fixed)

`crates/wem-analysis/src/psychoacoustics/pipeline.rs` passed the literal 44.1k
sample rate to `update_frame_spectrum_peak` on the short-frame path while the
reference port reads `resources.short_look.sample_rate` (the long path already
used its table's
rate). That silently corrupts the cross-frame spectrum peak for any non-44100
input, and it is a kernel/oracle divergence the parity suites cannot see because
they run the 6ch/44.1k fixtures, where the literal happens to be right. Fixed to
read the profile rate; `cargo test --workspace`, `make golden` (still
byte-identical) and the full gate suite pass. Measured: the the round output is
unchanged, so this was not the O-7 cause -- but it is a contract violation for
every other input length/rate.

### Verified since: the analysis tables are not the O-7 cause

* `analysis.frozen-tables`: `fft_twiddles` is byte-identical across rates,
  `window_halves` likewise, and `coordinate_ln` matches
  `(f32((i+0.5)*rate/256), ln(that))` **128/128 at both 44100 and 48000** -- the
  2ch table is recomputed for 48k, not resampled.
* `analysis.profile_u32` / `seed.profile_u32`: byte-identical to the
  authoritative 6ch registration (0/256 words differ).
* `seed_outer_u32` rate-dependent scalars, the second geometry copy, and the
  short-path rate literal: all wrong and now either fixed or recorded, all
  measured inert for this signal.

Every input the long psy path consumes is therefore either read-verified or
positively verified, and three real defects were fixed while checking. The
remaining difference (12.8% of long-frame floor posts, in integer steps 1..16)
must come from the analysis *mechanism* at 2ch/48k rather than from a table.
Next probe: dump this port's internal `post` curve for one long frame and compare
it at the 29 floor-post positions against the reference's decoded posts -- if
they agree within the quantization step, the divergence is in the fit/windowing
rather than in the psy floor input.

### O-11: `dwUnknown_0x24` / `uUnknown_0x32` are derived, not constant

An all-silent probe found these two non-zero where every music-like reference has
zero, and they are the **same number**: `dwUnknown_0x24 == uUnknown_0x32 << 16`.
A length sweep of silent input gives

| input frames | `uUnknown_0x32` |
| --- | --- |
| 48000 | 704 |
| 96000 | 832 |
| 192000 | 64 |

so the value is encoder-derived (all three are multiples of the 64-sample
detector quantum) and not a function of length alone.  The registered profile
pins 0/0, which matches the the round target and all thirteen lab references, so this
does not block the round -- but a silent or near-silent input would differ.  The rule
is not yet known; the sweep above is the starting evidence.

### O-12: silence reproduction (closed the loop)

Feeding digital silence through the paired build and through this encoder gives
**byte-identical coded audio** (97 packets, setup included), which shows the psy
floor/envelope path is right when there is no signal content, and localises the
the round divergence to signal-dependent stages.  The probe harness generates the
source on the host (SHA-verified identical to the local file) and converts it
through the same external-source project, so cases can be constructed on demand.

## Fix queue

Ordered, so a later session can resume here.

1. **O-7 -- the coded-audio divergence.** Localised to the residue at the minimum
   quantum, caused by a floor that differs on 12.8% of long-frame posts.  Next
   probe: dump this port's internal `post` curve for one long frame and compare it
   at the 29 floor-post positions against the reference's decoded posts; if they
   agree within the quantization step, move to the windowing/scheduler and the
   cross-frame state carry (`carried_global_specmax`, seed state).
2. **Kernel/oracle parity for non-44.1k inputs.** O-10 fixed the one hardcoded
   rate, but the generated parity suites still only exercise the 6ch/44.1k
   fixtures, so the same class of divergence can hide.  Add a 2ch/48k parity case.
3. **`psy_geom` / `psy_geom_long` model one profile only.** They cover
   `mask_pool_0` and the 6ch LONG geometry, so kernel-side materialisation of the
   second profile and of the 2ch LONG tables is unmodelled.  The encode path reads
   the profile data, so this is parity-suite surface rather than encoder
   behaviour.
4. **O-11 -- the derived `unknown_0x24`/`unknown_0x32` pair.**  Non-zero only for
   silent-like input; needs more probes to find the rule.
5. **Pre-existing repo-wide analyzer backlog.** A fresh full scan reports ~89
   findings outside the files this work touched (mostly `rust-unwrap` in
   `crates/`, reserved identifiers in `include/wem.h`).  None were introduced here
   (verified: no `unwrap()` was added, and the one in `seed.rs` is a pre-existing
   test).  Deserves its own change.

## Tooling note

`.pi-lens.json` disables two mutation paths (`format.enabled`,
`actionableWarnings.autoFix.enabled`) and two rules
(`unchecked-throwing-call-python`, `reportMissingImports`). The mutations are off
because the autoformatter rewrote unrelated lines in a file this work only
needed a small edit in. `unchecked-throwing-call-python` is off because it fires
on every intentional fail-loud coercion in the port (`int()`/`float()` on
already-validated inputs), where raising is the specified behaviour.
`reportMissingImports` is off because both of the plugin's Python analyzers
cannot see this repo's declared import roots: they live in
`pyproject.toml [tool.mypy] mypy_path = "src:reference"`, and Pyright reads
`extraPaths` only from `pyrightconfig.json` or LSP settings.  A project
`pyrightconfig.json` declaring those roots is committed (so editors and the LSP
path resolve them correctly), but the `pyright` runner ignores it -- verified by
editing a file and watching the same pre-existing import lines reappear -- so the
rule is additionally filtered project-wide.  Nothing is suppressed in the source:
the imports are unchanged, and `make lint` (mypy over `src reference`, ruff over
`src reference tests scripts`) remains the authoritative gate.
The repository's own gates -- ruff and mypy, run by `make lint` -- are unaffected
and remain authoritative.

### O-7: content divergence from packet 1

Localised, not yet caused. Reading both streams back through the pack decoder
splits the payload difference into two clean facts:

* The **frame plan matches** (mode sequence and packet count are identical),
  and the *decoded spectra* agree in energy, sum-of-squares and peak.
* The difference is confined to the **residue at the minimum quantum**: every
  coefficient that is non-zero on our side and zero on the reference side has
  magnitude 1 (74/74, 99/107 and 72/72 in three sampled long packets), with a
  handful of the reverse direction at magnitude 1-2. Those extra bins sit in the
  mid band (contiguous-run percentiles ~365-730 of 1024) and never above bin
  768.
* Cause: the encoded **floor** differs. Over all 87 long frames, 645 of 5046
  decoded floor posts differ (12.8%), by integer steps of 1..16. A slightly
  different floor moves coefficients across the +/-0.5 quantization boundary,
  which is exactly the signature above. Long frames carry 13.2% more payload
  (short frames 5.1%).

Every input the long mode-2 psy path consumes is either read-verified
(`analysis_curves[1]`, `field_19_curve`, `interval_u32`, `seed.base_curve`,
`group_labels_u32`) or structurally identical to 6ch (`profile_u32[84:124]`,
whose LUT is `[0,1,2,3,4,5,6,7,7,7,7,6,...]`). So the residual gap is in the
long psy mechanism itself at 2ch/48k, not in a missing table. Next probe: dump
this port's internal `post`/`raw` curve for one long frame and compare its shape
against the reference's decoded floor, to name the stage.

### O-9: rate-dependent `seed_outer_u32` scalars (closed)

The live read of the 2ch/48k seed record (anchored on the long geometry
5-tuple) shows the serialised `outer_u32` holds **two** records, and several
rate-dependent scalar fields in both copies still carry the 6ch values:

| index | registered | read from the build | role |
| --- | --- | --- | --- |
| `[16]`, `[46]` | f32 `1.0` | f32 `1.205` | look group head |
| `[17]`, `[47]` | 720 | 664 | look group |
| `[20]`, `[50]` | 696 | 640 | look group |
| `[23]`, `[53]` | 608 | 560 | peak-suppression active span |
| `[37]`, `[40]`, `[41]` | -234 / 777 / 44100 | -226 / 776 / 48000 | **second** geometry copy |

The remaining differing words are load-time-relocated pointers and must stay as
registered. Only the first geometry copy was corrected when O-2 closed.

Closed: both copies corrected from the live read and locked by
`tests/contract/test_paired_2ch_scalars_contract.py`.  Measured: correcting them
leaves the 2ch encode byte-identical, so they are not the O-7 cause (the active
span in particular is inert for this signal).  Only the scalar words were
written; the pointer-bearing ones differ by load-time relocation and stay as
registered.

## Measuring progress

The end-to-end measurement is the paired encode: run the real build's
conversion over a deterministic PCM file and compare against this encoder's
output byte-for-byte. Current status for a 2ch/48k source: ours 42390 bytes over
149 packets, the reference 37658 bytes over 143 packets, diverging from packet 1
onward.

The O-1 promotion did not move that boundary (the corrected floor does not bind
on this signal). The O-2 promotion *widened* the size gap (36987 -> 42390) while
replacing every one of those surfaces with values read byte-exact out of the
running build, which rules the surfaces out as the cause and localises the
remaining divergence in the LONG consumption path and the O-5 fields.

## 逐帧阶段地图（O-7）

逐帧侧例程与地址证据归档在语料库台账
`corpus/paired-build/the round/l3-perframe-map.md`；`docs/` 内只保留结论：

* 已登记的心理声学表在**初始化期**被几何物化器消费；实测逐帧路径**不读**这些表，
  因此"以登记表或其邻域为锚"无法定位逐帧阶段（三次尝试均落在初始化路径上）。
* O-7 的分歧（信号活跃频带上地板偏高 0.2–0.9 dB）必须落在**逐帧检测核**与
  **长表面构造器**上；两者的起始位置与调用关系已记入上述台账。
* 下一步：离线逐指令比对这两个例程与移植侧对应实现（无需远端主机）。
