# wwise-wem-wasm

Browser encoding for the WEM encoder kernel — the wasm-bindgen shell
(`crates/wem-wasm`) plus a typed JS wrapper. WAV/PCM in, WEM bytes out. The
profile data is **compiled into the wasm module**: nothing is fetched,
indexed, or passed in, and no entry touches the filesystem. One-shot and
streaming APIs mirror the C ABI (`include/wem.h`, ABI revision 2):
same lifecycle, same error codes, same bytes, selected by the structured
profile selection (one Wwise generation plus the PCM geometry).

## Layout

```
js/
  package.json     name: wwise-wem-wasm (type: module)
  src/index.ts     the wrapper (typed entry; erasable-TS only, no build step)
  pkg/             wasm-pack --target web      (browser/worker; .wasm fetched by URL)
  pkg-node/        wasm-pack --target nodejs   (Node; .wasm read from disk on import)
  test-node.mjs    Node parity test (reference bytes + chunking consistency + selection/error codes)
```

Rebuilds (need wasm-pack + a wasm32 toolchain):

```sh
npm run build:web    # → pkg/
npm run build:node   # → pkg-node/
npm test             # → node test-node.mjs (requires pkg-node)
```

> **Build-tool note (wasm-pack 0.15+):** `--out-dir` is resolved relative to
> the crate (`crates/wem-wasm`), not the current directory, so a relative
> `--out-dir pkg` would write `crates/wem-wasm/pkg` and leave the committed
> artifacts stale. The npm scripts therefore pass an absolute out-dir
> (`--out-dir "$(pwd)/pkg"`); the equivalent from `crates/` is:
>
> ```sh
> wasm-pack build wem-wasm --target web    --release --out-dir "$(pwd)/../js/pkg"
> wasm-pack build wem-wasm --target nodejs --release --out-dir "$(pwd)/../js/pkg-node"
> ```
>
> The committed `pkg/` and `pkg-node/` artifacts are refreshed that way and
> must track the kernel source: the reference-sha check in `test-node.mjs` fails
> against stale kernel bytes, and its 2ch/48k positive check fails against a
> kernel that still refuses the 2ch configuration.

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
const out = await encodeWav(wav);       // { data, totalLen, sha256Hex, stats }

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
const result = session.finish();        // { data, totalLen, sha256Hex, stats }
session.destroy();
```

Errors: kernel failures throw a JS `Error` with a stable `code` —
`WEM_ERR_PROFILE_NOT_FOUND` (no compiled configuration satisfies the
selection), `WEM_ERR_STATE_ERROR` (non-positive geometry, malformed
argument), `WEM_ERR_GEOMETRY_MISMATCH`, `WEM_ERR_INPUT_TOO_SHORT`,
`WEM_ERR_FORMAT_UNSUPPORTED` (unknown version code, non-PCM WAV),
`WEM_ERR_INTERNAL` (1:1 with `include/wem.h`, append-only).

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
kernel reference `tests/fixtures/reference.wem` (SHA-256
`17851d26c6210b85e498ae0452d2562d7b9e2c3e9e795c459656b9c9d8d35247` — the
test compares that file's bytes, so this digest documents the expected value
rather than being restated by the test),
across one-shot (auto-selected, explicit, and raw-PCM paths) and three
chunking schemes; the compiled-in version table, the resolved selection of
every constructor, and the selection/error code mapping must hold. Wired into
CI (`.github/workflows/web.yml`).