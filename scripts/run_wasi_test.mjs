// Cargo runner for the container crate's tests on a real 32-bit usize target.
import { readFile } from "node:fs/promises";
import { WASI } from "node:wasi";

const [binary, ...args] = process.argv.slice(2);
if (!binary) throw new Error("usage: node scripts/run_wasi_test.mjs TEST.wasm [ARGS...]");
const wasi = new WASI({ version: "preview1", args: [binary, ...args], returnOnExit: true });
const module = await WebAssembly.compile(await readFile(binary));
const instance = await WebAssembly.instantiate(module, { wasi_snapshot_preview1: wasi.wasiImport });
process.exitCode = wasi.start(instance);
