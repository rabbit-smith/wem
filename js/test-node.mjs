#!/usr/bin/env node
/**
 * wwise-wem-wasm — Node parity test.
 *
 * Proves the wasm shell encodes byte-exactly against the kernel reference
 * bytes, through the package's own entry point (src/index.ts, run via
 * Node's native type stripping on >= 22.18), and that its profile selection
 * is the structured one of `include/wem.h` (ABI revision 3):
 *
 *   1. ONE-SHOT: the pinned 6ch/44.1kHz recording -> WEM, byte-identical to
 *      the committed kernel reference (tests/fixtures/reference.wem) —
 *      auto-selected from the WAV geometry, with an explicit selection, and
 *      through the raw-PCM entry.
 *   2. STREAMING CHUNK CONSISTENCY: the same PCM fed to sessions with three
 *      different chunkings (single chunk, fixed-size chunks, irregular
 *      frame-aligned chunks) must each reproduce the reference — chunk
 *      boundaries never change the output bytes (include/wem.h).
 *   3. SELECTION BEHAVIOUR: the compiled-in generation table, the resolved
 *      selection of every constructor, and the selection error mapping
 *      (unknown version code -> FORMAT_UNSUPPORTED, non-positive geometry ->
 *      STATE_ERROR, unsatisfiable selection -> PROFILE_NOT_FOUND) are
 *      pinned; the profile bundle is compiled in, so nothing is fetched.
 *   4. ERROR MAPPING: kernel failures surface as JS Errors whose `code` is
 *      the stable WEM_ERR_* string.
 *   5. DECODING: the wrapper's decode surface (createDecoder / decodeWem, the
 *      wasm `WemDecoder` of include/wem.h section 5) decodes the committed
 *      reference WEM at run time — the declared geometry and frame count, a
 *      reconstruction of the WAV the container was produced from, chunk
 *      invariance, one header announcement, and the refusal/lifecycle codes.
 *
 * The decode half needs no oracle from outside this checkout: the WEM and the
 * WAV it was produced from are both committed, and the comparison against the
 * source is live (a relative bound), never a recorded number.
 *
 * Run: make wasm-build   (both packages: js/pkg, js/pkg-node)
 *      node js/test-node.mjs   — needs a built js/pkg-node.
 * The package is wasm-pack output and is not part of the checkout; with no
 * package built there is nothing to compare, so this test fails and names the
 * build command rather than skipping.
 */

import { existsSync, readFileSync } from "node:fs";
import { dirname, join } from "node:path";
import { fileURLToPath } from "node:url";
import {
  initWasm,
  listVersions,
  parseWav,
  encodeWav,
  createEncoder,
  createStreamSession,
  createDecoder,
  decodeWem,
  WEM_ERROR_CODES,
} from "./src/index.ts";

const here = dirname(fileURLToPath(import.meta.url));
const repoRoot = join(here, "..");

// Pinned kernel reference: tests/fixtures/input.wav (6ch/44.1kHz, 139398 frames)
// -> tests/fixtures/reference.wem. The profile bundle rides inside the wasm
// module. The comparison is the committed file's bytes; the SHA-256 of that
// file is documented in js/README.md and is deliberately not restated here.
const RECORDING = join(repoRoot, "tests/fixtures/input.wav");
const REFERENCE = join(repoRoot, "tests/fixtures/reference.wem");
const RECORDING_FRAMES = 139398;

// wasm-pack writes js/pkg-node (and js/pkg) -- neither is in the checkout, so
// the test builds nothing silently and skips nothing: it stops here and prints
// the command that produces the package.
const PACKAGE_FILES = ["wem_wasm.js", "wem_wasm_bg.wasm"].map((name) =>
  join(here, "pkg-node", name),
);
const missingPackageFiles = PACKAGE_FILES.filter((path) => !existsSync(path));
if (missingPackageFiles.length > 0) {
  console.error(
    [
      "FAIL: the Node wasm package (js/pkg-node) is not built, so the byte",
      "comparison against tests/fixtures/reference.wem cannot run.",
      ...missingPackageFiles.map((path) => `  missing: ${path}`),
      "",
      "Build it first:",
      "  make wasm-build               # both packages (js/pkg, js/pkg-node)",
      "  cd js && npm run build:node   # the Node package only",
    ].join("\n"),
  );
  process.exit(1);
}

let failures = 0;

function check(label, ok, detail = "") {
  const mark = ok ? "PASS" : "FAIL";
  console.log(`[${mark}] ${label}${detail ? ` — ${detail}` : ""}`);
  if (!ok) failures += 1;
}

const equal = (x, y) =>
  x.byteLength === y.byteLength && [...x].every((v, i) => v === y[i]);

async function expectWemError(label, fn, expectedCode) {
  try {
    await fn();
    check(label, false, "expected a throw, call succeeded");
  } catch (error) {
    const code = error && typeof error.code === "string" ? error.code : "<no code>";
    check(
      label,
      code === expectedCode,
      `code=${code}, message="${error?.message ?? error}"`,
    );
  }
}

const sameSelection = (got, want) =>
  got.versionCode === want.versionCode &&
  got.version === want.version &&
  got.generation === want.generation &&
  got.channels === want.channels &&
  got.sampleRate === want.sampleRate &&
  got.description === want.description;

const describe = (s) =>
  `versionCode=${s.versionCode}, version=${s.version}, generation=${s.generation}, ` +
  `channels=${s.channels}, sampleRate=${s.sampleRate}, description=${s.description}`;

const wavBytes = new Uint8Array(readFileSync(RECORDING));
const referenceWem = new Uint8Array(readFileSync(REFERENCE));

await initWasm();

// --- 0) the compiled-in generation table -----------------------------------
const versions = await listVersions();
check(
  "compiled-in Wwise version table (WemVersion)",
  Array.isArray(versions) &&
    versions.length === 1 &&
    versions[0].code === 0 &&
    versions[0].label === "2013" &&
    versions[0].generation === "2013.2",
  JSON.stringify(versions),
);

const SIX_CHANNEL = {
  versionCode: 0,
  version: "2013",
  generation: "2013.2",
  channels: 6,
  sampleRate: 44100,
  description: "6ch/44100Hz/2013",
};

// --- 1) one-shot parity ----------------------------------------------------
const t0 = Date.now();
const auto = await encodeWav(wavBytes); // no selection: auto-selected from the WAV
check(
  "one-shot (auto-selected) bytes == reference.wem",
  equal(auto.data, referenceWem),
  `bytes=${auto.data.byteLength}, ${Date.now() - t0}ms`,
);

const explicit = await encodeWav(wavBytes, { version: 0, channels: 6, sampleRate: 44100 });
check(
  "one-shot (explicit selection 0/6ch/44100) bytes == reference.wem",
  equal(explicit.data, referenceWem),
  `bytes=${explicit.data.byteLength}`,
);

const parsed = await parseWav(wavBytes);
check(
  "kernel WAV parse: geometry and frame count",
  parsed.channels === 6 &&
    parsed.sampleRate === 44100 &&
    parsed.frames === RECORDING_FRAMES &&
    parsed.pcm.byteLength === RECORDING_FRAMES * 12,
  `channels=${parsed.channels}, sampleRate=${parsed.sampleRate}, frames=${parsed.frames}`,
);

// spelling variants of the version selector, and the resolved selection each
// constructor entry reports (no encode: construction + selection only)
const encoder = await createEncoder({
  version: "2013",
  channels: 6,
  sampleRate: 44100,
});
check(
  "encoder selection (label \"2013\") resolves to 6ch/44100Hz/2013",
  sameSelection(encoder.selection(), SIX_CHANNEL),
  describe(encoder.selection()),
);
const pcmOneShot = encoder.encodePcm16Interleaved(parsed.pcm);
check(
  "one-shot through the raw-PCM entry bytes == reference.wem",
  equal(pcmOneShot.data, referenceWem),
  `bytes=${pcmOneShot.data.byteLength}`,
);
encoder.destroy();

for (const [label, source] of [
  ["generation \"2013.2\"", { version: "2013.2", channels: 6, sampleRate: 44100 }],
  ["geometry only (auto generation)", { channels: 6, sampleRate: 44100 }],
  ["auto-selected from the parsed WAV", parsed],
]) {
  const handle = await createEncoder(source);
  check(
    `encoder selection (${label}) resolves to 6ch/44100Hz/2013`,
    sameSelection(handle.selection(), SIX_CHANNEL),
    describe(handle.selection()),
  );
  handle.destroy();
}

// --- 2) streaming chunk consistency ----------------------------------------
const framesPer = parsed.pcm.byteLength / 12; // 6ch x s16 = 12 bytes/frame

async function streamEncode(chunks, source) {
  const session = await createStreamSession(source);
  try {
    const resolved = session.selection();
    for (const chunk of chunks) {
      session.push(chunk);
    }
    // Streaming observability: the count reflects exactly the frames pushed
    // so far, and crosses the boundary as a JS number (the shell returns it
    // as f64, so no count a session can reach is truncated).
    const frames = session.pcmFrames();
    return { result: session.finish(), resolved, frames };
  } finally {
    session.destroy();
  }
}

const slice = (from, to) => parsed.pcm.slice(from * 12, to * 12);

const plans = {
  "single-chunk": [parsed.pcm],
  "fixed-5462-frames": (() => {
    const out = [];
    for (let off = 0; off < framesPer; off += 5462) {
      out.push(slice(off, Math.min(off + 5462, framesPer)));
    }
    return out;
  })(),
  "irregular-frame-aligned": (() => {
    const sizes = [1, 80000, 3, 4096, 648]; // frames per chunk
    const out = [];
    let f = 0;
    while (f < framesPer) {
      const end = Math.min(f + sizes[f % sizes.length], framesPer);
      out.push(slice(f, end));
      f = end;
    }
    return out;
  })(),
};

const results = [];
for (const [label, chunks] of Object.entries(plans)) {
  // first plan: explicit geometry, generation auto-selected; the rest: the
  // parsed WAV as the geometry source
  const source = label === "single-chunk" ? { channels: 6, sampleRate: 44100 } : parsed;
  const { result, resolved, frames } = await streamEncode(chunks, source);
  results.push(result);
  check(
    `streaming (${label}) bytes == reference.wem`,
    equal(result.data, referenceWem),
    `bytes=${result.data.byteLength}, packets=${result.stats.audioPackets}, chunks=${chunks.length}`,
  );
  check(
    `streaming (${label}) session selection`,
    sameSelection(resolved, SIX_CHANNEL),
    describe(resolved),
  );
  check(
    `streaming (${label}) pcmFrames() reports the pushed frames exactly`,
    Number.isSafeInteger(frames) && frames === framesPer,
    `pcmFrames=${frames}, pushed=${framesPer}, type=${typeof frames}`,
  );
}

check(
  "streaming outputs byte-identical across all chunkings",
  equal(results[0].data, results[1].data) && equal(results[0].data, results[2].data),
);

// --- 3) selection behaviour (directly over the nodejs core) -----------------
const core = await import("./pkg-node/wem_wasm.js");

await expectWemError(
  "unknown version code -> WEM_ERR_FORMAT_UNSUPPORTED",
  () => new core.WemEncoder(7, 6, 44100),
  "WEM_ERR_FORMAT_UNSUPPORTED",
);

await expectWemError(
  "unknown version label -> WEM_ERR_FORMAT_UNSUPPORTED",
  () => new core.WemSession("2011", 6, 44100),
  "WEM_ERR_FORMAT_UNSUPPORTED",
);

await expectWemError(
  "malformed version argument -> WEM_ERR_STATE_ERROR",
  () => new core.WemEncoder(true, 6, 44100),
  "WEM_ERR_STATE_ERROR",
);

await expectWemError(
  "non-positive geometry -> WEM_ERR_STATE_ERROR",
  () => new core.WemEncoder(0, 0, 44100),
  "WEM_ERR_STATE_ERROR",
);

await expectWemError(
  "unsatisfiable selection (2013, 2ch/44100) -> WEM_ERR_PROFILE_NOT_FOUND",
  () => new core.WemEncoder(0, 2, 44100),
  "WEM_ERR_PROFILE_NOT_FOUND",
);

await expectWemError(
  "auto-selection without a compiled geometry -> WEM_ERR_PROFILE_NOT_FOUND",
  () => new core.WemSession(null, 1, 8000),
  "WEM_ERR_PROFILE_NOT_FOUND",
);

// --- 4) error mapping -------------------------------------------------------
await expectWemError(
  "WAV geometry vs explicit selection -> WEM_ERR_GEOMETRY_MISMATCH",
  () => encodeWav(wavBytes, { version: 0, channels: 2, sampleRate: 48000 }),
  "WEM_ERR_GEOMETRY_MISMATCH",
);

await expectWemError(
  "not a signed-16 PCM WAV -> WEM_ERR_FORMAT_UNSUPPORTED",
  () => parseWav(new Uint8Array([1, 2, 3, 4])),
  "WEM_ERR_FORMAT_UNSUPPORTED",
);

await expectWemError(
  "unaligned chunk -> WEM_ERR_GEOMETRY_MISMATCH",
  () => {
    const s = new core.WemSession(0, 6, 44100);
    s.push(parsed.pcm.slice(0, 13)); // 13 bytes: not a multiple of 12
  },
  "WEM_ERR_GEOMETRY_MISMATCH",
);

await expectWemError(
  "below the 4096-frame minimum -> WEM_ERR_INPUT_TOO_SHORT",
  () => {
    const s = new core.WemSession(0, 6, 44100);
    s.push(slice(0, 1024));
    s.finish();
  },
  "WEM_ERR_INPUT_TOO_SHORT",
);

check(
  "WEM_ERROR_CODES mirrors the include/wem.h table",
  WEM_ERROR_CODES.length === 8 &&
    WEM_ERROR_CODES[0] === "WEM_OK" &&
    WEM_ERROR_CODES[6] === "WEM_ERR_INTERNAL" &&
    WEM_ERROR_CODES[7] === "WEM_ERR_INPUT_MALFORMED",
  WEM_ERROR_CODES.join(", "),
);

// --- 5) 2ch/48000 configuration (fully registered: positive case) -----------
// The selection must name the 2ch configuration even though the 6ch one is
// the bundle's default: this catches accidential fallback to a default.
const TWO_CHANNEL = {
  versionCode: 0,
  version: "2013",
  generation: "2013.2",
  channels: 2,
  sampleRate: 48000,
  description: "2ch/48000Hz/2013",
};

let twoChannelEncoder = null;
let twoChannelError = null;
try {
  twoChannelEncoder = await createEncoder({ version: 0, channels: 2, sampleRate: 48000 });
} catch (error) {
  twoChannelError = error;
}
check(
  "2ch/48000 selection builds through the wasm kernel",
  twoChannelEncoder !== null && twoChannelError === null,
  twoChannelError
    ? `code=${twoChannelError.code ?? "<no code>"}, message="${twoChannelError.message ?? twoChannelError}"`
    : "built",
);

if (twoChannelEncoder) {
  check(
    "2ch encoder reports the resolved 2ch/48000Hz/2013 selection",
    sameSelection(twoChannelEncoder.selection(), TWO_CHANNEL),
    describe(twoChannelEncoder.selection()),
  );
  // Encode a deterministic 8192-frame 2ch/48k stream, integer arithmetic
  // only: the positive case is that encoding succeeds with the right stats,
  // not the audio quality (that is covered by the Python E2E suite).
  const frames = 8192;
  const twoChannelPcm = new Uint8Array(frames * 2 * 2);
  const twoChannelView = new DataView(twoChannelPcm.buffer);
  let lcg = 0x9e3779b9 | 0;
  const lcgNext = () => {
    lcg = (Math.imul(lcg, 48271) + 11) | 0;
    return lcg >> 20; // [-2048, 2047]
  };
  for (let f = 0; f < frames; f++) {
    const tone0 = f % 64 < 32 ? 6000 : -6000;
    const tone1 = f % 96 < 48 ? 5000 : -5000;
    twoChannelView.setInt16(f * 4, tone0 + lcgNext(), true);
    twoChannelView.setInt16(f * 4 + 2, tone1 + lcgNext(), true);
  }
  const twoChannelResult = twoChannelEncoder.encodePcm16Interleaved(twoChannelPcm);
  check(
    "2ch encode produces a WEM stream through the wasm kernel",
    twoChannelResult.data.byteLength > 0 &&
      twoChannelResult.stats.channels === 2 &&
      twoChannelResult.stats.pcmFrames === frames &&
      twoChannelResult.stats.audioPackets > 0 &&
      twoChannelResult.stats.shortPackets + twoChannelResult.stats.longPackets ===
        twoChannelResult.stats.audioPackets,
    `bytes=${twoChannelResult.data.byteLength}, packets=${twoChannelResult.stats.audioPackets}`,
  );
  twoChannelEncoder.destroy();
}

// --- 6) decoding: WEM -> interleaved f32, through the wrapper ---------------
// The runtime check the compile-only shell lane could not write: the committed
// paired-build container goes in through the wrapper's decode surface
// (createDecoder / decodeWem over the wasm `WemDecoder`), and what comes out is
// compared against the WAV that container was produced from. Both files are
// committed, so the comparison is live and nothing here is a recorded number.

// The source PCM as f32 at the kernel's own normalization (i16 / 32768.0).
const sourceView = new DataView(
  parsed.pcm.buffer,
  parsed.pcm.byteOffset,
  parsed.pcm.byteLength,
);
const sourceSamples = new Float32Array(parsed.pcm.byteLength / 2);
let sourcePeak = 0;
for (let i = 0; i < sourceSamples.length; i++) {
  const sample = sourceView.getInt16(i * 2, true) / 32768;
  sourceSamples[i] = sample;
  sourcePeak = Math.max(sourcePeak, Math.abs(sample));
}

/** `max |decoded - source|` over the samples both sides have. */
function reconstructionError(decodedPcm) {
  const shared = Math.min(decodedPcm.length, sourceSamples.length);
  let peak = 0;
  for (let i = 0; i < shared; i++) {
    peak = Math.max(peak, Math.abs(decodedPcm[i] - sourceSamples[i]));
  }
  return { peak, shared };
}

/** One streaming decode through the wrapper, collecting steps as it goes. */
async function streamDecode(chunks) {
  const decoder = await createDecoder();
  const steps = [];
  try {
    for (const chunk of chunks) steps.push(decoder.push(chunk));
    steps.push(decoder.finish());
  } finally {
    decoder.destroy();
  }
  let announcements = 0;
  let samples = 0;
  let pcmBeforeHeader = 0;
  for (const step of steps) {
    if (step.header) announcements += 1;
    if (step.error) throw new Error(`decode refused: ${step.error.code}: ${step.error.message}`);
    if (samples === 0 && step.pcm.length > 0 && step.header === null) pcmBeforeHeader += 1;
    samples += step.pcm.length;
  }
  const total = new Float32Array(samples);
  let offset = 0;
  for (const step of steps) {
    total.set(step.pcm, offset);
    offset += step.pcm.length;
  }
  return { steps, total, announcements, pcmBeforeHeader };
}

const oneShot = await decodeWem(referenceWem);
const declaredFrames = oneShot.frames;
check(
  "decode(reference.wem): declared geometry and frame count",
  oneShot.channels === 6 &&
    oneShot.sampleRate === 44100 &&
    declaredFrames === RECORDING_FRAMES &&
    oneShot.pcm.length === declaredFrames * 6 &&
    oneShot.setup instanceof Uint8Array &&
    oneShot.setup.byteLength === 201,
  `channels=${oneShot.channels}, sampleRate=${oneShot.sampleRate}, frames=${declaredFrames}, ` +
    `samples=${oneShot.pcm.length}, setup=${oneShot.setup.byteLength} bytes`,
);
// The same fact the C ABI announces and the facade exposes. It is checked
// separately from `frames` above because they are different claims: this one
// comes from the header, before any block, and the other is counted from the
// steps that delivered it.
check(
  "decode(reference.wem): the header announces the declared frame count",
  oneShot.totalFrames === RECORDING_FRAMES,
  `totalFrames=${oneShot.totalFrames}, expected=${RECORDING_FRAMES}`,
);

const oneShotError = reconstructionError(oneShot.pcm);
check(
  "decode(reference.wem) reconstructs tests/fixtures/input.wav",
  oneShotError.shared === sourceSamples.length &&
    oneShotError.peak <= 2 * sourcePeak,
  `max|error|=${oneShotError.peak.toExponential(3)} against a source peak of ` +
    `${sourcePeak.toFixed(6)} (${((oneShotError.peak / sourcePeak) * 100).toFixed(3)}%), ` +
    `${oneShotError.shared} samples compared`,
);

// Streaming, as the C ABI frames it: Init -> push* -> Finish. The announced
// header is the only place the geometry appears, and it arrives before PCM.
const wemChunkPlans = {
  "single-chunk": [referenceWem],
  "fixed-8192-bytes": (() => {
    const out = [];
    for (let off = 0; off < referenceWem.byteLength; off += 8192) {
      out.push(referenceWem.slice(off, Math.min(off + 8192, referenceWem.byteLength)));
    }
    return out;
  })(),
  "irregular": (() => {
    const sizes = [1, 40960, 3, 4096, 648];
    const out = [];
    let cursor = 0;
    while (cursor < referenceWem.byteLength) {
      const end = Math.min(cursor + sizes[out.length % sizes.length], referenceWem.byteLength);
      out.push(referenceWem.slice(cursor, end));
      cursor = end;
    }
    return out;
  })(),
};

const decodedStreams = [];
for (const [label, chunks] of Object.entries(wemChunkPlans)) {
  const { total, announcements, pcmBeforeHeader } = await streamDecode(chunks);
  decodedStreams.push(total);
  check(
    `streaming decode (${label}, ${chunks.length} chunk(s)) delivers the declared frames`,
    total.length === RECORDING_FRAMES * 6,
    `samples=${total.length}, frames=${total.length / 6}`,
  );
  check(
    `streaming decode (${label}) announces the header exactly once, before any PCM`,
    announcements === 1 && pcmBeforeHeader === 0,
    `announcements=${announcements}, PCM-before-header steps=${pcmBeforeHeader}`,
  );
  check(
    `streaming decode (${label}) samples == one-shot decode samples`,
    equal(new Uint8Array(total.buffer, total.byteOffset, total.byteLength), new Uint8Array(oneShot.pcm.buffer, oneShot.pcm.byteOffset, oneShot.pcm.byteLength)),
  );
}

check(
  "streaming decode outputs identical across all chunkings",
  equal(
    new Uint8Array(decodedStreams[0].buffer, decodedStreams[0].byteOffset, decodedStreams[0].byteLength),
    new Uint8Array(decodedStreams[1].buffer, decodedStreams[1].byteOffset, decodedStreams[1].byteLength),
  ) &&
    equal(
      new Uint8Array(decodedStreams[0].buffer, decodedStreams[0].byteOffset, decodedStreams[0].byteLength),
      new Uint8Array(decodedStreams[2].buffer, decodedStreams[2].byteOffset, decodedStreams[2].byteLength),
    ),
);

// Identical bytes decode identically: an empty chunk is a no-op, and a second
// decode of the same container is the same samples.
const emptyChunkDecode = await streamDecode([new Uint8Array(0), referenceWem, new Uint8Array(0)]);
check(
  "an empty chunk is a no-op for the decode (include/wem.h section 5)",
  equal(
    new Uint8Array(emptyChunkDecode.total.buffer, emptyChunkDecode.total.byteOffset, emptyChunkDecode.total.byteLength),
    new Uint8Array(oneShot.pcm.buffer, oneShot.pcm.byteOffset, oneShot.pcm.byteLength),
  ),
);

// Refusals travel on the step (the C ABI's order: pcm_cb, then the code), and
// the class is the one include/wem.h section 5 names for the input's own bytes.
await expectWemError(
  "bytes that are not a container this revision parses -> WEM_ERR_INPUT_MALFORMED",
  () => decodeWem(new Uint8Array([1, 2, 3, 4])),
  "WEM_ERR_INPUT_MALFORMED",
);

await expectWemError(
  "a container truncated before its declared frame count -> WEM_ERR_INPUT_MALFORMED",
  () => decodeWem(referenceWem.slice(0, 8192)),
  "WEM_ERR_INPUT_MALFORMED",
);

{
  // A refusal is *returned on the step*, not thrown. The wasm shell reports a
  // decode refusal the way the C ABI does — the samples this step completed,
  // then the code — and only WEM_ERR_INTERNAL throws. `w_format_tag` (offset
  // 20, 0xFFFF in every Wwise container) is cleared here, which is a container
  // that parses but is not Wwise Vorbis.
  const foreign = referenceWem.slice();
  foreign[20] = 0x00;
  foreign[21] = 0x00;
  const decoder = await createDecoder();
  const step = decoder.push(foreign);
  check(
    "a refusal is returned on the step, not thrown (the kernel's delivery order)",
    step.error !== null &&
      step.error.code === "WEM_ERR_FORMAT_UNSUPPORTED" &&
      step.pcm.length === 0 &&
      step.header === null,
    `code=${step.error?.code ?? "<null>"}, message="${step.error?.message ?? ""}"`,
  );
  const repeated = decoder.push(foreign);
  check(
    "a refused push leaves the handle usable: nothing is skipped, the same refusal repeats",
    repeated.error !== null && repeated.error.code === "WEM_ERR_FORMAT_UNSUPPORTED",
    `code=${repeated.error?.code ?? "<null>"}, message="${repeated.error?.message ?? ""}"`,
  );
  decoder.destroy();
}

{
  const decoder = await createDecoder();
  decoder.push(referenceWem);
  decoder.finish();
  await expectWemError(
    "push after Finish -> WEM_ERR_STATE_ERROR (Finish is terminal)",
    () => decoder.push(referenceWem),
    "WEM_ERR_STATE_ERROR",
  );
  await expectWemError(
    "Finish twice -> WEM_ERR_STATE_ERROR",
    () => decoder.finish(),
    "WEM_ERR_STATE_ERROR",
  );
  decoder.destroy();
}

await expectWemError(
  "a WEM chunk of the wrong type -> WEM_ERR_STATE_ERROR",
  () => decodeWem("not bytes"),
  "WEM_ERR_STATE_ERROR",
);

// --- summary ------------------------------------------------------------------
if (failures > 0) {
  console.error(`\n${failures} check(s) FAILED`);
  process.exit(1);
}
console.log("\nALL NODE PARITY CHECKS PASSED");
console.log(
  `reference: ${referenceWem.byteLength} bytes identical to tests/fixtures/reference.wem`,
);
console.log(
  `decode: reference.wem -> ${declaredFrames} frames x 6ch, max|error| against ` +
    `tests/fixtures/input.wav ${oneShotError.peak.toExponential(3)} ` +
    `(source peak ${sourcePeak.toFixed(6)})`,
);