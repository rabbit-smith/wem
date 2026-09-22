/**
 * WEM browser demo — streaming encode and decode in the tab.
 *
 * Loads the wasm-bindgen web build (../../js/pkg, relative .wasm fetch), then:
 *
 *   - a dropped .wav is streamed through `WemSession` in frame-aligned chunks
 *     and offered for download as a .wem;
 *   - a dropped .wem is streamed through `WemDecoder` (include/wem.h section 5)
 *     and offered for download as raw interleaved little-endian f32.
 *
 * Nothing else is fetched: the profile data is compiled into the wasm module,
 * and the profile selection (Wwise generation + PCM geometry, include/wem.h
 * "PROFILE SELECTION") is auto-selected from the WAV's own geometry — no
 * index, manifest, resource file, profile name or data directory is involved.
 * The decode direction has no selection at all: a WEM is self-describing, and
 * the geometry arrives on the step that parses its setup packet.
 *
 * The page loads the wasm package directly rather than the typed wrapper in
 * js/src/index.ts: the wrapper is TypeScript, consumed by Node (native type
 * stripping) and by bundlers, and this page has no build step. The wrapper's
 * own runtime decode check is js/test-node.mjs.
 */

import init, { WemSession, WemDecoder, wem_parse_wav, wem_versions } from "../../js/pkg/wem_wasm.js";

const statusEl = document.getElementById("status");
const dropEl = document.getElementById("drop");
const fileEl = document.getElementById("file");
const barEl = document.getElementById("progress");
const resultEl = document.getElementById("result");
const dlEl = document.getElementById("dl");
const dlBtnEl = document.getElementById("dlbtn");
const againEl = document.getElementById("again");
const summaryEl = document.getElementById("summary");

// Both directions are fed in bounded chunks; chunk boundaries never change the
// bytes or the samples (include/wem.h sections 1 and 5).
const CHUNK_BYTES = 512 * 1024;

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
  statusEl.textContent = `ready — ${generations} compiled in, drop a .wav or .wem`;
  ready = true;
} catch (error) {
  statusEl.textContent = `startup failed: ${error.message}`;
  console.error(error);
}

// ---------------------------------------------------------------------------
// shared UI
// ---------------------------------------------------------------------------

function setProgress(fraction) {
  barEl.style.width = `${Math.round(fraction * 100)}%`;
}

function offerDownload(bytes, filename, mime) {
  const blob = new Blob([bytes], { type: mime });
  dlEl.href = URL.createObjectURL(blob);
  dlEl.download = filename;
}

function timestamp() {
  return new Date().toISOString().replace(/[:.]/g, "-");
}

// ---------------------------------------------------------------------------
// encode flow: .wav -> .wem
// ---------------------------------------------------------------------------

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
  const chunkBytes = Math.floor(CHUNK_BYTES / frameBytes) * frameBytes;

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

  offerDownload(result.data, `wem-${timestamp()}.wem`, "application/octet-stream");
  summaryEl.textContent = [
    `input      ${file.name}: ${wav.channels}ch @ ${wav.sampleRate}Hz, ${totalFrames} frames`,
    `output     ${result.data.byteLength} bytes`,
    `packets    ${result.stats.audioPackets} (short ${result.stats.shortPackets} / long ${result.stats.longPackets})`,
    `selection  ${selection.description} (version code ${selection.versionCode})`,
    `time       ${(performance.now() - t0) / 1000} s (main thread)`,
  ].join("\n");
  resultEl.style.display = "block";
  statusEl.textContent = "done — download below";
  setProgress(1);
}

// ---------------------------------------------------------------------------
// decode flow: .wem -> raw interleaved f32
// ---------------------------------------------------------------------------

/**
 * Consume one decode step: the header announcement when this is the step that
 * carried it, this step's samples, and only then a refusal — the order the C
 * ABI delivers them in (`pcm_cb`, then the return code).
 */
function collect(step, decoded) {
  if (step.header) decoded.header = step.header;
  if (step.pcm.length > 0) {
    decoded.blocks.push(step.pcm);
    decoded.samples += step.pcm.length;
  }
  if (step.error) {
    const error = new Error(`${step.error.code}: ${step.error.message}`);
    error.code = step.error.code;
    throw error;
  }
}

/** The decoded samples as one interleaved little-endian f32 buffer. */
function toLittleEndianF32(blocks, sampleCount) {
  const samples = new Float32Array(sampleCount);
  let cursor = 0;
  for (const block of blocks) {
    samples.set(block, cursor);
    cursor += block.length;
  }
  const bytes = new Uint8Array(samples.length * 4);
  const view = new DataView(bytes.buffer);
  for (let i = 0; i < samples.length; i++) {
    view.setFloat32(i * 4, samples[i], true);
  }
  return bytes;
}

async function decodeFile(file) {
  if (!ready) {
    statusEl.textContent = "wasm module not ready yet — retry in a moment";
    return;
  }
  const t0 = performance.now();
  statusEl.textContent = `reading ${file.name}…`;
  setProgress(0);
  resultEl.style.display = "none";

  const bytes = new Uint8Array(await file.arrayBuffer());
  // Init: no selection argument, because a WEM is self-describing.
  const decoder = new WemDecoder();
  const decoded = { header: null, blocks: [], samples: 0 };
  try {
    let offset = 0;
    while (offset < bytes.byteLength) {
      const end = Math.min(offset + CHUNK_BYTES, bytes.byteLength);
      // `subarray` lends the bytes: push copies them into wasm memory.
      collect(decoder.push(bytes.subarray(offset, end)), decoded);
      offset = end;
      setProgress(offset / bytes.byteLength);
      statusEl.textContent = `decoding… ${offset} / ${bytes.byteLength} bytes`;
    }
    statusEl.textContent = "finishing (last frames)…";
    collect(decoder.finish(), decoded);
  } catch (error) {
    // A refusal stops the decode; whatever the earlier packets completed has
    // already been collected, and the code says which class it was.
    statusEl.textContent = `decode refused (${error.code ?? "no code"}): ${error.message}`;
    console.error(error);
    return;
  } finally {
    decoder.free();
  }

  if (decoded.header === null) {
    statusEl.textContent = "decode produced no geometry — nothing to interpret";
    return;
  }
  const channels = decoded.header.channels;
  const frames = decoded.samples / channels;
  const output = toLittleEndianF32(decoded.blocks, decoded.samples);
  offerDownload(output, `pcm-${timestamp()}.f32`, "application/octet-stream");
  summaryEl.textContent = [
    `input      ${file.name}: ${bytes.byteLength} bytes`,
    `output     ${output.byteLength} bytes — raw interleaved little-endian f32, +-1.0 full scale, no header`,
    `geometry   ${channels}ch @ ${decoded.header.sampleRate}Hz, setup packet ${decoded.header.setup.byteLength} bytes`,
    `declared   ${decoded.header.totalFrames} frames (the container's own count)`,
    `samples    ${decoded.samples} (${frames} frames)`,
    `time       ${(performance.now() - t0) / 1000} s (main thread)`,
  ].join("\n");
  resultEl.style.display = "block";
  statusEl.textContent = "done — download below";
  setProgress(1);
}

// ---------------------------------------------------------------------------
// UI wiring
// ---------------------------------------------------------------------------

/** Send a dropped file to the direction its own bytes call for. */
function handleFile(file) {
  const name = file.name.toLowerCase();
  if (name.endsWith(".wav") || /wav/i.test(file.type)) {
    encodeFile(file);
  } else if (name.endsWith(".wem")) {
    decodeFile(file);
  } else {
    statusEl.textContent = "expected a .wav (encode) or a .wem (decode)";
  }
}

dropEl.addEventListener("click", () => fileEl.click());
fileEl.addEventListener("change", () => {
  if (fileEl.files[0]) handleFile(fileEl.files[0]);
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
  const file = event.dataTransfer.files[0];
  if (file) handleFile(file);
  else statusEl.textContent = "expected a .wav or a .wem file";
});
againEl.addEventListener("click", () => {
  resultEl.style.display = "none";
  setProgress(0);
  fileEl.value = "";
});
dlBtnEl.addEventListener("click", () => dlEl.click());