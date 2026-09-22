/**
 * WEM browser demo — streaming encode in the tab.
 *
 * Loads the wasm-bindgen web build (../../js/pkg, relative .wasm fetch), then
 * streams a dropped .wav through WemSession in frame-aligned chunks and offers
 * the result for download.
 *
 * Nothing else is fetched: the profile data is compiled into the wasm module,
 * and the profile selection (Wwise generation + PCM geometry, include/wem.h
 * "PROFILE SELECTION") is auto-selected from the WAV's own geometry — no
 * index, manifest, resource file, profile name or data directory is involved.
 */

import init, { WemSession, wem_parse_wav, wem_versions } from "../../js/pkg/wem_wasm.js";

const statusEl = document.getElementById("status");
const dropEl = document.getElementById("drop");
const fileEl = document.getElementById("file");
const barEl = document.getElementById("progress");
const resultEl = document.getElementById("result");
const dlEl = document.getElementById("dl");
const dlBtnEl = document.getElementById("dlbtn");
const againEl = document.getElementById("again");
const summaryEl = document.getElementById("summary");

// ---------------------------------------------------------------------------
// startup: wasm module only
// ---------------------------------------------------------------------------

let ready = false;

try {
  statusEl.textContent = "loading wasm…";
  await init(); // resolves wem_wasm_bg.wasm relative to this module
  const generations = wem_versions()
    .map((version) => `Wwise ${version.label} (${version.generation})`)
    .join(", ");
  statusEl.textContent = `ready — ${generations} compiled in, drop a .wav`;
  ready = true;
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
  if (!ready) {
    statusEl.textContent = "wasm module not ready yet — retry in a moment";
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

  let session;
  let selection;
  try {
    // Auto-selection: no version named, so the compiled generation is
    // resolved from the WAV's own geometry (a geometry no compiled
    // configuration satisfies throws WEM_ERR_PROFILE_NOT_FOUND).
    session = new WemSession(null, wav.channels, wav.sampleRate);
    selection = session.selection();
  } catch (error) {
    statusEl.textContent =
      `no compiled configuration for ${wav.channels}ch @ ${wav.sampleRate}Hz: ` +
      `${error.message}`;
    return;
  }

  const totalFrames = wav.frames;
  const frameBytes = selection.channels * 2;
  const chunkBytes = Math.floor((512 * 1024) / frameBytes) * frameBytes;

  let result;
  try {
    const pcm = wav.pcm;
    let offset = 0;
    while (offset < pcm.byteLength) {
      const chunk = pcm.slice(offset, Math.min(offset + chunkBytes, pcm.byteLength));
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
    `selection  ${selection.description} (version code ${selection.versionCode})`,
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