# wwise-wem-wasm

Browser encoding for the WEM encoder kernel — the wasm-bindgen shell
(`crates/wem-wasm`) plus a typed JS wrapper. PCM in, WEM bytes out, profile
data as bytes; SHA-256 verification of every profile resource happens in the
kernel on load. One-shot and streaming APIs mirror the C ABI contract
(`include/wem.h`): same lifecycle, same error codes, same bytes.

## Layout

```
js/
  package.json     name: wwise-wem-wasm (type: module)
  src/index.ts     the wrapper (typed entry; erasable-TS only, no build step)
  pkg/             wasm-pack --target web      (browser/worker; .wasm fetched by URL)
  pkg-node/        wasm-pack --target nodejs   (Node; .wasm read from disk on import)
  test-node.mjs    Node parity gate (golden sha256 + chunking consistency + error codes)
```

Rebuilds (need wasm-pack + a wasm32 toolchain):

```sh
npm run build:web    # → pkg/
npm run build:node   # → pkg-node/
npm test             # → node test-node.mjs (requires pkg-node)
```

> **Build-tool note (wasm-pack 0.15+):** `--out-dir` is resolved relative to
> the crate (`crates/wem-wasm`), not the current directory, so the npm scripts
> above may no longer update `pkg/` and `pkg-node/` in place.  If the
> `test-node.mjs` gate behaves as if it ran against stale kernel bytes, rebuild
> with an absolute out-dir, e.g. from `crates/`:
>
> ```sh
> wasm-pack build wem-wasm --target web    --release --out-dir "$(pwd)/../js/pkg"
> wasm-pack build wem-wasm --target nodejs --release --out-dir "$(pwd)/../js/pkg-node"
> ```
>
> The committed `pkg/` and `pkg-node/` artifacts are refreshed that way and
> must track the kernel source: the golden-sha gate in `test-node.mjs` fails
> against stale kernel bytes, and its 2ch/48k positive gate fails against a
> kernel that still refuses the 2ch profile.

## API (sketch)

```ts
import {
  initWasm, loadProfileBundle, parseWav,
  encodeWav, createStreamSession,
  WEM_ERROR_CODES,
} from "wwise-wem-wasm";

// bytes travel from your loader (fetch / import / IndexedDB cache)
const indexBytes = /* index.json bytes */;
const files = new Map([ /* profiles-dir-relative path → bytes */ ]);

await initWasm();                              // or initWasm(url | response | arrayBuffer)
const bundle = await loadProfileBundle("wwise2013-2ch-48000", indexBytes, files);
// bundle.info → { name, setupSha256, channels, sampleRate }

// one-shot: WAV bytes → WEM
const wav = /* ArrayBuffer | Uint8Array */;
const out = await encodeWav(wav, bundle);      // { data, totalLen, sha256Hex, stats }

// streaming: Init → push* → Finish (frame-aligned chunks; boundaries never
// affect the output bytes)
const session = await createStreamSession(bundle);
for (const chunk of chunksOfPcm(pcmBytes)) {
  session.push(chunk, (packets) => { /* seq 0 = setup packet, then audio */ });
}
const result = session.finish();               // { data, totalLen, sha256Hex, stats }
session.destroy();
```

Errors: kernel failures throw a JS `Error` with a stable `code` —
`WEM_ERR_PROFILE_NOT_FOUND`, `WEM_ERR_STATE_ERROR`,
`WEM_ERR_GEOMETRY_MISMATCH`, `WEM_ERR_INPUT_TOO_SHORT`,
`WEM_ERR_FORMAT_UNSUPPORTED`, `WEM_ERR_INTERNAL` (1:1 with `include/wem.h`,
append-only).

## Environments

- **Node ≥ 22.18**: imports `pkg-node` automatically (native type stripping
  runs `src/index.ts` directly).
- **Browser / Web Worker**: imports `pkg` (web target). Pass the wasm URL /
  `Response` / bytes to `initWasm()` when the module is not served next to
  its binary. No DOM dependencies.
- Bundlers: `pkg` works with Vite et al.; a `--target bundler` build is also
  available (`wasm-pack build ../crates/wem-wasm --target bundler --release`).

## Verification

`test-node.mjs` pins byte-exactness: the fixture PCM encoded through the
wasm package must match the kernel golden
(`17851d26c6210b85e498ae0452d2562d7b9e2c3e9e795c459656b9c9d8d35247`),
across one-shot and three chunking schemes, and kernel error codes must
surface as `WEM_ERR_*` JS errors. Wired into CI (`.github/workflows/web.yml`).
