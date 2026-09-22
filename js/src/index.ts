/**
 * wwise-wem-wasm — typed wrapper over the wem-wasm kernel shell.
 *
 * The Rust side (crates/wem-wasm) is a parallel language shell over the
 * WEM encoder kernel (`include/wem.h`, ABI revision 3): one-shot
 * and streaming WAV/PCM -> WEM, plus the decode direction (`WemDecoder`,
 * section 5) WEM -> interleaved f32 PCM, errors as `WEM_ERR_*` codes. This
 * module adds a small ergonomic, promise-shaped API on top of both
 * directions and NOTHING else: no numerics, no profile logic, no WAV or WEM
 * parsing of its own — the kernel owns all of that.
 *
 * # Profile selection (ABI revision 3)
 *
 * The encoder configurations are compiled into the wasm module: nothing is
 * fetched, downloaded, indexed, or passed in. A selection is one Wwise
 * generation plus the PCM geometry — `{ version, channels, sampleRate }`
 * mirroring `WemProfile` in `include/wem.h`:
 *
 * - `version`: the stable code (`0` = Wwise 2013), the label `"2013"`, the
 *   generation `"2013.2"`, or omitted for **auto-selection**: the unique
 *   compiled generation whose configuration satisfies the geometry (in the
 *   browser flow, the geometry of the WAV the user picked).
 * - `channels` / `sampleRate`: the PCM geometry to encode (> 0).
 *
 * A selection no compiled configuration satisfies is rejected with
 * `WEM_ERR_PROFILE_NOT_FOUND`, an unknown version code with
 * `WEM_ERR_FORMAT_UNSUPPORTED`, and a non-positive geometry with
 * `WEM_ERR_STATE_ERROR` — never silently substituted by a default.
 *
 * ```ts
 * import { initWasm, parseWav, encodeWav, createStreamSession } from "wwise-wem-wasm";
 *
 * await initWasm();
 *
 * // one-shot, auto-selected from the WAV's own geometry
 * const out = await encodeWav(wavBytes);            // { data, stats }
 *
 * // one-shot, explicit selection
 * const same = await encodeWav(wavBytes, { version: 0, channels: 6, sampleRate: 44100 });
 *
 * // streaming (Init -> push* -> Finish); chunk boundaries never change the bytes
 * const wav = await parseWav(wavBytes);
 * const session = await createStreamSession(wav);   // auto-selected from wav
 * for (const chunk of chunksOf(wav.pcm)) session.push(chunk);
 * const result = session.finish();
 * session.destroy();
 * ```
 *
 * # Decoding (include/wem.h section 5)
 *
 * The mirror of the encode direction with the data reversed, and the same
 * shape: a WEM is self-describing, so there is no selection argument, and the
 * geometry arrives on the step that parses the container's setup packet.
 *
 * ```ts
 * import { initWasm, createDecoder, decodeWem } from "wwise-wem-wasm";
 *
 * await initWasm();
 *
 * // one-shot: WEM bytes -> the whole decode result
 * const decoded = await decodeWem(wemBytes);        // { channels, sampleRate, setup, frames, pcm }
 * decoded.pcm;                                      // Float32Array, interleaved, ±1.0 full scale
 *
 * // streaming (Init -> push* -> Finish); chunk boundaries never move a sample
 * const decoder = await createDecoder();
 * for (const chunk of chunksOf(wemBytes)) {
 *   const step = decoder.push(chunk);               // step.header, step.pcm, step.error
 * }
 * const last = decoder.finish();
 * decoder.destroy();
 * ```
 *
 * A refusal that is not a defect travels on the step beside the samples that
 * step completed — the order the C ABI delivers them in (`pcm_cb`, then the
 * return code) — so a caller that stops at `step.error` still sees every frame
 * the packets before the refusal produced. `WEM_ERR_INTERNAL` is the exception:
 * it is a defect, its output is not delivered, and it throws (a wasm trap
 * cannot be a value — see crates/wem-wasm's panic note).
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

/** The version argument the wasm constructors take. */
type CoreVersionArg = number | string | null | undefined;

interface CoreEncoder {
  encode_pcm16_interleaved(pcm: Uint8Array): WemResultRaw;
  encode_wav(wav: Uint8Array): WemResultRaw;
  selection(): SelectionInfoRaw;
  free(): void;
}

interface CoreSession {
  push(pcm: Uint8Array): Uint8Array[];
  finish(): WemResultRaw;
  pcm_frames(): number;
  selection(): SelectionInfoRaw;
  free(): void;
}

interface CoreDecoder {
  push(data: Uint8Array): DecodeStepRaw;
  finish(): DecodeStepRaw;
  free(): void;
}

interface CoreModule {
  WemEncoder: new (
    version: CoreVersionArg,
    channels: number,
    sampleRate: number,
  ) => CoreEncoder;
  WemSession: new (
    version: CoreVersionArg,
    channels: number,
    sampleRate: number,
  ) => CoreSession;
  /** The decode session: Init takes no selection (include/wem.h section 5). */
  WemDecoder: new () => CoreDecoder;
  wem_parse_wav(wav: Uint8Array): ParsedWavRaw;
  /** The compiled-in `WemVersion` table (include/wem.h). */
  wem_versions(): WwiseVersionInfo[];
  // web target only: async init (nodejs target self-initializes on import)
  default?: (input?: unknown) => Promise<unknown>;
}

interface SelectionInfoRaw {
  versionCode: number;
  version: string;
  generation: string;
  channels: number;
  sampleRate: number;
  description: string;
}

interface ParsedWavRaw {
  sampleRate: number;
  channels: number;
  frames: number;
  pcm: Uint8Array;
}

interface WemResultRaw {
  data: Uint8Array;
  stats: WemStats;
}

/**
 * One decode step as the wasm shell returns it. The shell's own shape: the
 * header announcement on the one step that resolved it, this step's samples,
 * and the refusal that stopped it (`null` when it consumed everything).
 */
interface DecodeStepRaw {
  header: DecodedHeaderRaw | null;
  channels: number;
  frames: number;
  pcm: Float32Array;
  error: DecodeRefusal | null;
}

interface DecodedHeaderRaw {
  channels: number;
  sampleRate: number;
  setup: Uint8Array;
}

// ---------------------------------------------------------------------------
// Public types
// ---------------------------------------------------------------------------

/**
 * One Wwise generation selector: the stable cross-language code
 * (`0` = Wwise 2013), its label (`"2013"`) or generation (`"2013.2"`).
 * `null` / `undefined` asks for auto-selection from the geometry.
 */
export type WwiseVersionSelector = number | string | null | undefined;

/** One selectable Wwise generation (a row of the compiled-in table). */
export interface WwiseVersionInfo {
  /** Stable cross-language code (`WemVersion` in include/wem.h). */
  code: number;
  /** Short label the generation is spelled with (`"2013"`). */
  label: string;
  /** Full generation string (`"2013.2"`). */
  generation: string;
}

/**
 * One structured profile selection (mirrors `WemProfile` in
 * `include/wem.h`): a Wwise generation plus the PCM geometry to encode.
 * Omit `version` to auto-select the compiled generation that satisfies the
 * geometry.
 */
export interface ProfileSelection {
  version?: WwiseVersionSelector;
  channels: number;
  sampleRate: number;
}

/** The selection a handle resolved to (and encodes). */
export interface ResolvedSelection {
  /** Stable cross-language version code (`0` = Wwise 2013). */
  versionCode: number;
  /** Short version label (`"2013"`). */
  version: string;
  /** Full generation (`"2013.2"`). */
  generation: string;
  /** PCM channel count being encoded. */
  channels: number;
  /** PCM sample rate being encoded. */
  sampleRate: number;
  /** Human description, e.g. `"2ch/48000Hz/2013"`. */
  description: string;
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
 * Where a geometry comes from: an explicit selection, or a WAV the kernel
 * already parsed (then the generation is auto-selected from the WAV's own
 * geometry).
 */
export type GeometrySource = ProfileSelection | ParsedWav;

/**
 * One encoded container (the kernel's `EncodeResult`): the WEM bytes plus
 * the statistics observed while assembling them.
 *
 * Nothing derived from those bytes travels beside them — a `Uint8Array`
 * knows its own `byteLength`, and a digest is the caller's to compute from
 * the bytes it received.
 */
export interface WemResult {
  /** The assembled WEM container bytes. */
  data: Uint8Array;
  stats: WemStats;
}

/**
 * One encode's statistics (kernel EncodeStats, camelCase): what the library
 * observed and the caller cannot recompose. `pcmFrames` because a streaming
 * caller may never have counted the frames it pushed; the packet counts
 * because they require parsing the assembled container.
 */
export interface WemStats {
  pcmFrames: number;
  channels: number;
  audioPackets: number;
  shortPackets: number;
  longPackets: number;
}

/**
 * One reusable one-shot encoder (Init once, encode many buffers). The
 * underlying wasm handle is shareable; release it with `destroy()`.
 */
export interface Encoder {
  /** The selection this handle resolved to. */
  selection(): ResolvedSelection;
  /**
   * Encode one interleaved s16-LE PCM buffer. The buffer carries no
   * geometry of its own: it is read at the selection's rate and channel
   * count, so it must match the selection (the C ABI rule for raw PCM).
   */
  encodePcm16Interleaved(pcm: Uint8Array | ArrayBuffer): WemResult;
  /**
   * Parse one signed-16 PCM WAV and encode it. The kernel cross-checks the
   * WAV's geometry against the selection
   * (`WEM_ERR_GEOMETRY_MISMATCH` when they disagree).
   */
  encodeWav(wavBytes: Uint8Array | ArrayBuffer): WemResult;
  /** Release the handle. */
  destroy(): void;
}

/**
 * One streaming encode session (Init -> push* -> Finish -> destroy).
 *
 * Single-threaded ownership (as the C ABI requires). All methods are
 * synchronous: the encode work itself is synchronous CPU work, so a
 * Promise wrapper would only obscure error handling. Feed chunks with
 * lengths that are multiples of `channels * 2` bytes (one complete
 * frame each).
 */
export interface StreamSession {
  /** The selection this session resolved to. */
  selection(): ResolvedSelection;
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

/**
 * The decoder's one-time header announcement (the mirror of the C ABI's
 * `header_cb`): what the container itself declares, resolved on the step that
 * parsed its setup packet.
 *
 * It carries no frame count: the wasm shell's step object does not announce
 * the container's declared `dw_total_pcm_frames`, so this surface does not
 * invent one. What a caller can always count is what it received — see
 * {@link DecodeStep.frames} and {@link DecodeResult.frames}.
 */
export interface DecodedHeader {
  /** PCM channel count the container declares. */
  channels: number;
  /** PCM sample rate the container declares. */
  sampleRate: number;
  /** The setup packet this build parsed, exactly as the container carried it. */
  setup: Uint8Array;
}

/** One refusal, as the stable `WEM_ERR_*` code plus the kernel's diagnostic. */
export interface DecodeRefusal {
  code: string;
  message: string;
}

/**
 * What one decode step produced, and how it ended (`wem_decoder_push` /
 * `wem_decoder_finish`).
 *
 * The step reports its output whether or not it was refused: a rejection stops
 * the step at the packet it could not read, and the samples the earlier packets
 * completed are still real, so they are handed over rather than dropped.
 * Nothing was advanced past the failure, and the same rejection is reported
 * again by the next call.
 */
export interface DecodeStep {
  /** The header announcement, on the one step that resolved it. */
  header: DecodedHeader | null;
  /** Channel count `pcm` is interleaved over (`0` before the header is known). */
  channels: number;
  /** Frames in `pcm` (`0` when the step produced none). */
  frames: number;
  /** This step's samples: interleaved f32 at ±1.0 full scale. */
  pcm: Float32Array;
  /** The refusal that stopped this step, or `null` when it consumed its input. */
  error: DecodeRefusal | null;
}

/**
 * One streaming decode session (Init -> push* -> Finish -> destroy): the
 * mirror of the encode session with the data direction reversed.
 *
 * There is no selection argument — a WEM is self-describing and the geometry
 * arrives on the step that parses its setup packet. Single-threaded ownership,
 * as the C ABI requires. All methods are synchronous: the decode work is
 * synchronous CPU work, so a Promise wrapper would only obscure error
 * handling. Chunk boundaries never affect the emitted samples, and an empty
 * chunk is a no-op.
 */
export interface Decoder {
  /**
   * Push one chunk of WEM bytes and return what it completed.
   *
   * @param wemChunk chunk bytes (any split; the samples do not depend on it)
   * @returns the header announcement (at most once) and the frames this chunk
   *   completed, plus the refusal that stopped it if there was one
   */
  push(wemChunk: Uint8Array | ArrayBuffer): DecodeStep;
  /**
   * Finish the decode and return the last frames. Terminal — the decoder must
   * be destroyed afterwards. On success the session has delivered exactly the
   * frame count the container declares.
   */
  finish(): DecodeStep;
  /** Release the decoder (safe before or after finish). */
  destroy(): void;
}

/**
 * One one-shot decode: the container's header announcement (geometry and setup
 * packet) plus every frame the session delivered as one interleaved f32 buffer.
 */
export interface DecodeResult {
  /** PCM channel count the container declares. */
  channels: number;
  /** PCM sample rate the container declares. */
  sampleRate: number;
  /** The setup packet this build parsed. */
  setup: Uint8Array;
  /**
   * Frames delivered — counted from the steps, which for a session that
   * finished with `WEM_OK` is exactly the container's declared count.
   */
  frames: number;
  /** Interleaved f32 samples at ±1.0 full scale. */
  pcm: Float32Array;
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
  "WEM_ERR_INPUT_MALFORMED",
] as const;

// ---------------------------------------------------------------------------
// Core loading (environment-aware)
// ---------------------------------------------------------------------------

let corePromise: Promise<CoreModule> | null = null;
let initInput: unknown;

/**
 * Whether this module runs under Node (the nodejs build self-initializes on
 * import; the web build needs an explicit init call). Read through
 * `globalThis` so the file type-checks and runs without `@types/node`.
 */
function isNodeEnvironment(): boolean {
  const process = (globalThis as { process?: { versions?: { node?: string } } }).process;
  return typeof process?.versions?.node === "string";
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
// Small helpers (no numerics, no profile logic)
// ---------------------------------------------------------------------------

/** One wrapper-side argument error carrying a stable `WEM_ERR_*` code. */
function wemError(codeMessage: string): Error & { code: string } {
  const code = codeMessage.split(":")[0];
  const err = new Error(codeMessage) as Error & { code: string };
  err.code = code;
  return err;
}

/** Accept `Uint8Array` or `ArrayBuffer` (one canonical form downstream). */
function asBytes(value: Uint8Array | ArrayBuffer, what: string): Uint8Array {
  if (value instanceof Uint8Array) return value;
  if (typeof ArrayBuffer !== "undefined" && value instanceof ArrayBuffer) {
    return new Uint8Array(value);
  }
  throw wemError(`WEM_ERR_STATE_ERROR: ${what} must be Uint8Array or ArrayBuffer`);
}

/** The kernel result object as the public shape. */
function toResult(raw: WemResultRaw): WemResult {
  return {
    data: raw.data,
    stats: raw.stats,
  };
}

/** Whether one geometry source is an already-parsed WAV. */
function isParsedWav(source: GeometrySource): source is ParsedWav {
  return (source as ParsedWav).pcm instanceof Uint8Array;
}

/**
 * The selection one geometry source means: an explicit selection is taken
 * as given (with `version` omitted, the shell auto-selects the compiled
 * generation); a parsed WAV yields its own geometry.
 */
function asSelection(source: GeometrySource): ProfileSelection {
  return isParsedWav(source)
    ? { version: null, channels: source.channels, sampleRate: source.sampleRate }
    : source;
}

// ---------------------------------------------------------------------------
// Profile selection
// ---------------------------------------------------------------------------

/** Every Wwise generation this build can select (the compiled-in table). */
export async function listVersions(): Promise<WwiseVersionInfo[]> {
  const coreModule = await core();
  return coreModule.wem_versions();
}

/**
 * Open one reusable one-shot encoder on a selection (or on a parsed WAV's
 * own geometry).
 *
 * Throws `WEM_ERR_FORMAT_UNSUPPORTED` on an unknown version code,
 * `WEM_ERR_STATE_ERROR` on a non-positive geometry, and
 * `WEM_ERR_PROFILE_NOT_FOUND` when no compiled configuration satisfies the
 * selection.
 */
export async function createEncoder(source: GeometrySource): Promise<Encoder> {
  const coreModule = await core();
  const selection = asSelection(source);
  const encoder = new coreModule.WemEncoder(
    selection.version ?? null,
    selection.channels,
    selection.sampleRate,
  );
  return {
    selection() {
      return encoder.selection();
    },
    encodePcm16Interleaved(pcm) {
      return toResult(encoder.encode_pcm16_interleaved(asBytes(pcm, "PCM input")));
    },
    encodeWav(wavBytes) {
      return toResult(encoder.encode_wav(asBytes(wavBytes, "WAV input")));
    },
    destroy() {
      encoder.free();
    },
  };
}

// ---------------------------------------------------------------------------
// Encoding
// ---------------------------------------------------------------------------

/** Parse one signed-16 PCM WAV (kernel parser; throws on non-conforming files). */
export async function parseWav(wavBytes: Uint8Array | ArrayBuffer): Promise<ParsedWav> {
  const coreModule = await core();
  const parsed = coreModule.wem_parse_wav(asBytes(wavBytes, "WAV input"));
  return {
    sampleRate: parsed.sampleRate,
    channels: parsed.channels,
    frames: parsed.frames,
    pcm: parsed.pcm,
  };
}

/** The selection a WAV's own geometry asks for (generation auto-selected). */
function selectionFromWav(coreModule: CoreModule, wav: Uint8Array): ProfileSelection {
  const parsed = coreModule.wem_parse_wav(wav);
  return { version: null, channels: parsed.channels, sampleRate: parsed.sampleRate };
}

/**
 * One-shot encode: WAV bytes -> complete WEM container.
 *
 * Without `selection` the generation and geometry are auto-selected from
 * the WAV itself; with one, the kernel cross-checks the WAV geometry
 * against it (`WEM_ERR_GEOMETRY_MISMATCH` on disagreement) and rejects a
 * stream shorter than the 4096-frame minimum
 * (`WEM_ERR_INPUT_TOO_SHORT`).
 */
export async function encodeWav(
  wavBytes: Uint8Array | ArrayBuffer,
  selection?: GeometrySource,
): Promise<WemResult> {
  const coreModule = await core();
  const wav = asBytes(wavBytes, "WAV input");
  const resolved = asSelection(selection ?? selectionFromWav(coreModule, wav));
  const encoder = new coreModule.WemEncoder(
    resolved.version ?? null,
    resolved.channels,
    resolved.sampleRate,
  );
  try {
    return toResult(encoder.encode_wav(wav));
  } finally {
    encoder.free();
  }
}

/**
 * Open one streaming encode session on a selection (or on a parsed WAV's
 * own geometry).
 *
 * Drives the kernel's `StreamSession::for_selection` (Init); the reply
 * framing (seq 0 = setup packet, then audio packets) and the chunk framing
 * rules are the kernel's — chunk boundaries never affect the output bytes.
 */
export async function createStreamSession(source: GeometrySource): Promise<StreamSession> {
  const coreModule = await core();
  const selection = asSelection(source);
  const session = new coreModule.WemSession(
    selection.version ?? null,
    selection.channels,
    selection.sampleRate,
  );
  return {
    selection() {
      return session.selection();
    },
    push(pcmChunk, onPackets) {
      const bytes = asBytes(pcmChunk, "PCM chunk");
      const packets = session.push(bytes);
      if (onPackets) onPackets(packets);
      return packets;
    },
    finish() {
      return toResult(session.finish());
    },
    pcmFrames() {
      return session.pcm_frames();
    },
    destroy() {
      session.free();
    },
  };
}

// ---------------------------------------------------------------------------
// Decoding (include/wem.h section 5)
// ---------------------------------------------------------------------------

/** The shell's step object as the public shape (a copy of the flat fields). */
function toStep(raw: DecodeStepRaw): DecodeStep {
  return {
    header:
      raw.header === null
        ? null
        : {
            channels: raw.header.channels,
            sampleRate: raw.header.sampleRate,
            setup: raw.header.setup,
          },
    channels: raw.channels,
    frames: raw.frames,
    pcm: raw.pcm,
    error:
      raw.error === null ? null : { code: raw.error.code, message: raw.error.message },
  };
}

/**
 * Open one streaming decode session (`wem_decoder_new`): Init -> push* ->
 * Finish -> destroy. There is no selection argument, and nothing is resolved
 * until the container's own header region arrives.
 */
export async function createDecoder(): Promise<Decoder> {
  const coreModule = await core();
  const decoder = new coreModule.WemDecoder();
  return {
    push(wemChunk) {
      return toStep(decoder.push(asBytes(wemChunk, "WEM chunk")));
    },
    finish() {
      return toStep(decoder.finish());
    },
    destroy() {
      decoder.free();
    },
  };
}

/**
 * One-shot decode: WEM bytes -> the container's geometry, its setup packet, and
 * every frame it delivered as one interleaved f32 buffer.
 *
 * The whole WEM is pushed as a single chunk, so the session's reply carries
 * the whole PCM at once; a caller that wants bounded pushes (and the frames
 * each one completed) drives {@link createDecoder} instead. Chunking never
 * changes the samples either way. `frames` is what the session delivered —
 * for a decode that finishes with `WEM_OK` that is exactly the frame count the
 * container declares, but this surface reads no declared count (the wasm
 * shell's header announcement does not carry one).
 *
 * Throws the stable `WEM_ERR_*` error of the refusal that stopped the decode
 * (including `WEM_ERR_INPUT_MALFORMED` for bytes that are not a container
 * this revision parses, and `WEM_ERR_FORMAT_UNSUPPORTED` for a container whose
 * setup packet this build does not carry). The prefix a refusal completed is
 * still available — from a streaming {@link Decoder}, whose step carries the
 * samples and the refusal together.
 */
export async function decodeWem(wemBytes: Uint8Array | ArrayBuffer): Promise<DecodeResult> {
  const decoder = await createDecoder();
  const steps: DecodeStep[] = [];
  try {
    steps.push(decoder.push(asBytes(wemBytes, "WEM input")));
    steps.push(decoder.finish());
  } finally {
    decoder.destroy();
  }

  let header: DecodedHeader | null = null;
  let frames = 0;
  let samples = 0;
  for (const step of steps) {
    // The announcement fires at most once, before any PCM.
    if (step.header) header = step.header;
    frames += step.frames;
    samples += step.pcm.length;
    if (step.error) throw wemError(`${step.error.code}: ${step.error.message}`);
  }
  if (header === null) {
    throw wemError("WEM_ERR_INTERNAL: the decode session resolved no geometry");
  }
  const pcm = new Float32Array(samples);
  let offset = 0;
  for (const step of steps) {
    pcm.set(step.pcm, offset);
    offset += step.pcm.length;
  }
  return {
    channels: header.channels,
    sampleRate: header.sampleRate,
    setup: header.setup,
    frames,
    pcm,
  };
}