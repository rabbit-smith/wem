/**
 * WEM browser demo — streaming encode in the tab.
 *
 * Loads the wasm-bindgen web build (../../js/pkg, relative .wasm fetch),
 * fetches the profile bundle from relative paths (SHA-256 verification is
 * done by the kernel on load), then streams a dropped .wav through
 * WemSession in frame-aligned chunks and offers the result for download.
 */

import init, { WemEncoder, WemSession, wem_parse_wav } from "../../js/pkg/wem_wasm.js";

const statusEl = document.getElementById("status");
const dropEl = document.getElementById("drop");
const fileEl = document.getElementById("file");
const barEl = document.getElementById("progress");
const resultEl = document.getElementById("result");
const dlEl = document.getElementById("dl");
const dlBtnEl = document.getElementById("dlbtn");
const againEl = document.getElementById("again");
const summaryEl = document.getElementById("summary");

const profileBase = new URLSearchParams(location.search).get("profileBase")
  ?? "../../src/wwise_wem/data/profiles";

// bytes per pushed chunk, aligned to whole 6ch s16 frames (12 bytes)
const CHUNK_BYTES = Math.floor((512 * 1024) / 12) * 12;

async function fetchBytes(url) {
  const res = await fetch(url);
  if (!res.ok) throw new Error(`fetch ${url}: ${res.status} ${res.statusText}`);
  return new Uint8Array(await res.arrayBuffer());
}

async function loadProfileBundle(base) {
  const indexBytes = await fetchBytes(`${base}/index.json`);
  const index = JSON.parse(new TextDecoder().decode(indexBytes));
  if (index.schema !== "wwise-wem.profile-index.v1") {
    throw new Error(`unexpected index schema: ${index.schema}`);
  }
  const profileName = index.default;
  const entry = index.profiles?.[profileName];
  if (!entry) throw new Error(`index has no '${profileName}' entry`);

  // manifest + every logical resource, as profiles-dir-relative files
  const manifestBytes = await fetchBytes(`${base}/${entry.manifest}`);
  const manifest = JSON.parse(new TextDecoder().decode(manifestBytes));
  const files = new Map();
  files.set(entry.manifest, manifestBytes);
  for (const [resourceName, resource] of Object.entries(manifest.resources ?? {})) {
    const key = `${profileName}/${resource.path}`;
    files.set(key, await fetchBytes(`${base}/${resource.path}`));
  }
  const setupSha256 = manifest.resources?.["vorbis.setup"]?.sha256;
  if (!setupSha256) throw new Error("manifest lacks the vorbis.setup resource");
  return { indexBytes, files, profileName, setupSha256 };
}

// ---------------------------------------------------------------------------
// startup: wasm + profile
// ---------------------------------------------------------------------------

let encoder = null;
let bundle = null;

try {
  statusEl.textContent = "loading wasm…";
  await init(); // resolves wem_wasm_bg.wasm relative to this module
  statusEl.textContent = "loading profile…";
  bundle = await loadProfileBundle(profileBase);
  // resolve geometry + verify the bundle through the kernel (one-shot handle)
  encoder = new WemEncoder(bundle.indexBytes, bundle.files);
  const info = encoder.profile_info();
  statusEl.textContent =
    `ready — profile ${info.name} (${info.channels}ch @ ${info.sampleRate}Hz, ` +
    `setup ${info.setupSha256.slice(0, 12)}…), drop a .wav`;
} catch (error) {
  statusEl.textContent = `startup failed: ${error.message}`;
  console.error(error);
}

// ---------------------------------------------------------------------------
// encode flow
// ---------------------------------------------------------------------------

function setProgress(fraction) {
  barEl.style.width = `${Math.round(fraction * 100)}%`;
}

async function encodeFile(file) {
  if (!encoder || !bundle) {
    statusEl.textContent = "encoder not ready yet — retry in a moment";
    return;
  }
  const t0 = performance.now();
  statusEl.textContent = `reading ${file.name}…`;
  setProgress(0);
  resultEl.style.display = "none";

  let wav;
  try {
    // kernel WAV parse (signed-16 PCM only)
    wav = wem_parse_wav(new Uint8Array(await file.arrayBuffer()));
  } catch (error) {
    statusEl.textContent = `WAV rejected by kernel: ${error.message}`;
    return;
  }

  const profile = encoder.profile_info();
  if (wav.sampleRate !== profile.sampleRate || wav.channels !== profile.channels) {
    statusEl.textContent =
      `geometry mismatch: file is ${wav.channels}ch @ ${wav.sampleRate}Hz, ` +
      `profile is ${profile.channels}ch @ ${profile.sampleRate}Hz`;
    return;
  }

  const totalFrames = wav.frames;
  const session = new WemSession(
    bundle.indexBytes,
    bundle.files,
    bundle.setupSha256,
    bundle.profileName,
  );

  let result;
  try {
    const pcm = wav.pcm;
    let offset = 0;
    while (offset < pcm.byteLength) {
      const chunk = pcm.slice(offset, Math.min(offset + CHUNK_BYTES, pcm.byteLength));
      offset += chunk.byteLength;
      session.push(chunk);
      setProgress(session.pcm_frames() / totalFrames);
      statusEl.textContent = `encoding… ${session.pcm_frames()} / ${totalFrames} frames`;
    }
    statusEl.textContent = "finishing (end-of-stream tail)…";
    result = session.finish();
  } catch (error) {
    statusEl.textContent = `encode failed: ${error.message}`;
    console.error(error);
    return;
  } finally {
    session.free();
  }

  const blob = new Blob([result.data], { type: "application/octet-stream" });
  const url = URL.createObjectURL(blob);
  const stamp = new Date().toISOString().replace(/[:.]/g, "-");
  dlEl.href = url;
  dlEl.download = `wem-${stamp}.wem`;
  summaryEl.textContent = [
    `input      ${file.name}: ${wav.channels}ch @ ${wav.sampleRate}Hz, ${totalFrames} frames`,
    `output     ${result.totalLen} bytes`,
    `sha256     ${result.sha256Hex}`,
    `packets    ${result.stats.audioPackets} (short ${result.stats.shortPackets} / long ${result.stats.longPackets})`,
    `profile    ${profile.name} (${profile.setupSha256})`,
    `time       ${(performance.now() - t0) / 1000} s (main thread)`,
  ].join("\n");
  resultEl.style.display = "block";
  statusEl.textContent = "done — download below";
  setProgress(1);
}

// ---------------------------------------------------------------------------
// UI wiring
// ---------------------------------------------------------------------------

dropEl.addEventListener("click", () => fileEl.click());
fileEl.addEventListener("change", () => {
  if (fileEl.files[0]) encodeFile(fileEl.files[0]);
});
for (const type of ["dragover", "dragenter"]) {
  dropEl.addEventListener(type, (event) => {
    event.preventDefault();
    dropEl.classList.add("over");
  });
}
for (const type of ["dragleave", "drop"]) {
  dropEl.addEventListener(type, (event) => {
    event.preventDefault();
    dropEl.classList.remove("over");
  });
}
dropEl.addEventListener("drop", (event) => {
  const file = [...event.dataTransfer.files].find(
    (f) => f.name.toLowerCase().endsWith(".wav") || /wav/i.test(f.type),
  );
  if (file) encodeFile(file);
  else statusEl.textContent = "expected a .wav file";
});
againEl.addEventListener("click", () => {
  resultEl.style.display = "none";
  setProgress(0);
  fileEl.value = "";
});
dlBtnEl.addEventListener("click", () => dlEl.click());
