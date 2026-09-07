/**
 * wwise-wem-wasm — typed wrapper over the wem-wasm kernel shell.
 *
 * The Rust side (crates/wem-wasm) is a parallel language shell over the
 * WEM encoder kernel (include/wem.h contract): one-shot and streaming
 * PCM -> WEM, profile data as bytes, errors as `WEM_ERR_*` codes. This
 * module adds a small ergonomic, promise-shaped API on top and NOTHING
 * else: no numerics, no profile logic, no WAV parsing of its own — the
 * kernel owns all of that.
 *
 * # Environments (all first-class, no DOM used here)
 *
 * - **Node**: imports `../pkg-node/wem_wasm.js` (wasm-pack `nodejs`
 *   target; the .wasm loads from disk on import).
 * - **Browser / web worker**: imports `../pkg/wem_wasm.js` (wasm-pack
 *   `web` target; the .wasm is fetched by URL — pass `url`/`Response`
 *   to {@link initWasm} when the module is not served next to its
 *   binary).
 *
 * # Notes for TS/Node consumers
 *
 * This file is the runtime entry: modern Node (>= 22.18) runs it with
 * native type stripping; bundlers (Vite et al.) consume it directly.
 * Only erasable TypeScript syntax is used so no build step is needed.
 */

// ---------------------------------------------------------------------------
// Core (wasm) surface — structural types over the generated bindings
// ---------------------------------------------------------------------------

interface CoreEncoder {
  encode_pcm16_interleaved(pcm: Uint8Array): WemResultRaw;
  profile_info(): ProfileInfoRaw;
  free(): void;
}

interface CoreSession {
  push(pcm: Uint8Array): Uint8Array[];
  finish(): WemResultRaw;
  pcm_frames(): number;
  free(): void;
}

interface CoreModule {
  WemEncoder: new (profileIndex: Uint8Array, files: Map<string, Uint8Array>) => CoreEncoder;
  WemSession: new (
    profileIndex: Uint8Array,
    files: Map<string, Uint8Array>,
    setupSha256: string,
    profileName: string,
  ) => CoreSession;
  wem_parse_wav(wav: Uint8Array): ParsedWavRaw;
  // web target only: async init (nodejs target self-initializes on import)
  default?: (input?: unknown) => Promise<unknown>;
}

interface ProfileInfoRaw {
  name: string;
  setupSha256: string;
  channels: number;
  sampleRate: number;
}

interface ParsedWavRaw {
  sampleRate: number;
  channels: number;
  frames: number;
  pcm: Uint8Array;
}

interface WemResultRaw {
  data: Uint8Array;
  totalLen: number;
  sha256Hex: string;
  stats: WemStats;
}

// ---------------------------------------------------------------------------
// Public types
// ---------------------------------------------------------------------------

/** Terminal container summary (mirrors the kernel's WemMeta + stats). */
export interface WemResult {
  /** The assembled WEM container bytes. */
  data: Uint8Array;
  /** Byte length of `data`. */
  totalLen: number;
  /** SHA-256 of `data` (lowercase hex, 64 chars). */
  sha256Hex: string;
  stats: WemStats;
}

/** One encode's statistics (kernel EncodeStats, camelCase). */
export interface WemStats {
  pcmFrames: number;
  channels: number;
  audioPackets: number;
  shortPackets: number;
  longPackets: number;
  bytes: number;
  metadataSource: string;
}

/** A signed-16 PCM WAV parsed by the kernel. */
export interface ParsedWav {
  sampleRate: number;
  channels: number;
  frames: number;
  /** Interleaved little-endian signed-16 PCM bytes. */
  pcm: Uint8Array;
}

/**
 * One loaded profile bundle, verified by the kernel on load
 * (every logical resource's SHA-256 is checked by the `*_bytes` entries).
 */
export interface ProfileBundle {
  /** The raw index.json bytes (kept for the streaming entry). */
  readonly indexBytes: Uint8Array;
  /** Profiles-dir-relative path -> bytes (Map form the kernel expects). */
  readonly files: Map<string, Uint8Array>;
  /** The kernel-verified identity of the selected (default) profile. */
  readonly info: {
    name: string;
    setupSha256: string;
    channels: number;
    sampleRate: number;
  };
}

/**
 * One streaming encode session (Init -> push* -> Finish -> destroy).
 *
 * Single-threaded ownership (the C ABI contract). All methods are
 * synchronous: the encode work itself is synchronous CPU work, so a
 * Promise wrapper would only obscure error handling. Feed chunks with
 * lengths that are multiples of `channels * 2` bytes (one complete
 * frame each).
 */
export interface StreamSession {
  /**
   * Push one interleaved s16-LE PCM chunk.
   *
   * @param pcmChunk chunk bytes (must align to complete frames)
   * @param onPackets optional callback for the packets that just
   *   completed (seq 0 = setup packet, then audio packets in order)
   * @returns the same packets (reply order)
   */
  push(pcmChunk: Uint8Array | ArrayBuffer, onPackets?: (packets: Uint8Array[]) => void): Uint8Array[];
  /**
   * Finish the stream: complete the encode and return the container.
   * Terminal — the session must be destroyed afterwards.
   */
  finish(): WemResult;
  /** PCM frames accumulated so far (progress observability). */
  pcmFrames(): number;
  /** Release the session (safe before or after finish). */
  destroy(): void;
}

/** Kernel error shape: a JS Error with the stable WEM_ERR_* code. */
export interface WemError extends Error {
  /** One of WEM_ERR_PROFILE_NOT_FOUND / WEM_ERR_STATE_ERROR / ... */
  code: string;
}

export const WEM_ERROR_CODES = [
  "WEM_OK",
  "WEM_ERR_PROFILE_NOT_FOUND",
  "WEM_ERR_STATE_ERROR",
  "WEM_ERR_GEOMETRY_MISMATCH",
  "WEM_ERR_INPUT_TOO_SHORT",
  "WEM_ERR_FORMAT_UNSUPPORTED",
  "WEM_ERR_INTERNAL",
] as const;

// ---------------------------------------------------------------------------
// Core loading (environment-aware)
// ---------------------------------------------------------------------------

let corePromise: Promise<CoreModule> | null = null;
let initInput: unknown;

function isNodeEnvironment(): boolean {
  return typeof process !== "undefined" && typeof process.versions?.node === "string";
}

/**
 * Initialize the wasm core.
 *
 * - Node: nothing to do (the nodejs build self-initializes on import);
 *   this call still resolves the module.
 * - Browser/worker: passes the load input to the web build's init:
 *   `undefined` (fetch relative to the module URL — the default),
 *   a URL string, a `Response`, or an `ArrayBuffer` of the .wasm.
 */
export async function initWasm(input?: string | Response | ArrayBuffer): Promise<void> {
  if (corePromise) return;
  initInput = input;
  corePromise = (async () => {
    const module = isNodeEnvironment()
      ? await import("../pkg-node/wem_wasm.js")
      : await import("../pkg/wem_wasm.js");
    const init = (module as { default?: (i?: unknown) => Promise<unknown> }).default;
    if (typeof init === "function") {
      // web target: instantiate from URL/Response/bytes (node target: absent)
      await init(initInput);
    }
    return module as unknown as CoreModule;
  })();
}

async function core(): Promise<CoreModule> {
  if (!corePromise) await initWasm();
  return corePromise!;
}

// ---------------------------------------------------------------------------
// Profile bundles
// ---------------------------------------------------------------------------

/**
 * Accept the profile files as a Map or a plain object; normalize
 * ArrayBuffer values to Uint8Array (the kernel accepts both, the
 * wrapper keeps one canonical form).
 */
function normalizeFiles(
  files: Map<string, Uint8Array | ArrayBuffer> | Record<string, Uint8Array | ArrayBuffer>,
): Map<string, Uint8Array> {
  const out = new Map<string, Uint8Array>();
  const push = (key: string, value: Uint8Array | ArrayBuffer) => {
    if (value instanceof Uint8Array) {
      out.set(key, value);
    } else if (typeof ArrayBuffer !== "undefined" && value instanceof ArrayBuffer) {
      out.set(key, new Uint8Array(value));
    } else {
      throw WemErrorLike("WEM_ERR_STATE_ERROR: profile file bytes must be Uint8Array or ArrayBuffer");
    }
  };
  if (files instanceof Map) {
    for (const [key, value] of files) push(key, value);
  } else if (files && typeof files === "object") {
    for (const key of Object.keys(files)) push(key, (files as Record<string, Uint8Array | ArrayBuffer>)[key]);
  } else {
    throw new WemErrorLike("WEM_ERR_STATE_ERROR: profile files must be a Map or object");
  }
  return out;
}

function WemErrorLike(codeMessage: string): Error & { code: string } {
  const code = codeMessage.split(":")[0];
  const err = new Error(codeMessage);
  (err as { code: string }).code = code;
  return err;
}

/**
 * Load (and kernel-verify) one profile bundle from bytes.
 *
 * The kernel's bytes entry re-checks every resource's SHA-256 on load
 * and selects the index `default` profile; a failing bundle throws
 * `WEM_ERR_PROFILE_NOT_FOUND` / `WEM_ERR_STATE_ERROR` / `WEM_ERR_INTERNAL`.
 * The bundle carries a resolved encoder handle for one-shot encodes.
 */
export async function loadProfileBundle(
  indexBytes: Uint8Array | ArrayBuffer,
  files: Map<string, Uint8Array | ArrayBuffer> | Record<string, Uint8Array | ArrayBuffer>,
): Promise<ProfileBundle & { readonly encoder: CoreEncoder }> {
  const coreModule = await core();
  const index =
    indexBytes instanceof Uint8Array ? indexBytes : new Uint8Array(indexBytes);
  const filesMap = normalizeFiles(files);
  // Throws (with a WEM_ERR_* code) when the bundle fails verification.
  const encoder = new coreModule.WemEncoder(index, filesMap);
  const info = encoder.profile_info();
  return {
    indexBytes: index,
    files: filesMap,
    info,
    encoder,
  };
}

// ---------------------------------------------------------------------------
// Encoding
// ---------------------------------------------------------------------------

/** Parse one signed-16 PCM WAV (kernel parser; throws on non-conforming files). */
export async function parseWav(wavBytes: Uint8Array | ArrayBuffer): Promise<ParsedWav> {
  const coreModule = await core();
  const wav =
    wavBytes instanceof Uint8Array ? wavBytes : new Uint8Array(wavBytes);
  const parsed = coreModule.wem_parse_wav(wav);
  return {
    sampleRate: parsed.sampleRate,
    channels: parsed.channels,
    frames: parsed.frames,
    pcm: parsed.pcm,
  };
}

/**
 * One-shot encode: WAV bytes -> complete WEM container.
 *
 * The WAV is parsed by the kernel (`parse_pcm16`); PCM geometry must
 * match the profile (6ch/44100 for the bundled profile), otherwise
 * `WEM_ERR_GEOMETRY_MISMATCH` / `WEM_ERR_INPUT_TOO_SHORT` is thrown.
 */
export async function encodeWav(
  wavBytes: Uint8Array | ArrayBuffer,
  bundle: ProfileBundle,
): Promise<WemResult> {
  const parsed = await parseWav(wavBytes);
  const resultRaw = bundle.encoder.encode_pcm16_interleaved(parsed.pcm);
  return {
    data: resultRaw.data,
    totalLen: resultRaw.totalLen,
    sha256Hex: resultRaw.sha256Hex,
    stats: resultRaw.stats,
  };
}

/**
 * Open one streaming encode session on a loaded bundle.
 *
 * Drives the kernel's bytes-entry `StreamSession::for_profile_ref_bytes`
 * (Init): the reference asserts the bundle's setup SHA-256 (hard) and
 * profile name (soft) — the same verification runs again here, which is
 * the kernel doing its job, not duplicated logic.
 */
export async function createStreamSession(bundle: ProfileBundle): Promise<StreamSession> {
  const coreModule = await core();
  const session = new coreModule.WemSession(
    bundle.indexBytes,
    bundle.files,
    bundle.info.setupSha256,
    bundle.info.name,
  );
  return {
    push(pcmChunk, onPackets) {
      const bytes =
        pcmChunk instanceof Uint8Array ? pcmChunk : new Uint8Array(pcmChunk);
      const packets = session.push(bytes);
      if (onPackets) onPackets(packets);
      return packets;
    },
    finish() {
      const resultRaw = session.finish();
      return {
        data: resultRaw.data,
        totalLen: resultRaw.totalLen,
        sha256Hex: resultRaw.sha256Hex,
        stats: resultRaw.stats,
      };
    },
    pcmFrames() {
      return session.pcm_frames();
    },
    destroy() {
      session.free();
    },
  };
}
