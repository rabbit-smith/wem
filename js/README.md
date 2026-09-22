# wwise-wem-wasm

Browser encoding and decoding for the WEM kernel — the wasm-bindgen shell
(`crates/wem-wasm`) plus a typed JS wrapper. WAV/PCM in, WEM bytes out; WEM in,
interleaved f32 PCM out. The profile data is **compiled into the wasm module**:
nothing is fetched, indexed, or passed in, and no entry touches the filesystem.
One-shot and streaming APIs mirror the C ABI (`include/wem.h`, ABI revision 3):
same lifecycle, same error codes, same bytes, selected by the structured
profile selection (one Wwise generation plus the PCM geometry) for encoding —
and, for decoding, no selection at all, because a WEM is self-describing.

## Layout

```
js/
  package.json     name: wwise-wem-wasm (type: module)
  src/index.ts     the wrapper (typed entry; erasable-TS only, no build step)
  pkg/             wasm-pack --target web      (browser/worker; .wasm fetched by URL)   — built
  pkg-node/        wasm-pack --target nodejs   (Node; .wasm read from disk on import)   — built
  test-node.mjs    Node parity test (reference bytes + chunking consistency + selection/error
                   codes + the decode direction against the committed reference WEM)
```

`pkg/` and `pkg-node/` are `wasm-pack --out-dir` output and are not committed
(`.gitignore`): nothing in the checkout is a wasm module, so build before
testing or serving.

Rebuilds (need wasm-pack + a wasm32 toolchain):

```sh
make wasm-build      # repo root: builds both packages (js/pkg, js/pkg-node)
node js/test-node.mjs   # repo root: the Node parity test, over a built js/pkg-node

cd js
npm run build:web    # → pkg/
npm run build:node   # → pkg-node/
npm test             # → node test-node.mjs (needs a built pkg-node)
```

> **Build-tool note (wasm-pack 0.15+):** `--out-dir` is resolved relative to
> the crate (`crates/wem-wasm`), not the current directory, so a relative
> `--out-dir pkg` would write `crates/wem-wasm/pkg` and leave whatever is in
> `js/pkg` stale. The npm scripts therefore pass an absolute out-dir
> (`--out-dir "$(pwd)/pkg"`); the equivalent from `crates/` is:
>
> ```sh
> wasm-pack build wem-wasm --target web    --release --out-dir "$(pwd)/../js/pkg"
> wasm-pack build wem-wasm --target nodejs --release --out-dir "$(pwd)/../js/pkg-node"
> ```
>
> The built packages must track the kernel source: the reference-bytes check in
> `test-node.mjs` fails against stale kernel bytes, and its 2ch/48k positive
> check fails against a kernel that still refuses the 2ch configuration. The
> test builds nothing itself: with no package present it stops and prints the
> build command rather than reporting a comparison it did not run.

## API (sketch)

```ts
import {
  initWasm, listVersions, parseWav,
  encodeWav, createEncoder, createStreamSession,
  WEM_ERROR_CODES,
} from "wwise-wem-wasm";

await initWasm();                       // or initWasm(url | response | arrayBuffer)
await listVersions();                   // [{ code: 0, label: "2013", generation: "2013.2" }]

// one-shot: WAV bytes → WEM. No selection → auto-selected from the WAV geometry.
const wav = /* ArrayBuffer | Uint8Array */;
const out = await encodeWav(wav);       // { data, stats }

// …or name the selection explicitly ({ version?, channels, sampleRate });
// version: code (0 = Wwise 2013), label ("2013"), generation ("2013.2"), or omit to auto-select
const twoChannel = await encodeWav(wav, { version: 0, channels: 2, sampleRate: 48000 });

// a reusable one-shot handle: selection() + encodeWav() + encodePcm16Interleaved()
const encoder = await createEncoder({ channels: 6, sampleRate: 44100 });
encoder.selection();                    // { versionCode, version, generation, channels, sampleRate, description }
encoder.destroy();

// streaming: Init → push* → Finish (frame-aligned chunks; boundaries never
// affect the output bytes). A parsed WAV is itself a geometry source.
const parsed = await parseWav(wav);
const session = await createStreamSession(parsed);   // auto-selected from parsed
for (const chunk of chunksOf(parsed.pcm)) {
  session.push(chunk, (packets) => { /* seq 0 = setup packet, then audio */ });
}
const result = session.finish();        // { data, stats }
session.destroy();
```

Decoding is the same shape with the data reversed (`include/wem.h` section 5) —
`WemDecoder`, reached through `createDecoder()` (streaming) or `decodeWem()`
(one-shot). There is no selection argument on this side:

```ts
import { initWasm, createDecoder, decodeWem } from "wwise-wem-wasm";

await initWasm();

// one-shot: WEM bytes → { channels, sampleRate, setup, frames, pcm }
const decoded = await decodeWem(wem);
decoded.pcm;                            // Float32Array, interleaved, ±1.0 full scale

// streaming: Init → push* → Finish; a step carries the header announcement
// (at most once, before any PCM), this step's samples, and a refusal if one
// stopped it — the C ABI's pcm_cb-then-code order.
const decoder = await createDecoder();
for (const chunk of chunksOf(wem)) {
  const step = decoder.push(chunk);      // { header, channels, frames, pcm, error }
}
const last = decoder.finish();           // terminal whatever it returns
decoder.destroy();
```

Errors: kernel failures throw a JS `Error` with a stable `code` —
`WEM_ERR_PROFILE_NOT_FOUND` (no compiled configuration satisfies the
selection), `WEM_ERR_STATE_ERROR` (non-positive geometry, malformed
argument, a call outside a handle's lifecycle), `WEM_ERR_GEOMETRY_MISMATCH`,
`WEM_ERR_INPUT_TOO_SHORT`, `WEM_ERR_FORMAT_UNSUPPORTED` (unknown version code,
non-PCM WAV, or a WEM whose setup packet this build does not carry),
`WEM_ERR_INTERNAL` (1:1 with `include/wem.h`, append-only).

On the decode side one refusal class is *returned on the step* instead of
thrown — `WEM_ERR_INPUT_MALFORMED` (bytes that are not a container this
revision parses, a container truncated before its declared frame count, an
audio packet that does not parse), together with the frames that step
completed. `WEM_ERR_INTERNAL` is the exception: it is a defect, its output is
not delivered, and it throws. `WEM_ERROR_CODES` lists the table in ABI order,
`WEM_OK` first and `WEM_ERR_INPUT_MALFORMED` last.

## Environments

- **Node ≥ 22.18**: imports `pkg-node` automatically (native type stripping
  runs `src/index.ts` directly).
- **Browser / Web Worker**: imports `pkg` (web target). Pass the wasm URL /
  `Response` / bytes to `initWasm()` when the module is not served next to
  its binary. No DOM dependencies.
- Bundlers: `pkg` works with Vite et al.; a `--target bundler` build is also
  available (`wasm-pack build ../crates/wem-wasm --target bundler --release`).

## Verification

`test-node.mjs` pins byte-exactness: the representative 6ch/44.1kHz recording
encoded through the wasm package must be byte-identical to the committed
kernel reference `tests/fixtures/reference.wem` — the test compares that
file's bytes, so it restates neither a digest nor a length of it —
across one-shot (auto-selected, explicit, and raw-PCM paths) and three
chunking schemes; the compiled-in version table, the resolved selection of
every constructor, and the selection/error code mapping must hold.

The same file checks the **decode** direction at run time, against the
committed paired-build container and the committed WAV it was produced from:
the wrapper's `decodeWem` and `createDecoder` must report 6ch/44100 and
139 398 frames, deliver a 201-byte setup packet, reconstruct the source within
the source's own peak (`max|error| ≤ 2 × peak`, the structural bound the Rust
suite uses — the numbers are printed by the run, never pinned), announce the
header exactly once before any PCM, deliver identical samples across three
chunkings and for empty chunks, and map the refusals
(`WEM_ERR_INPUT_MALFORMED` for foreign/truncated bytes and
`WEM_ERR_FORMAT_UNSUPPORTED` for a container this build does not carry, returned
on the step; `WEM_ERR_STATE_ERROR` for a call outside the lifecycle, thrown).
This is the check the browser/Node path did not have while the shell could only
be compiled: the wasm module and the wrapper now run a real decode here.

`make wasm-build` builds both packages and `node js/test-node.mjs` then runs the
test; CI's `web` job runs those two steps and publishes the built packages as a
distribution artifact
(`.github/workflows/web.yml`).
