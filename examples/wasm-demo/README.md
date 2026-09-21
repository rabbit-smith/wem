# WEM browser demo

Static page: drag a `.wav` → streamed encode in WebAssembly → download the `.wem`.

The page loads one artifact — the wasm module — plus the WAV you pick. The
profile bundle (Vorbis setup, codebooks, psychoacoustic tables) is compiled
into that module, so there is no profile index, manifest, resource file,
profile name or data directory to fetch or cache: the encode configuration is
selected by the structured profile selection of `include/wem.h` (Wwise
generation + PCM geometry) and auto-selected from the WAV's own geometry.

## Run

```sh
node examples/wasm-demo/serve.mjs            # port 8090 (or: [port])
# open http://localhost:8090/examples/wasm-demo/
```

The server (zero dependencies) serves the repo root so the demo can reach
`../../js/pkg/wem_wasm.js` + `wem_wasm_bg.wasm` (the committed wasm-pack
**web** build; rebuild with `cd js && npm run build:web`) via a relative URL.
No query parameters are used.

No build step is needed: the wasm artifacts are committed under `js/`.

## How it encodes

1. **Selection**: the WAV is parsed by the kernel (`wem_parse_wav`), then
   `new WemSession(null, wav.channels, wav.sampleRate)` — no version named, so
   the compiled Wwise generation whose configuration satisfies that geometry
   is resolved inside the wasm shell. A geometry no compiled configuration
   satisfies throws `WEM_ERR_PROFILE_NOT_FOUND`; the resolved selection is
   read back with `session.selection()`.
2. **Stream**: `WemSession` (Init → push* → Finish) fed in frame-aligned
   512 KB chunks; progress from `pcm_frames()`. Chunk boundaries never
   change the output bytes (include/wem.h).
3. **Output**: `finish()` returns container bytes + SHA-256; served as a
   download link.

Naming one generation explicitly is the same call with a version argument:

```js
const explicit = new WemSession(0, 6, 44100);        // 0 = Wwise 2013 (WemVersion)
const labelled = new WemSession("2013", 2, 48000);   // label or "2013.2"
```

## Memory notes (read before scaling up)

- A 6ch/44.1kHz s16 WAV is ≈ 529 KB/s ≈ **127 MB per 4-minute song**. The
  WAV read into memory is the dominant cost; many browsers cap a single
  ArrayBuffer well below that.
- The streaming session itself is bounded on the **input** side
  (9216-sample ring + look-ahead), but the **emitted audio packets
  accumulate** (the container must carry them; output ≈ 8–12% of the PCM
  size). 139k frames of representative audio → ≈ 109 KB of WEM.
- For large tracks: stream in chunks (decode with WebCodecs /
  `AudioWorklet` instead of one `ArrayBuffer`), run the encode loop in a
  **Web Worker** (the shell is worker-friendly: no DOM in it), and/or encode
  per section.

## Production deployment (recommended)

- **Host** `wem_wasm_bg.wasm` with `Content-Type: application/wasm`
  (enables `instantiateStreaming`). That binary plus the WAV is the whole
  deployment: there are no profile files to mirror or cache.
- Pin the wasm bytes in git or a CDN, and add a `Content-SHA256` check in
  your loader if the host is untrusted.