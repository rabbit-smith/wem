# WEM browser demo

Static page: drag a `.wav` → streamed encode in WebAssembly → download the
`.wem`; drag a `.wem` → streamed decode → download raw interleaved f32 PCM.

The page loads one artifact — the wasm module — plus the file you pick. The
profile data (Vorbis setup, codebooks, psychoacoustic tables) is compiled
into that module, so there is no profile index, manifest, resource file,
profile name or data directory to fetch or cache: an **encode** configuration
is selected by the structured profile selection of `include/wem.h` (Wwise
generation + PCM geometry), auto-selected from the WAV's own geometry; a
**decode** takes no selection at all, because a WEM is self-describing and the
geometry arrives on the step that parses its setup packet.

## Run

```sh
make wasm-build                              # writes js/pkg (wasm-pack web build)
node examples/wasm-demo/serve.mjs            # port 8090 (or: [port])
# open http://localhost:8090/examples/wasm-demo/
```

The server (zero dependencies) serves the repo root so the demo can reach
`../../js/pkg/wem_wasm.js` + `wem_wasm_bg.wasm` (the wasm-pack **web** build;
the same build from `js/` is `npm run build:web`) via a relative URL.
No query parameters are used.

The page itself has no build step, but there is nothing to load until
`make wasm-build` has written `js/pkg`: the packages are wasm-pack output and
are not committed (`.gitignore`), so a fresh checkout serves the demo only
after that build.

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
3. **Output**: `finish()` returns the container bytes; offered as a
   download link.

Naming one generation explicitly is the same call with a version argument:

```js
const explicit = new WemSession(0, 6, 44100);        // 0 = Wwise 2013 (WemVersion)
const labelled = new WemSession("2013", 2, 48000);   // label or "2013.2"
```

## How it decodes

1. **Init**: `new WemDecoder()` — no arguments (include/wem.h section 5).
2. **Stream**: `WemDecoder` (Init → push* → Finish) fed 512 KB chunks of the
   WEM. Each `push` *returns* the step it completed (the shell's translation of
   the C ABI's `pcm_cb`): the one-time header announcement when this was the
   step that resolved it, the frames this chunk completed as a `Float32Array`,
   and a refusal if one stopped it. Chunk boundaries never move a sample, and
   an empty chunk is a no-op.
3. **Output**: the samples, written as raw **interleaved little-endian f32 at
   ±1.0 full scale, with no header** — the decoder's own output, not a
   conversion of it. A WAV writer here would have to choose a sample format, a
   chunk layout and a header, and a wrong one would look more plausible than
   the raw samples do; the summary states the geometry, the sample and frame
   counts, and the setup packet's length, which is what makes the file
   interpretable.

Two things this page cannot show, and does not pretend to:

- **The declared frame count is not on the wasm step.** The C ABI's
  `header_cb` carries `total_frames` (the container's
  `dw_total_pcm_frames`); the wasm shell's header object carries
  `channels`/`sampleRate`/`setup` only, so the page reports the frames it
  actually received. On a successful decode those are equal — the kernel
  delivers exactly the declared count or reports a mismatch — but the page
  restates nothing it cannot read.
- **The geometry is the only channel count available**, which is why the header
  announcement is collected before the samples are interpreted.

Errors: a refusal that is not a defect arrives on the step (`step.error`) with
the stable `WEM_ERR_*` code — the decode-side malformed-input class is
`WEM_ERR_INPUT_MALFORMED`, and a container this build does not carry is
`WEM_ERR_FORMAT_UNSUPPORTED`. The samples the packets before the refusal
completed have already been collected, which is the C ABI's `pcm_cb`-then-code
order. `WEM_ERR_INTERNAL` (a defect) throws from the shell, and on wasm32 a
kernel panic traps the instance instead of returning.

### Why the page loads the wasm package, not the typed wrapper

`js/src/index.ts` is the typed wrapper (`Encoder`, `StreamSession`,
`createDecoder`/`decodeWem`) — TypeScript, consumed by Node (native type
stripping) and by bundlers. A static page has no build step and no TypeScript
loader, so `demo.mjs` imports the wasm-pack web build directly for both
directions, exactly as it always has for encode. The wrapper's decode surface
has its own **runtime** check, against the committed reference WEM:
`make wasm-build` then `node js/test-node.mjs`, which is also the only place the
wrapper is exercised outside a compiler.

## Memory notes (read before scaling up)

- A 6ch/44.1kHz s16 WAV is ≈ 529 KB/s ≈ **127 MB per 4-minute song**. The
  WAV read into memory is the dominant encode-side cost; many browsers cap a
  single ArrayBuffer well below that.
- The streaming encode session itself is bounded on the **input** side
  (9216-sample ring + look-ahead), but the **emitted audio packets
  accumulate** (the container must carry them; output ≈ 8–12% of the PCM
  size). 139k frames of representative audio → ≈ 109 KB of WEM.
- The decode session is bounded too, but this page accumulates the whole
  decoded PCM to offer one download: **4 bytes per sample**, so 6ch/44.1kHz is
  ≈ 1 MB/s ≈ 250 MB per 4-minute song — twice the WAV. A page that only needs
  to play or analyse the audio should consume each step's `pcm` and drop it.
- For large tracks: stream in chunks (decode with WebCodecs /
  `AudioWorklet` instead of one `ArrayBuffer`), run the loops in a
  **Web Worker** (the shell is worker-friendly: no DOM in it), and/or process
  per section.

## Production deployment (recommended)

- **Host** `wem_wasm_bg.wasm` with `Content-Type: application/wasm`
  (enables `instantiateStreaming`). That binary plus the input file is the whole
  deployment: there are no profile files to mirror or cache.
- Pin the built module in your own deployment — a vendored copy in your app's
  repository or a CDN — and serve it from a versioned path, so a loader can be
  sure which build it fetched. What you ship is a copy of what `make wasm-build`
  produced; this repository keeps no wasm module of its own.