#!/usr/bin/env node
/**
 * wwise-wem-wasm — Node parity test.
 *
 * Proves the wasm shell encodes byte-exactly against the kernel reference
 * bytes, through the package's own entry point (src/index.ts, run via
 * Node's native type stripping on >= 22.18), and that its profile selection
 * is the structured one of `include/wem.h` (ABI revision 2):
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
 *
 * Run: node js/test-node.mjs   (requires js/pkg-node built: npm run build:node)
 */

import { readFileSync } from "node:fs";
import { dirname, join } from "node:path";
import { fileURLToPath } from "node:url";
import {
  initWasm,
  listVersions,
  parseWav,
  encodeWav,
  createEncoder,
  createStreamSession,
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
  `sha256=${auto.sha256Hex}, bytes=${auto.totalLen}, ${Date.now() - t0}ms`,
);

const explicit = await encodeWav(wavBytes, { version: 0, channels: 6, sampleRate: 44100 });
check(
  "one-shot (explicit selection 0/6ch/44100) bytes == reference.wem",
  equal(explicit.data, referenceWem),
  `sha256=${explicit.sha256Hex}, bytes=${explicit.totalLen}`,
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
  `sha256=${pcmOneShot.sha256Hex}, bytes=${pcmOneShot.totalLen}`,
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
    return { result: session.finish(), resolved };
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
  const { result, resolved } = await streamEncode(chunks, source);
  results.push(result);
  check(
    `streaming (${label}) bytes == reference.wem`,
    equal(result.data, referenceWem),
    `sha256=${result.sha256Hex}, packets=${result.stats.audioPackets}, chunks=${chunks.length}`,
  );
  check(
    `streaming (${label}) session selection`,
    sameSelection(resolved, SIX_CHANNEL),
    describe(resolved),
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
  WEM_ERROR_CODES.length === 7 &&
    WEM_ERROR_CODES[0] === "WEM_OK" &&
    WEM_ERROR_CODES[6] === "WEM_ERR_INTERNAL",
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
    `bytes=${twoChannelResult.data.byteLength}, packets=${twoChannelResult.stats.audioPackets}, sha=${twoChannelResult.sha256Hex.slice(0, 12)}...`,
  );
  twoChannelEncoder.destroy();
}

// --- summary ------------------------------------------------------------------
if (failures > 0) {
  console.error(`\n${failures} check(s) FAILED`);
  process.exit(1);
}
console.log("\nALL NODE PARITY CHECKS PASSED");
console.log(
  `reference: ${referenceWem.byteLength} bytes identical to tests/fixtures/reference.wem`,
);
console.log(`reference sha256 (computed, not compared): ${auto.sha256Hex}`);