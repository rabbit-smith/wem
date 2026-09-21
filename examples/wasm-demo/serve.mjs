#!/usr/bin/env node
/**
 * Zero-dependency static server for the WEM browser demo.
 *
 *   node examples/wasm-demo/serve.mjs [port]
 *
 * Serves the repo root on http://localhost:8090 (default port 8090),
 * with correct MIME types (including application/wasm). Then open:
 *
 *   http://localhost:8090/examples/wasm-demo/
 *
 * The repo root is the root because the demo reaches into
 * ../../js/pkg (the wasm build) via a relative URL. The profile bundle
 * travels inside that wasm module — nothing else is served to the page.
 */

import { createServer } from "node:http";
import { createReadStream, existsSync, statSync } from "node:fs";
import { dirname, extname, join, normalize, resolve, sep } from "node:path";
import { fileURLToPath } from "node:url";

const here = dirname(fileURLToPath(import.meta.url)); // examples/wasm-demo
const repoRoot = resolve(here, "..", "..");
const port = Number(process.argv[2]) || 8090;

const MIME = new Map(
  [
    [".html", "text/html; charset=utf-8"],
    [".mjs", "text/javascript; charset=utf-8"],
    [".js", "text/javascript; charset=utf-8"],
    [".wasm", "application/wasm"],
    [".json", "application/json; charset=utf-8"],
    [".css", "text/css; charset=utf-8"],
    [".svg", "image/svg+xml"],
    [".ico", "image/x-icon"],
    [".wav", "audio/wav"],
    [".txt", "text/plain; charset=utf-8"],
    [".md", "text/plain; charset=utf-8"],
  ].map(([k, v]) => [k, v]),
);

const server = createServer((req, res) => {
  const urlPath = (req.url ?? "/").split("?")[0];
  let filePath;
  try {
    const candidate = normalize(join(repoRoot, urlPath));
    // path traversal guard: never resolve above the repo root
    if (!candidate.startsWith(repoRoot + sep) && candidate !== repoRoot) {
      res.writeHead(403);
      return res.end("forbidden");
    }
    filePath = candidate;
  } catch {
    res.writeHead(400);
    return res.end("bad request");
  }

  let target = filePath;
  if (existsSync(target) && statSync(target).isDirectory()) {
    target = join(target, "index.html");
  }
  if (!existsSync(target) || !statSync(target).isFile()) {
    res.writeHead(404);
    return res.end("not found");
  }

  res.writeHead(200, {
    "Content-Type": MIME.get(extname(target).toLowerCase()) ?? "application/octet-stream",
    "Cache-Control": "no-cache",
  });
  createReadStream(target).pipe(res);
});

server.listen(port, () => {
  console.log(`WEM demo server: http://localhost:${port}/examples/wasm-demo/`);
  console.log(`serving repo root: ${repoRoot}`);
});
