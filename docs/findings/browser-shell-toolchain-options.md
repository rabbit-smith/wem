# Browser/Node shell toolchain options

Date: 2026-09-22. This page is the evidence record for one question: **for this
repository's browser and Node target, what are the realistic options for the
binding/shell layer and the toolchain around it — MoonBit, C, Zig, the component
model, or combinations — and what does each cost against what the repository does
today?** It is a survey of options, three of which were *not* taken, so it is
written as a decision record rather than as reference material: the reference
documents describe how the system *is*, and this page describes what was
considered and what settled it. The one thing it is not is a plan; nothing here
has been adopted.

## The constraint this page holds fixed

The repository's integration topology is not part of the question
([`../reference/standards.md`](../reference/standards.md#integration-topology)):

> The Rust kernel is the sole integration point. … Every language binding is a
> parallel shell over the kernel … never a parallel implementation. Shells
> mirror that interface 1:1, map errors 1:1 without inventing variants, and own
> no numerics and no profile logic; a new language integrates by writing a shim
> over the C ABI, never by changing the kernel for it.

So the search space is exactly three things, and nothing else: a **binding/shell
technology**, a **build or link toolchain**, or an **alternative way to produce
the shipped wasm module around the same kernel**. One candidate fails that test
on its face and is dropped here in one line: *re-implementing the encoder in C,
Zig or MoonBit is out of bounds* — a second encoder would be a parallel
implementation, which the topology forbids, and would put bit-exactness in two
places at once.

## 1. What the repository does today (measurement, not recollection)

| Quantity | Value | How established |
|---|---:|---|
| Committed wasm module | 1,651,013 B | `js/pkg/wem_wasm_bg.wasm` and `js/pkg-node/wem_wasm_bg.wasm` are byte-identical copies |
| … of which the `data` section | 1,171,534 B (**71.0 %**) | section walk of the committed binary |
| … of which the `code` section | 476,588 B (**28.9 %**) | same |
| … `import` / `export` / all other sections | 762 B / 571 B / <1 kB | same |
| Imports | 14, all from `./wem_wasm_bg.js` (`__wbg_*`, `__wbindgen_generic_*`, `__wbindgen_init_externref_table`) | import section |
| Exports | 26: `memory`, the `__wbindgen_externrefs` table, two globals (`__abort_handler`, `__instance_terminated`), 15 shell functions (`wem_versions`, `wem_parse_wav`, the `wemencoder_*` / `wemsession_*` entries and their `__wbg_*_free` helpers) and 7 `wasm-bindgen` runtime functions (`__wbindgen_malloc`, `__wbindgen_realloc`, `__wbindgen_free`, `__wbindgen_exn_store`, `__externref_table_alloc`, `__externref_table_dealloc`, `__wbindgen_start`) | export section |
| Globals | 3: a mutable `i32` initialised to 1,048,576 (0x100000 — the stack pointer), and two `const` `i32`s at 0x2f7818 / 0x2f7840 (heap/data-end) | global section |
| Data segments | 3,863 | data section |
| Linear memory | 48 pages (3,145,728 B), no declared maximum | memory section |
| Generated glue, web target | 18,751 B (`js/pkg/wem_wasm.js`) + 7,285 B typings | file sizes |
| Generated glue, Node target | 15,516 B (`js/pkg-node/wem_wasm.js`) | file size |
| Hand-written facade | 17,227 B ([`../../js/src/index.ts`](../../js/src/index.ts), 508 lines) | file size |
| `WebAssembly.compile` of the committed module | **1.0 ms** | `node v22.23.2`, macOS arm64 |
| Import of the glue + instantiation | **4.5 ms** | same |
| `parseWav` of the 6ch/44.1 kHz fixture | 0.9 ms (1,672,776 B PCM, 139,398 frames) | same |
| `encodeWav` of that fixture | **225.9 ms ≈ 14.0× realtime**, SHA-256 `17851d26…d35247` | same; matches [`../reference/standards.md`](../reference/standards.md#what-is-established) |
| PCM copy into wasm memory (`TypedArray.set` of 1,672,776 B) | 0.19 / 0.036 / 0.029 ms over three runs | same |

Two readings follow directly from that table and they decide most of what
follows.

**The binding layer is not what makes the module big.** 71 % of the shipped
bytes are the `data` section — the profile tables the kernel carries compiled in
([`profile-as-code.md`](profile-as-code.md)) — and the whole wasm-bindgen *interface*
(imports + exports + type + function + table + element + memory + global + custom
sections) is under 3 kB. No shell technology can move the number that matters.

**The binding layer is not the hot path either.** Instantiating the module costs
~5.5 ms against a 226 ms encode, and moving a 1.67 MB PCM buffer across the
boundary costs ~0.03–0.19 ms — about 0.1 % of the encode. The encoder is 14×
realtime in Node already.

## 2. The options, and what each actually is

### 2.1 Rust + wasm-bindgen (status quo)

`crates/wem-wasm` is a `cdylib` over `wem-core` with `default-features = false`
(threadless scalar path), compiled by wasm-pack to two targets: `--target web`
(`js/pkg`) and `--target nodejs` (`js/pkg-node`), both committed
([`../../crates/wem-wasm/Cargo.toml`](../../crates/wem-wasm/Cargo.toml),
[`../../.github/workflows/web.yml`](../../.github/workflows/web.yml)). The
`#[wasm_bindgen]` surface maps the C ABI's lifecycle and error table 1:1 and owns
no numerics ([`../../crates/wem-wasm/src/lib.rs`](../../crates/wem-wasm/src/lib.rs)).
`wasm-bindgen` turns `&[u8]` parameters into `malloc` + `TypedArray.set` and
returned `Vec<u8>` into `Uint8Array` views over wasm memory — visible in the
committed glue at `passArray8ToWasm0` / `getArrayU8FromWasm0`
(`js/pkg/wem_wasm.js`).

The toolchain's governance changed in 2025 while the project was running:
the Rust and WebAssembly Working Group was archived and the
[rustwasm org was sunset](https://blog.rust-lang.org/inside-rust/2025/07/21/sunsetting-the-rustwasm-github-org/),
with `wasm-bindgen` transferred to a new
[wasm-bindgen organisation](https://github.com/wasm-bindgen/wasm-bindgen) "with
new additional maintainers" and "all other repositories … archived". The
maintained home of `wasm-pack` is now
[`wasm-bindgen/wasm-pack`](https://github.com/wasm-bindgen/wasm-pack), whose own
book still labels itself "the **unpublished** documentation"
([wasm-pack book](https://wasm-bindgen.github.io/wasm-pack/book/index.html)).
`wasm-bindgen` 0.2.x is still released (its README records MSRV entries for
0.2.118 on 2026-04-10 and 0.2.106 on 2025-11-27, and the workspace locks
0.2.128 — [`../../crates/Cargo.lock`](../../crates/Cargo.lock)), so this is a
change of steward, not of health.

### 2.2 Rust without wasm-bindgen: a hand-written C-ABI-style export surface + thin TS

Nothing in the topology requires wasm-bindgen; it requires that a shell *mirror*
`include/wem.h` and own no numerics. The same crate can export
`#[unsafe(no_mangle)] pub extern "C" fn …` entry points — integer/pointer-only
signatures, a scalar export of the allocator for input buffers, `(ptr, len)` or an
out-param struct for results — and the file `js/src/index.ts` already defines the
facade over exactly such a structural interface (`CoreEncoder`, `CoreSession`,
`CoreModule` interfaces over the generated bindings). In other words the shell's
*JS half* is already hand-written; only the generated half would change, at a cost
of about 200–400 lines of loader and marshalling code replacing 16–19 kB of
generated glue. The `wasm32-unknown-unknown` target is a
[Tier 2 Rust target](https://doc.rust-lang.org/nightly/rustc/platform-support/wasm32-unknown-unknown.html)
that "does not import any functions from the host for the standard library", so
the export surface is the only interface the module needs.

### 2.3 A C shim in the same module (clang or `zig cc`, linked by wasm-ld)

`lld`'s WebAssembly port is `wasm-ld`, and its inputs are WebAssembly **objects**
in the LLVM wasm object format, not finished modules
([lld WebAssembly port](https://lld.llvm.org/WebAssembly.html): "The WebAssembly
object file format used by LLVM and LLD is specified as part of the WebAssembly
tool conventions on linking. This is the object format that the llvm will produce
when run with the `wasm32-unknown-unknown` target"); the linking convention makes
the distinction explicit — "In order to distinguish object files from executable
WebAssembly modules the linker can check for the presence of the
['linking'](#linking-metadata-section) custom section which must exist in all
object files" ([tool-conventions `Linking.md`](https://github.com/WebAssembly/tool-conventions/blob/main/Linking.md)).
`--import-memory` ("Import memory from the environment") and `--allow-undefined`
/ `--import-undefined` ("Generate WebAssembly imports for undefined symbols")
are the two flags a shim needs, and both are documented in the lld port's usage
section. So a C shim *is* a real route to a single module that exports the
literal `include/wem.h` surface — provided the Rust side can be produced as a
`staticlib` wasm archive to link against (the Rust Reference documents
`staticlib` as "a static library containing all of the local crate's code along
with all upstream dependencies … recommended for use in situations such as
linking Rust code into an existing non-Rust application", but its platform list
names only Linux/macOS/Windows — see §9).

This was measured here, not assumed: Apple clang 21 accepts
`--target=wasm32-unknown-unknown -nostdlib`, `rust-lld -flavor wasm` links the
resulting object into a working 471-byte module, and that module was wired in
Node against the *committed* kernel instance (§3).

### 2.4 C, Zig or MoonBit as a second wasm module beside the kernel

The other shape is two modules in one JS realm: the kernel module stays as it is,
and a shell module — C, Zig or MoonBit — imports the kernel's exports and calls
them. This is legal and stable at the JS API level (§3), needs no dynamic linker
and no `dylink` section, and is exactly what the MoonBit and Zig probes below
produce. It costs a second module to download, instantiate and keep in step, and
it puts two independently linked runtimes into one linear memory — measured to
mean two stacks at the same address unless the layouts are coordinated by hand
(§4, §9.4).

* **Zig.** `zig build-exe -target wasm32-freestanding -fno-entry -rdynamic
  --import-memory` produced a 169-byte module in this environment (Zig 0.16.0)
  whose import section is byte-for-byte the same shape as the clang-built C
  module's: `env.memory` plus `env.__wbindgen_malloc`. Its documented wasm
  targets are `wasm32-freestanding`, `wasm32-wasi` and `wasm32-emscripten` — Zig
  does not accept clang's `wasm32-unknown-unknown` spelling, and `wasm32-wasip1`
  is not a Zig target name
  ([language reference, WebAssembly](https://ziglang.org/documentation/master/)).
  Its default wasm linker is lld's `wasm-ld`: the link line recorded here for a
  freestanding module was `wasm-ld … --stack-first --no-entry -z
  stack-size=1048576`, and a self-hosted wasm linker is reachable with
  `-fno-lld`, which linked the same module. Zig is pre-1.0 and moving:
  [ziglang.org/download](https://ziglang.org/download/) lists 0.16.0 (2026-04-13),
  0.15.2 (2025-10-11), 0.14.1 (2025-05-21) and a 0.17.0-dev line dated 2026-09-20
  — roughly two to three releases a year, no 1.0.
* **MoonBit.** Its own documentation says it is "an end-to-end programming
  language toolchain … across `wasm`, `wasm-gc`, `js`, and `native` backends"
  ([MoonBit docs](https://docs.moonbitlang.com/en/latest/)), and the FFI page
  documents naming an import's module and field for the wasm target
  (`fn cos(d : Double) -> Double = "math" "cos"`,
  [FFI](https://docs.moonbitlang.com/en/latest/language/ffi.html)), plus
  `import-memory` / `export-memory-name` / `heap-start-address` link options
  ([moon package config](https://docs.moonbitlang.com/en/latest/toolchain/moon/package.html)).
  Its wasm output is self-contained with its own reference counting ("MoonBit
  uses reference counting for Wasm backend and C backend") and the docs' own
  advice is to avoid `println`/`env` because they import a MoonBit runtime host
  function, with `wasm-merge` offered as the workaround
  ([WebAssembly integration](https://docs.moonbitlang.com/en/latest/toolchain/wasm/index.html)).
  Maturity: "MoonBit is currently in beta-preview"
  ([language index](https://docs.moonbitlang.com/en/latest/language/index.html)),
  monthly 0.10.x releases through
  [2026-09-21](https://www.moonbitlang.com/updates/2026/09/21/index), and a
  roadmap that had 1.0 planned for
  [the first half of 2026](https://www.moonbitlang.com/blog/roadmap).
* **Combinations.** Binaryen's
  [`wasm-merge`](https://github.com/WebAssembly/binaryen/blob/main/src/tools/wasm-merge.cpp)
  "does at compile time what you can do with JS at runtime: connect some wasm
  modules together by hooking up imports to exports", resolving them by the
  module names given on its command line and fusing them into one module — it is
  a bundler, not a linker ("Unlike wasm-ld, this does not have the full semantics
  of native linkers"), and it merges the code, it does not reconcile two
  allocators or two stack layouts.

### 2.5 The component model as the binding technology (wit-bindgen, jco)

The Component Model is a W3C WebAssembly **Community Group** work item at
**Phase 1 — Feature Proposal (CG)** in the
[proposals repository](https://github.com/WebAssembly/proposals/blob/main/README.md),
developed "incrementally as part of [WASI] 'Developer Preview' releases"
(0.2.0, 0.3.0, 0.3.1) with the formal `spec/` directory still a placeholder
([component-model](https://github.com/WebAssembly/component-model/blob/main/README.md)).
WIT is the interface definition language and the canonical ABI is "the process
for reading component-level values into and out of linear memory"
([CanonicalABI.md](https://github.com/WebAssembly/component-model/blob/main/design/mvp/CanonicalABI.md)).
For a Rust guest, `wit-bindgen` generates `#[unsafe(no_mangle)]` exports with
canonical names and the module is turned into a component by
`wasm-tools component new` ([wit-bindgen](https://github.com/bytecodealliance/wit-bindgen)).
In JS, `jco` "'Transpile[s]' WebAssembly components into ES modules that can run
in environments like NodeJS and the browser" and ships `preview2-shim` for
"NodeJS and Browsers" ([jco](https://github.com/bytecodealliance/jco)); `jco`
describes itself as "an experimental project. **No guarantees** are provided for
stability, security or support and breaking changes may be made without notice".

### 2.6 Emscripten, wasi-sdk, and plain `wasm32-unknown-unknown`

* **Emscripten** is a main-module + side-module dynamic-linking system: "There
  are two types of shared modules: 1. **Main modules**, which have system
  libraries linked in. 2. **Side modules**, which do not have system libraries
  linked in … only the singleton main module includes the JavaScript environment
  and side modules are pure WebAssembly modules", built with `-sMAIN_MODULE` and
  `-shared`/`-sSIDE_MODULE`
  ([Dynamic Linking](https://emscripten.org/docs/compiling/Dynamic-Linking.html)).
  Its default output is not self-contained — "the .wasm file is not standalone -
  it's not easy to manually run it without that .js code"
  ([Building to WebAssembly](https://emscripten.org/docs/compiling/WebAssembly.html)) —
  and the "standalone Wasm" mode (`-sSTANDALONE_WASM`, enabled by emitting
  `<name>.wasm`) targets WASI.
* **wasi-sdk** "contains no compiler or library code itself" — it is a
  preconfigured clang + wasm-ld + wasi-libc sysroot; upstream clang "can compile
  for WASI out of the box … all that's done here is to provide builds configured
  to set the default target and sysroot for convenience"; the documented triple
  is `--target=wasm32-wasip1`, and wasi-sdk-34 removed `wasm32-wasi`
  ([wasi-sdk](https://github.com/WebAssembly/wasi-sdk),
  [release notes](https://github.com/WebAssembly/wasi-sdk/releases/tag/wasi-sdk-34)).
  Browsers do not provide WASI; the official WASI site instead points at runtimes
  and says of `jco` that it is for "running in JS environments and browsers"
  ([wasi.dev](https://wasi.dev/)), i.e. a shim layer, not a host API.
* **Plain `wasm32-unknown-unknown`** is supported by clang and lld (above), has
  no sysroot of its own, and is the target the committed module is built for.

## 3. The crux: can each option consume the *existing* module's exports?

This is the question that decides whether C, Zig or MoonBit can be a shell over
the kernel we already ship, rather than a reason to rebuild it.

**Established (standards).** The JavaScript API defines instantiation as a
lookup of `importObject[moduleName][componentName]`, and — decisively for module
to module wiring — when the value found for a function import is itself an
exported function, it is linked **by its function address** rather than wrapped
as a new host function: "If v has a \[\[FunctionAddress\]\] internal slot, and
therefore is an Exported Function … Let funcaddr be the value of v's
\[\[FunctionAddress\]\] internal slot"; for a memory import, "If v does not
implement `Memory`, throw a `LinkError`" and the instance's memory object is
taken directly ([WebAssembly JS API](https://webassembly.github.io/spec/js-api/)).
The core specification adds that "Instantiation checks that the module is valid
and the provided imports match the declared types"
([core spec, execution](https://webassembly.github.io/spec/core/exec/modules.html)),
and each import "is labeled by a two-level name space, consisting of a module
name and an item name"
([core spec, modules](https://webassembly.github.io/spec/core/syntax/modules.html)).
Growth is also specified: `Memory.grow` refreshes the buffer and, for a
fixed-length buffer, performs `DetachArrayBuffer`
([JS API](https://webassembly.github.io/spec/js-api/)) — which is why the
committed glue re-creates its cached views.

**Measured, in this environment.** A 471-byte C module (Apple clang 21 →
`wasm32-unknown-unknown` object → `rust-lld -flavor wasm --no-entry
--allow-undefined --import-memory`) declaring exactly
`env.memory` and `env.__wbindgen_malloc` was instantiated in Node with
`{ env: { memory: kernel.memory, __wbindgen_malloc: kernel.__wbindgen_malloc } }`
taken from an instance of the **committed** module. It called the kernel's own
allocator, wrote 16 bytes into the kernel's linear memory at the returned
pointer (0x300008 — the call grew the memory from 48 to 65 pages, and a cached
pre-growth view was observed `detached === true` while a fresh view worked), and
the kernel then encoded `tests/fixtures/input.wav` to the reference SHA-256
`17851d26…d35247`. So: **calling exports, passing buffers and coping with memory
growth all work, for C and by the same mechanism for any language that emits a
core wasm module with plain imports.**

| Option | Can it consume the shipped module's exports? | How | What it cannot do |
|---|---|---|---|
| C (`clang`/`zig cc` + `wasm-ld`) | **Yes**, measured | Declare `import_module`/`import_name` (or `--allow-undefined`), `--import-memory`, then hand it the kernel instance's `memory` and functions from JS | It cannot be *linked into* the shipped module — only into a freshly linked one; function-pointer callbacks across the boundary need a shared table |
| Zig | **Yes**, same mechanism (import section measured identical to the C module's) | `extern` declarations become imports (module `env` unless `@extern`'s `library_name` says otherwise); `-rdynamic` to keep exports; `--import-memory` | **No**: Zig 0.16.0 refuses a final `.wasm` as an input — `error: unrecognized file extension of parameter '…wem_wasm_bg.wasm'` — and the rule underneath it is lld's: a wasm input is an object only if it carries the `linking` custom section (`WasmObjectFile::isRelocatableObject() { return HasLinkingSection; }`), otherwise [`fatal(… ": not a relocatable wasm file")`](https://github.com/llvm/llvm-project/blob/main/lld/wasm/InputFiles.cpp). It accepts and links wasm *objects* and archives (a clang-made `shim.o` linked into a 465-byte module) |
| MoonBit | **Documented in principle, not demonstrated** — the FFI names an import's module+field, and `import-memory` exists | A MoonBit wasm module importing the kernel's functions and memory, wired in JS or fused with `wasm-merge` | No documented mechanism for consuming a pre-existing `.wasm`; its own `moon.pkg.json` schema exposes no module-linking key; its docs describe imports as coming "from the runtime host" |
| Component model (`wit-bindgen` + `jco`) | **No** | — | `wasm-tools component new` requires WIT metadata already embedded in the core module (`component-type*` custom sections) and exports named per the component-model scheme: an arbitrary module fails with "input is a core wasm module with no `component-type*` custom sections meaning that there is not WIT information". Adopting it means recompiling the same kernel through `wit-bindgen` against a WIT world |
| Emscripten main/side modules | **No** | — | Emscripten's model is a main module plus Emscripten-built side modules; a module built by rustc/wasm-ld is neither. The ABI it relies on is described by its own spec document (`DynamicLinking.md`) as "still a work in progress. There is no stable ABI yet" — and lld's page agrees: "Dynamic linking support is still experimental. The spec for this is not yet finalized." Note the honest gap: the Emscripten documentation never states the prohibition in words (§9) |

## 4. The axes a decision turns on

Facts in this table are sourced above or measured in §1/§3; the "verdict" column
is judgement, marked as such.

| Axis | Established fact | Judgement |
|---|---|---|
| Module size | 1,651,013 B, **71.0 % data**, 28.9 % code; ≤3 kB of it is the binding interface | No shell technology moves this. Only removing profile data from the module would, and that is excluded by [`../reference/standards.md`](../reference/standards.md#profile-data-ownership) |
| Glue size | 18,751 B (web) / 15,516 B (Node) generated + 17,227 B hand-written facade | Replacing the generated glue by hand is a ~20 kB-sized change, not a size strategy |
| Instantiation | compile 1.0 ms, glue import + instantiate 4.5 ms; module has no declared memory maximum, 48 initial pages | Nothing to optimize; a second module adds a second compile/instantiate of the same order |
| Hot path | Every `&[u8]` in is copied (`passArray8ToWasm0`), results come back as views; the copy of the 1.67 MB fixture PCM costs 0.03–0.19 ms against a 225.9 ms encode | Interop friction is ~0.1 % of the work. Sharing one memory would remove the copy and buy nothing measurable |
| Cross-module ABI | `DynamicLinking.md`: "There is no stable ABI yet"; lld: "still experimental … not yet finalized"; JS-API-level wiring *is* stable and was measured to work | Second-module designs are buildable but rest on an unfinished ABI if they need the dynamic linker; with hand wiring they instead rest on hand-coordinated memory layout |
| One memory, two runtimes | Measured across three modules: the kernel's one mutable global is a stack pointer at 0x100000 (1 MiB); a Zig-built module's is **also** 0x100000, because Zig's default link line is `--stack-first -z stack-size=1048576`; a clang-built module's is 64 KiB (lld's smaller default). The kernel's data begins at 1 MiB and its heap at 0x2f7818 | Sharing one memory aliases two runtimes' stacks **by construction**: the kernel's stack grows down from 1 MiB into exactly the region the Zig module reserved for its own stack. Nothing in the JS API, in `wasm-ld` or in either language's defaults coordinates the two layouts |
| Toolchain maturity | `wasm-bindgen` 0.2.x current, steward changed in 2025; Zig 0.16.0 / 0.17.0-dev, no 1.0; MoonBit beta-preview 0.10.x, monthly, 1.0 slipped; component model Phase 1 CG; `jco` self-declared experimental; `wasm-tools compose` deprecated in favour of `wac` | Every alternative here is *less* settled than the status quo, and none of them is settled in a way this project needs |
| CI cost | One `web` job: rustup + wasm-pack action + Node 22 + two `wasm-pack build`s + `node js/test-node.mjs` ([`../../.github/workflows/web.yml`](../../.github/workflows/web.yml)) | A C shim adds clang/wasi-sdk/zig to that job; a second module adds a second build and a second committed artifact; the current job is one toolchain |
| C-ABI/versioned surface | `include/wem.h` is revision 2, append-only error codes, callback-based output (`WemWriteCb`, `WemPacketCb`) | A same-module C shim *could* export the literal header, but the header's callback design means either real function pointers inside one module or a re-shaped surface; `crates/wem-capi` itself refuses `panic = "abort"` (`#[cfg(panic = "abort")] compile_error!`), which is what wasm-pack builds by default, so "the C ABI compiled to wasm" is a nightly-toolchain decision, not a shim decision |
| Panic semantics | `wasm-pack`: "By default, Rust panics in WebAssembly compile with `panic=abort`, which aborts the WebAssembly instance"; wasm-bindgen's catch-unwind path needs nightly, `-Zbuild-std=std,panic_unwind`, `-Cpanic=unwind` and wasm exception handling, and Node ≥ 22.22.3 ([catch-unwind](https://wasm-bindgen.github.io/wasm-bindgen/reference/catch-unwind.html)) | The wasm shell's panic divergence from the C ABI is real and documented in [`../../crates/wem-wasm/src/lib.rs`](../../crates/wem-wasm/src/lib.rs) — and it is orthogonal to this survey: no shell language fixes it; the catch-unwind stack is the only documented route and is its own decision |
| What must be maintained | Today: one Rust crate, two committed build outputs, one facade, one parity test | Each alternative adds a language, a toolchain, or a second artifact, and all of them still owe the same `js/test-node.mjs` parity checks |

## 5. What each option would change in this repository

| Option | Files and workflows touched | Kernel touched? |
|---|---|---|
| Status quo | nothing | no |
| Rust without wasm-bindgen | [`../../crates/wem-wasm/src/lib.rs`](../../crates/wem-wasm/src/lib.rs) (attributes → `extern "C"` + `#[unsafe(no_mangle)]`, a small `(ptr,len)`/status protocol, an exported allocator), [`../../crates/wem-wasm/Cargo.toml`](../../crates/wem-wasm/Cargo.toml) (drop `wasm-bindgen`/`js-sys`), the workspace dependency block in [`../../crates/Cargo.toml`](../../crates/Cargo.toml) and [`../../crates/Cargo.lock`](../../crates/Cargo.lock), both committed outputs `js/pkg/*` and `js/pkg-node/*` (now hand-written loaders), [`../../js/src/index.ts`](../../js/src/index.ts) (loader/init), [`../../js/test-node.mjs`](../../js/test-node.mjs) (import path/init), [`../../.github/workflows/web.yml`](../../.github/workflows/web.yml) (wasm-pack → `cargo build --target wasm32-unknown-unknown` + a copy step), [`../../js/README.md`](../../js/README.md), [`../../examples/wasm-demo/demo.mjs`](../../examples/wasm-demo/demo.mjs) (init call), [`../guides/usage.md`](../guides/usage.md) (the "wasm-bindgen shell" row) | no |
| C shim in the same module | everything above, plus a new shim source + a link step (`clang`/`zig cc` object, `rust-lld` link), a `staticlib`-capable build of the kernel crates, `include/wem.h` gaining wasm-specific notes or a wasm section, and a decision about the panic contract | no, but `wem-core`/`wem-capi` build configuration changes |
| Second module (C/Zig/MoonBit) | a second source tree + a second build in `web.yml`, a second committed artifact in `js/`, loader changes in `js/src/index.ts` and `js/test-node.mjs`, demo changes, and a memory-layout contract documented somewhere that neither module can enforce | no |
| Component model | replace `crates/wem-wasm` with a `wit-bindgen` guest crate + a WIT world, build for `wasm32-wasip2`, add `jco`/`wasm-tools` to the toolchain and to `web.yml`, replace `js/pkg*` with transpiled output, rewrite the facade's loader, keep `js/test-node.mjs`'s checks (`../../js/test-node.mjs`), and re-document [`../reference/standards.md`](../reference/standards.md#integration-topology) | no, but the shell is rewritten rather than edited |
| `include/wem.h` | unchanged by every option above except the C-shim row, where it becomes a wasm-facing surface too | — |

## 6. The "not worth it" list, with the fact that settles each

* **Re-implement the encoder in C, Zig or MoonBit.** Out of bounds by
  [`../reference/standards.md`](../reference/standards.md#integration-topology);
  dropped.
* **Emscripten as the toolchain for the shipped module.** Its dynamic-linking
  model is a main module plus Emscripten side modules, its default `.wasm` "is
  not standalone", and the ABI behind it has "no stable ABI yet". Also, the page
  documenting it says of itself: "This documentation is somewhat outdated and is
  in the process of being refreshed."
* **wasi-sdk / WASI as a browser target.** No browser provides WASI; the
  documented browser bridge is a JS shim (`jco`'s `preview2-shim`), and wasi-sdk
  builds for `wasm32-wasip1`, a host-interface target this module does not use.
* **A C shim just to "get rid of wasm-bindgen".** It does not remove a
  toolchain — it *adds* one (clang or Zig) while keeping rustc, and it adds a
  second language to a shell whose entire job is already 508 lines of TS plus
  18 kB of generated glue. The interface it would expose is ≤3 kB of the module.
* **Zig as the shell language or the toolchain.** Pre-1.0 with two-to-three
  releases a year; measured to reject a final `.wasm` as an input (and lld's own
  `not a relocatable wasm file` check is why), so it can only ever be a producer
  or a second module — a second module whose default `--stack-first -z
  stack-size=1048576` reservation lands on the kernel's own stack region; as a C
  compiler (`zig cc`) it is a preference, not an architectural choice.
* **MoonBit as the shell language.** beta-preview, monthly 0.10.x, its own RC
  runtime and host-import requirements (`spectest.print_char`, `moonbit:ffi`),
  no emitted JS glue for the wasm target, and no documented path to bind against
  an existing module's exports — a second module, a second toolchain and a second
  artifact for no measured gain.
* **The component model as a drop-in binding layer.** `wasm-tools component new`
  rejects a core module without embedded WIT metadata and canonical export names;
  `jco` is component → JS only and self-declared experimental; `wasm-tools
  compose` is deprecated in favour of `wac`; and no browser ships a
  `WebAssembly.Component` API, which is why the project's own goal text says to
  "polyfill … via Ahead-of-Time compilation" first.
* **Two modules fused with `wasm-merge`.** Binaryen's tool is "sort of like a
  wasm bundler": it fuses imports to exports by CLI-assigned module names and
  leaves two allocators, two stack layouts and the sum of both sizes in place.
  It is a way to *make* the second-module shape work, not a reason to adopt it.

## 7. Recommendation

**Judgement, from the facts above: keep the Rust + wasm-bindgen shell as the
browser/Node binding technology, and do not introduce C, Zig or MoonBit into
this lane.** The module's size is 71 % compiled-in profile data and its measured
binding costs (1.0 ms compile, 4.5 ms instantiate, ~0.1 % of an encode for the
PCM copy) are not what a shell technology can improve, so every alternative on
the table trades a maintained single-toolchain shell for a second language, a
second toolchain or a second module in exchange for nothing measurable — and the
second-module shapes additionally rest on either an ABI their own specification
calls unfinished or on a memory layout that two independently linked runtimes
cannot see or enforce. If the wasm lane is to be simplified at all, the only
alternative worth a spike is **the same Rust kernel without wasm-bindgen** —
hand-written `extern "C"` exports plus the TS loader that already exists in
shape — because it is the one option that removes a dependency without adding a
language, and its blast radius is the file list in §5.

Three specific things follow, each small and independent:

1. **Do not re-open the binding technology for size or speed.** The measured
   numbers are in §1; any future proposal should have to name a quantity from
   that table that it improves.
2. **If the wasm lane's dependency on `wasm-bindgen`/`wasm-pack` is the actual
   concern** (it moved organisations once already), the response is the
   no-wasm-bindgen spike, not a language change. It is bounded, and it is
   testable against `js/test-node.mjs` unchanged.
3. **Treat the panic divergence as its own item.** `crates/wem-wasm/src/lib.rs`
   documents that a panic in the wasm shell is a trap, not `WEM_ERR_INTERNAL`; the
   documented route to closing that gap is wasm-bindgen's catch-unwind stack
   (nightly + `-Zbuild-std` + wasm exception handling). No shell technology
   considered here changes it, and mixing it into this decision would hide a
   real, separate trade-off.

## 8. What would have to be true for a different option to win

**Rust without wasm-bindgen** wins if any of these becomes true: `wasm-bindgen`
or `wasm-pack` stalls (its governance changed once, in 2025); the glue/crate
version pairing blocks a Rust or dependency upgrade; or the generated glue's
`externref`/table machinery proves awkward for a new entry point the shell needs.
What must hold: the hand-written exports reproduce every check in
[`../../js/test-node.mjs`](../../js/test-node.mjs) — reference bytes, all three
chunkings, the selection table and the seven error codes — with the same or
smaller module, and the kernel is not touched. What would settle it is the
measurement in §9.1.

**A C shim in the same module** wins if the repository decides the wasm artifact
must expose the *literal* `include/wem.h` surface — for instance so the C ABI
conformance suite can run against the wasm build, or so a wasm consumer can be
handed the same header as the native ones. What must hold: the kernel crates
produce a wasm `staticlib` that links with a clang/`zig cc` shim under `wasm-ld`;
the callback model (`WemWriteCb`/`WemPacketCb`) is either implemented with real
function pointers inside the single module or replaced by a documented
wasm-shaped equivalent; and the panic contract is either explicitly reduced for
wasm or paired with the nightly unwind stack — because
`crates/wem-capi/src/lib.rs` cannot compile under `panic = "abort"`, which is
what wasm-pack builds by default.

**The component model** wins if the encoder must be embedded somewhere that
speaks components rather than JS — a WASI 0.3 host, a non-JS runtime, or a
platform that consumes WIT interfaces — or if browsers ship native component
support. What must hold: the same kernel rebuilt through `wit-bindgen` on
`wasm32-wasip2` against a WIT world, `jco` transpilation accepted in the JS lane,
and a new size/latency baseline established (unmeasured today, §9.5).

**A second module in C, Zig or MoonBit** wins only if the shell itself becomes
non-trivial — needing numerics, streaming state or buffer juggling that TS
cannot express affordably — which the integration topology forbids, or if the
team standardises on one of those languages elsewhere and values one language for
all shells above a second artifact. Even then it is a second module, not a
replacement glue layer.

## 9. Unknown, and what a spike would have to measure

Honest gaps, in the order they would matter to a decision:

1. **How much of the 476,588-byte code section is wasm-bindgen?** Not established
   by any primary source and not measurable from the committed artifact alone.
   *Spike:* build `crates/wem-wasm` with and without `wasm-bindgen` (wasm-pack and
   `cargo build --target wasm32-unknown-unknown` respectively, same profile) and
   diff module size + section sizes. This is the number that decides whether the
   no-wasm-bindgen option is a size win, a wash, or a loss (the hand-written
   loader's `d.ts` and marshalling code are not free either).
2. **Does the kernel link as a wasm `staticlib` with a C shim?** The Rust
   Reference documents `staticlib` but names only Linux/macOS/Windows, and
   nothing states wasm `staticlib` support per target. *Spike:* add
   `crate-type = ["staticlib"]` to a throwaway copy of `wem-wasm`/`wem-capi`,
   `cargo build --target wasm32-unknown-unknown`, and link a clang shim against
   the archive with `rust-lld -flavor wasm`; measure whether the result exports
   the header's entries and reproduces the reference bytes.
3. **Cross-module callbacks through a shared table.** My probe exercised a
   function call and memory sharing, not `WemWriteCb`/`WemPacketCb` as a real
   callback: that needs `--export-table`/`--import-table`, a matching signature in
   both modules and a hand-kept table layout. *Spike:* measure whether the C ABI's
   callback shape survives the boundary, and at what complexity.
4. **The two-runtime memory layout.** Measured: the Rust kernel's stack pointer
   and a Zig-built module's are the same address (1 MiB, stack placed first), a
   clang-built module's is 64 KiB, and the kernel's heap begins at 0x2f7818. Two
   modules in one memory therefore alias their stacks by construction, and no
   document or linker coordinates them. *Spike:* if a second module is ever
   seriously considered, define the partition explicitly — `--global-base`, `-z
   stack-size`, dropping `--stack-first`, `--initial-memory`, or an allocator
   export from the kernel — and prove it with a test that writes to the top of
   each region.
5. **Component-model overhead.** No primary source quantifies the canonical
   ABI's lifting/lowering cost or a component wrapper's size against a
   wasm-bindgen baseline; the spec itself notes lifting/lowering "should be able
   to fuse … into a single direct copy" while requiring `memory`/`realloc` for
   any list or string. *Spike:* a `wit-bindgen` build of the same kernel with a
   `list<u8>` in / `list<u8>` out world, measured the same way as §1.
6. **Instantiation outside Node/V8.** The 1.0 ms compile / 4.5 ms instantiate
   figures are from `node v22.23.2` on macOS arm64 and reflect V8's lazy
   compilation, not a full baseline compile; browser and other-engine figures are
   unmeasured here.
7. **`zig cc` has no official page to cite.** Zig's language reference documents
   `zig translate-c`, not `zig cc`, and its FAQ returned 404 when checked; what is
   established here is behavioural — `zig cc -target wasm32-freestanding -O2 -c`
   produced a working wasm object in this environment — plus the toolchain list in
   the 0.16.0 release notes. Anyone weighing `zig cc` against clang should treat
   that as an evidence gap rather than a documented equivalence.
8. **MoonBit binding an existing module's exports.** Documented only as "from
   the runtime host"; whether an import can be satisfied by another module
   instance's export in the same realm is not stated anywhere in MoonBit's docs.
   *Spike:* one MoonBit wasm build with `import-memory` + a named import, wired
   against the kernel in Node.

## Sources

Primary sources, by owner. Repository paths are relative; everything else is the
document that owns the claim.

* Repository: [`../reference/standards.md`](../reference/standards.md)
  (integration topology, portability floor, panics, profile ownership),
  [`profile-as-code.md`](profile-as-code.md) (profile data as compiled tables),
  [`../../include/wem.h`](../../include/wem.h),
  [`../../crates/wem-wasm/src/lib.rs`](../../crates/wem-wasm/src/lib.rs),
  [`../../crates/wem-capi/src/lib.rs`](../../crates/wem-capi/src/lib.rs),
  [`../../crates/Cargo.toml`](../../crates/Cargo.toml),
  [`../../crates/Cargo.lock`](../../crates/Cargo.lock),
  [`../../js/src/index.ts`](../../js/src/index.ts),
  [`../../js/test-node.mjs`](../../js/test-node.mjs),
  [`../../js/README.md`](../../js/README.md),
  [`../../.github/workflows/web.yml`](../../.github/workflows/web.yml),
  [`../../examples/wasm-demo/README.md`](../../examples/wasm-demo/README.md),
  [`../guides/usage.md`](../guides/usage.md).
* LLVM/lld: [WebAssembly lld port](https://lld.llvm.org/WebAssembly.html),
  [lld's wasm input check](https://github.com/llvm/llvm-project/blob/main/lld/wasm/InputFiles.cpp),
  [lld's wasm option table](https://github.com/llvm/llvm-project/blob/main/lld/wasm/Options.td).
* WebAssembly tool conventions:
  [Linking](https://github.com/WebAssembly/tool-conventions/blob/main/Linking.md),
  [Dynamic Linking](https://github.com/WebAssembly/tool-conventions/blob/main/DynamicLinking.md).
* WebAssembly specifications:
  [JS API](https://webassembly.github.io/spec/js-api/),
  [core syntax/modules](https://webassembly.github.io/spec/core/syntax/modules.html),
  [core execution](https://webassembly.github.io/spec/core/exec/modules.html).
* Rust: [platform support](https://doc.rust-lang.org/nightly/rustc/platform-support.html),
  [wasm32-unknown-unknown](https://doc.rust-lang.org/nightly/rustc/platform-support/wasm32-unknown-unknown.html),
  [linkage / crate types](https://doc.rust-lang.org/reference/linkage.html).
* Rust project: [Sunsetting the rustwasm GitHub org](https://blog.rust-lang.org/inside-rust/2025/07/21/sunsetting-the-rustwasm-github-org/).
* `wasm-bindgen`: [repository](https://github.com/wasm-bindgen/wasm-bindgen),
  [deployment targets](https://wasm-bindgen.github.io/wasm-bindgen/reference/deployment.html),
  [catching panics](https://wasm-bindgen.github.io/wasm-bindgen/reference/catch-unwind.html).
* `wasm-pack`: [repository](https://github.com/wasm-bindgen/wasm-pack),
  [build command](https://wasm-bindgen.github.io/wasm-pack/book/commands/build.html).
* Emscripten: [Dynamic Linking](https://emscripten.org/docs/compiling/Dynamic-Linking.html),
  [Building to WebAssembly](https://emscripten.org/docs/compiling/WebAssembly.html),
  [settings reference](https://emscripten.org/docs/tools_reference/settings_reference.html).
* WASI: [wasi-sdk](https://github.com/WebAssembly/wasi-sdk),
  [wasi-sdk-34 release notes](https://github.com/WebAssembly/wasi-sdk/releases/tag/wasi-sdk-34),
  [wasi.dev](https://wasi.dev/).
* Component model:
  [component-model](https://github.com/WebAssembly/component-model/blob/main/README.md),
  [proposals (phase list)](https://github.com/WebAssembly/proposals/blob/main/README.md),
  [CanonicalABI](https://github.com/WebAssembly/component-model/blob/main/design/mvp/CanonicalABI.md),
  [Explainer](https://github.com/WebAssembly/component-model/blob/main/design/mvp/Explainer.md),
  [wasm-tools](https://github.com/bytecodealliance/wasm-tools),
  [wit-bindgen](https://github.com/bytecodealliance/wit-bindgen),
  [jco](https://github.com/bytecodealliance/jco),
  [wac](https://github.com/bytecodealliance/wac).
* Binaryen: [`wasm-merge`](https://github.com/WebAssembly/binaryen/blob/main/src/tools/wasm-merge.cpp).
* Zig: [downloads and release dates](https://ziglang.org/download/),
  [language reference (WebAssembly targets)](https://ziglang.org/documentation/master/).
* MoonBit: [documentation index](https://docs.moonbitlang.com/en/latest/),
  [language index](https://docs.moonbitlang.com/en/latest/language/index.html),
  [FFI](https://docs.moonbitlang.com/en/latest/language/ffi.html),
  [WebAssembly integration](https://docs.moonbitlang.com/en/latest/toolchain/wasm/index.html),
  [package configuration](https://docs.moonbitlang.com/en/latest/toolchain/moon/package.html),
  [updates](https://www.moonbitlang.com/updates/),
  [roadmap](https://www.moonbitlang.com/blog/roadmap).