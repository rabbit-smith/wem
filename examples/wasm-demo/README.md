# WEM browser demo

Static page: drag a `.wav` → streamed encode in WebAssembly → download the `.wem`.

## Run

```sh
node examples/wasm-demo/serve.mjs            # port 8090 (or: [port])
# open http://localhost:8090/examples/wasm-demo/
```

The server (zero dependencies) serves the repo root so the demo can reach,
via relative URLs:

- `../../js/pkg/wem_wasm.js` + `wem_wasm_bg.wasm` — the wasm-pack **web** build
  (committed; rebuild with `cd js && npm run build:web`),
- `../../src/wwise_wem/data/profiles/…` — the profile bundle
  (override with `?profileBase=/path/to/profiles`).

No build step is needed: the wasm artifacts are committed under `js/`.

## How it encodes

1. **Profile**: `index.json` → default profile → `manifest.json` → every
   logical resource (each with its `sha256`). All bytes are passed to the
   kernel as `(path, bytes)`; the kernel re-verifies every SHA-256 on load
   (`wem_profile_bundle_from_bytes` path) and asserts the setup digest.
2. **WAV**: dropped file → kernel `wem_parse_wav` (signed-16 PCM; geometry
   checked against the profile).
3. **Stream**: `WemSession` (Init → push* → Finish) fed in frame-aligned
   512 KB chunks; progress from `pcm_frames()`. Chunk boundaries never
   change the output bytes (include/wem.h contract).
4. **Output**: `finish()` returns container bytes + SHA-256; served as a
   download link.

## Memory notes (read before scaling up)

- A 6ch/44.1kHz s16 WAV is ≈ 529 KB/s ≈ **127 MB per 4-minute song**. The
  WAV read into memory is the dominant cost; many browsers cap a single
  ArrayBuffer well below that.
- The streaming session itself is bounded on the **input** side
  (9216-sample ring + look-ahead), but the **emitted audio packets
  accumulate** (the container must carry them; output ≈ 8–12% of the PCM
  size). 139k frames of fixture audio → ≈ 109 KB of WEM.
- For large tracks: stream in chunks (decode with WebCodecs /
  `AudioWorklet` instead of one `ArrayBuffer`), run the encode loop in a
  **Web Worker** (this package is worker-friendly: no DOM in
  `js/src/index.ts`), and/or encode per section.

## Production deployment (recommended)

- **Host** `wem_wasm_bg.wasm` with `Content-Type: application/wasm`
  (enables `instantiateStreaming`).
- **Cache the profile in IndexedDB** keyed by its SHA-256 (the index/manifest
  are checksum-addressed): fetch → sha256 → compare to manifest → store.
  Re-validate on load (the kernel re-checks everything anyway; the cache is
  only for latency/bandwidth).
- The wasm binary itself: pin its bytes in git or a CDN, add a
  `Content-SHA256` check in your loader if the host is untrusted.
