#!/usr/bin/env node
/**
 * wwise-wem-wasm — Node parity gate.
 *
 * Proves the wasm shell encodes byte-exactly against the kernel golden
 * contract, through the package's own entry point (src/index.ts, run via
 * Node's native type stripping on >= 22.18):
 *
 *   1. ONE-SHOT: fixture PCM -> WEM, sha256 must equal the golden.
 *   2. STREAMING CHUNK CONSISTENCY: the same PCM fed to sessions with
 *      three different chunkings (single chunk, fixed-size chunks,
 *      irregular frame-aligned chunks) must each reproduce the golden —
 *      chunk boundaries never change the output bytes (include/wem.h).
 *   3. ERROR CONTRACT: a wrong setup reference must throw a JS Error
 *      whose `code` is the stable WEM_ERR_* string.
 *
 * Run: node js/test-node.mjs   (requires js/pkg-node built: npm run build:node)
 */

import { readFileSync, readdirSync, statSync } from "node:fs";
import { dirname, join, relative } from "node:path";
import { fileURLToPath } from "node:url";
import {
  initWasm,
  loadProfileBundle,
  encodeWav,
  parseWav,
  createStreamSession,
} from "./src/index.ts";

const here = dirname(fileURLToPath(import.meta.url));
const repoRoot = join(here, "..");

// Pinned kernel golden (tests/fixtures/input.wav -> reference encode).
const GOLDEN_SHA256 =
  "17851d26c6210b85e498ae0452d2562d7b9e2c3e9e795c459656b9c9d8d35247";
const PROFILE_NAME = "wwise2013-6ch-44100";
const SETUP_SHA256 =
  "3ef56cbd6e6a66a5474005db05912624487faa555fb2cdfed130f606b322e4e3";

let failures = 0;

function check(label, ok, detail = "") {
  const mark = ok ? "PASS" : "FAIL";
  console.log(`[${mark}] ${label}${detail ? ` — ${detail}` : ""}`);
  if (!ok) failures += 1;
}

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

/** Walk the profiles tree as (profiles-dir-relative path, bytes) pairs. */
function readProfileBundle() {
  const profilesDir = join(repoRoot, "src/wwise_wem/data/profiles");
  const walk = (dir) => {
    const out = [];
    for (const entry of readdirSync(dir)) {
      const p = join(dir, entry);
      if (statSync(p).isDirectory()) out.push(...walk(p));
      else out.push(p);
    }
    return out;
  };
  const indexBytes = new Uint8Array(readFileSync(join(profilesDir, "index.json")));
  const files = new Map();
  for (const p of walk(profilesDir)) {
    if (p.endsWith("index.json")) continue;
    // POSIX keys regardless of platform (the kernel bytes contract).
    const rel = relative(profilesDir, p).split(join).join("/");
    files.set(rel, new Uint8Array(readFileSync(p)));
  }
  return { indexBytes, files };
}

const { indexBytes, files } = readProfileBundle();
const wavBytes = new Uint8Array(readFileSync(join(repoRoot, "tests/fixtures/input.wav")));

await initWasm();

// --- 1) one-shot parity ----------------------------------------------------
const bundle = await loadProfileBundle(indexBytes, files);
check(
  "profile bundle verified (kernel SHA-256 entries)",
  bundle.info.name === PROFILE_NAME && bundle.info.setupSha256 === SETUP_SHA256,
  `name=${bundle.info.name}, setup=${bundle.info.setupSha256}`,
);

const t0 = Date.now();
const oneShot = await encodeWav(wavBytes, bundle);
check(
  "one-shot encode sha256 == golden",
  oneShot.sha256Hex === GOLDEN_SHA256,
  `sha256=${oneShot.sha256Hex}, bytes=${oneShot.totalLen}, ${Date.now() - t0}ms`,
);

// --- 2) streaming chunk consistency ----------------------------------------
const { pcm } = await parseWav(wavBytes);
const framesPer = pcm.byteLength / 12; // 6ch x s16 = 12 bytes/frame
if (framesPer !== 139398) {
  check("fixture PCM geometry", false, `frames=${framesPer}`);
  process.exit(1);
}

async function streamEncode(chunks) {
  const session = await createStreamSession(bundle);
  try {
    for (const chunk of chunks) {
      session.push(chunk);
    }
    return session.finish();
  } finally {
    session.destroy();
  }
}

const slice = (from, to) => pcm.slice(from * 12, to * 12);

const plans = {
  "single-chunk": [pcm],
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
  const r = await streamEncode(chunks);
  results.push(r);
  check(
    `streaming (${label}) sha256 == golden`,
    r.sha256Hex === GOLDEN_SHA256,
    `sha256=${r.sha256Hex}, packets=${r.stats.audioPackets}, chunks=${chunks.length}`,
  );
}

const equal = (x, y) =>
  x.byteLength === y.byteLength && [...x].every((v, i) => v === y[i]);
check(
  "streaming outputs byte-identical across all chunkings",
  equal(results[0].data, results[1].data) && equal(results[0].data, results[2].data),
);

// --- 3) error contract (directly over the nodejs core) -----------------------
const core = await import("./pkg-node/wem_wasm.js");

await expectWemError(
  "wrong setup sha reference -> WEM_ERR_PROFILE_NOT_FOUND",
  () => {
    new core.WemSession(indexBytes, files, "0".repeat(64), PROFILE_NAME);
  },
  "WEM_ERR_PROFILE_NOT_FOUND",
);

await expectWemError(
  "name cross-check failure -> WEM_ERR_STATE_ERROR",
  () => {
    new core.WemSession(indexBytes, files, SETUP_SHA256, "not-this-profile");
  },
  "WEM_ERR_STATE_ERROR",
);

await expectWemError(
  "unaligned chunk -> WEM_ERR_GEOMETRY_MISMATCH",
  () => {
    const s = new core.WemSession(indexBytes, files, SETUP_SHA256, PROFILE_NAME);
    s.push(pcm.slice(0, 13)); // 13 bytes: not a multiple of 12
  },
  "WEM_ERR_GEOMETRY_MISMATCH",
);

const DRAFT_PROFILE = "wwise2013-2ch-48000";

// --- 4) draft profile (setup pending corpus export) --------------------------
// The 2ch/48000 draft profile ships its static data (quality curves etc.); the
// setup packet awaits a paired Wwise export. The kernel must load it through
// the bytes entry and refuse encoding with a clear pending error.
const draftIndexBytes = (() => {
  const index = JSON.parse(new TextDecoder().decode(indexBytes));
  index.default = DRAFT_PROFILE;
  return new TextEncoder().encode(JSON.stringify(index, null, 2));
})();

let draftEncodeError = null;
let draftBuildSucceeded = false;
try {
  const draftEncoder = new core.WemEncoder(draftIndexBytes, files);
  draftBuildSucceeded = true;
  draftEncoder.free();
} catch (error) {
  draftEncodeError = error;
}
check(
  "draft profile files are present in the profile tree",
  files.has(`${DRAFT_PROFILE}/analysis/quality-curves.json`) &&
    files.has(`${DRAFT_PROFILE}/pending.json`) &&
    files.has(`${DRAFT_PROFILE}/manifest.json`),
);
check(
  "draft profile encode attempt -> WEM_ERR_STATE_ERROR with clear pending error",
  !draftBuildSucceeded &&
    draftEncodeError &&
    typeof draftEncodeError.code === "string" &&
    draftEncodeError.code === "WEM_ERR_STATE_ERROR" &&
    typeof draftEncodeError.message === "string" &&
    draftEncodeError.message.includes("setup packet pending") &&
    draftEncodeError.message.includes("requires paired Wwise export"),
  `code=${draftEncodeError?.code ?? "<no code>"}, message="${draftEncodeError?.message ?? draftEncodeError}"`,
);

// --- summary ------------------------------------------------------------------
if (failures > 0) {
  console.error(`\n${failures} check(s) FAILED`);
  process.exit(1);
}
console.log("\nALL NODE PARITY GATES PASSED");
console.log(`golden sha256: ${GOLDEN_SHA256}`);
