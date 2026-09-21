/*
 * wem.h — WEM encoder kernel: C ABI core surface.
 *
 * This header is the normative cross-language contract of the WEM
 * encoder kernel. The Rust kernel (crates/) implements it; every
 * language binding (the PyO3 extension, Go cgo, a future wasm build, ...)
 * is a parallel shell over this surface and must mirror its lifecycle,
 * reply framing, and error codes without inventing variants.
 *
 * The sections below are normative:
 *
 *   PROFILE SELECTION
 *   1. LIFECYCLE
 *   2. ERROR CODES
 *   3. MEMORY OWNERSHIP
 *   4. CROSS-LANGUAGE INTEGRATION RULES
 *
 * Evolution: WemError values are stable across all revisions — new
 * codes are appended, never renumbered or reused. Lifecycle semantics
 * and callback behavior are likewise contract: a change that breaks a
 * conforming client is a new major revision of this header, not an edit.
 *
 * ABI revision 2 replaced the `profile_name` + `data_dir` argument pair
 * of `wem_encoder_new`, `wem_encode_pcm16_interleaved` and
 * `wem_session_new` with one `const WemProfile *` selection, so that
 * PROFILE SELECTION below is the entire profile contract: no entry
 * accepts a profile name, a profile directory, profile index/manifest
 * bytes, or an environment variable. A conforming revision 1 client
 * must be recompiled against this header; WemError values were not
 * renumbered and remain stable.
 */
#ifndef WEM_H
#define WEM_H

#include <stdint.h>
#include <stddef.h>

/* Current ABI revision of this header (see the evolution note above). */
#define WEM_ABI_REVISION 2

#ifdef __cplusplus
extern "C" {
#endif

/*
 * PROFILE SELECTION
 *
 * One structured selection names the encoder configuration to use: a
 * Wwise generation plus the PCM geometry. Nothing else selects a profile
 * — how the kernel stores and addresses an encoder configuration is its
 * own business, and never crosses this boundary.
 *
 * A WemProfile denotes exactly one configuration compiled into this
 * library:
 *
 *   - `version`      the Wwise generation (WemVersion below);
 *   - `channels`     the PCM channel count to encode, > 0;
 *   - `sample_rate`  the PCM sample rate to encode, > 0.
 *
 * A selection no compiled configuration satisfies fails with
 * WEM_ERR_PROFILE_NOT_FOUND. A `version` outside the table below is not a
 * supported value of this revision and fails with
 * WEM_ERR_FORMAT_UNSUPPORTED. A non-positive geometry is a malformed
 * argument and fails with WEM_ERR_STATE_ERROR. The selection is never
 * silently substituted by a default.
 *
 * The struct is passed by pointer, and the pointer need only stay valid
 * for the duration of the call that takes it. Its layout (two 32-bit
 * geometry fields after a 32-bit version code) is part of this contract;
 * fields are appended, never reordered.
 *
 * Evolution: WemVersion codes are stable and append-only, exactly like
 * WemError values — a new Wwise generation appends a code, it never
 * renumbers or reuses one.
 */
typedef enum WemVersion {
  /* Wwise 2013.2. */
  WEM_WWISE_2013 = 0
} WemVersion;

typedef struct WemProfile {
  WemVersion version;
  int32_t channels;
  int32_t sample_rate;
} WemProfile;

/*
 * 1. LIFECYCLE
 *
 * The kernel exposes two equivalent encoders over the same selection:
 *
 *  (a) ONE-SHOT:
 *      wem_encode_pcm16_interleaved(...)      — profile + PCM in,
 *      container bytes out via the write callback. Or via a handle:
 *      wem_encoder_new / wem_encoder_encode / wem_encoder_free.
 *
 *  (b) STREAMING:
 *      wem_session_new(...)    — Init: open on one profile selection
 *      wem_session_push(...)   — chunk*: zero or more PCM chunks
 *      wem_session_finish(...) — Finish: complete the encode; container
 *                                bytes out via the write callback, plus
 *                                the terminal WemMeta summary
 *      wem_session_free(...)   — release the handle
 *
 *  Streaming rules (identical to the kernel):
 *   - Exactly one Init, zero or more chunks, exactly one Finish.
 *   - Reply packets arrive through the packet callback in stream order:
 *     seq 0 is the Vorbis setup packet, then audio packets in encoding
 *     order. seq increases monotonically across push calls.
 *   - Chunk boundaries never affect the output bytes: any chunking of
 *     the same PCM stream reproduces the one-shot container byte-for-
 *     byte. An empty chunk is a no-op.
 *   - finish() is terminal regardless of its outcome: after it (success
 *     or error) the session must be freed, not reused.
 *   - The PCM input is interleaved little-endian signed 16-bit; a stream
 *     shorter than the 4096-frame minimum is rejected at finish.
 *
 *  Thread-safety semantics:
 *   - WemEncoder is shareable: concurrent encodes may use one handle on
 *     different threads.
 *   - WemSession has single-threaded ownership: it must not be shared
 *     between threads.
 */

/* Opaque handles (never dereferenced by clients). */
typedef struct WemEncoder WemEncoder;
typedef struct WemSession WemSession;

/*
 * 2. ERROR CODES
 *
 * 1:1 with the kernel error classes (wem-core EncoderError):
 *   WEM_OK                      — success
 *   WEM_ERR_PROFILE_NOT_FOUND   — no compiled profile satisfies the
 *                                 WemProfile selection
 *   WEM_ERR_STATE_ERROR         — lifecycle violation, or a malformed
 *                                 argument (NULL where a value is
 *                                 required)
 *   WEM_ERR_GEOMETRY_MISMATCH   — PCM geometry disagrees with the
 *                                 selection
 *   WEM_ERR_INPUT_TOO_SHORT     — fewer than 4096 PCM frames
 *   WEM_ERR_FORMAT_UNSUPPORTED  — sample layout, or a WemVersion code,
 *                                 not supported by this revision
 *   WEM_ERR_INTERNAL            — internal fault (a kernel panic can
 *                                 never unwind across this boundary; it
 *                                 surfaces as this code)
 */
typedef enum WemError {
  WEM_OK = 0,
  WEM_ERR_PROFILE_NOT_FOUND = 1,
  WEM_ERR_STATE_ERROR = 2,
  WEM_ERR_GEOMETRY_MISMATCH = 3,
  WEM_ERR_INPUT_TOO_SHORT = 4,
  WEM_ERR_FORMAT_UNSUPPORTED = 5,
  WEM_ERR_INTERNAL = 6
} WemError;

/*
 * Callbacks.
 *
 * WemWriteCb receives output bytes in bounded blocks (the whole
 * container is delivered across one or more calls — callback-style
 * output, so there is no container size limit). The pointer is valid
 * only during the call. Returning non-WEM_OK aborts the encode and the
 * caller receives that code.
 *
 * WemPacketCb receives emitted streaming packets in reply order; seq 0
 * is the setup packet. Returning non-WEM_OK marks the session failed.
 */
typedef WemError (*WemWriteCb)(const uint8_t *data, size_t len,
                               void *user_data);
typedef WemError (*WemPacketCb)(uint32_t seq, const uint8_t *data,
                                size_t len, void *user_data);

/* Terminal container summary: byte length + lowercase-hex SHA-256
 * (exactly 64 characters, no NUL). */
typedef struct WemMeta {
  uint64_t total_len;
  char sha256_hex[64];
} WemMeta;

/*
 * 3. MEMORY OWNERSHIP
 *
 *  - All output flows through the write callback in bounded blocks; the
 *    kernel owns every buffer it passes and the client copies.
 *  - The client must back `pcm` with at least frames * channels * 2
 *    bytes (channels per the selection) for the duration of the call;
 *    the kernel never writes into client PCM memory.
 *  - A `const WemProfile *` is borrowed for the duration of the call
 *    that takes it; the kernel never retains it.
 *  - Handles returned through `out_...` pointers are owned by the
 *    client and released with the matching *_free (NULL is a no-op).
 *  - `user_data` is an opaque client pointer, passed back verbatim.
 */

/* Profile-resolved shareable encoder (concurrent encodes OK). */
WemError wem_encoder_new(const WemProfile *profile, WemEncoder **out_encoder);
void wem_encoder_free(WemEncoder *encoder);
WemError wem_encoder_encode(const WemEncoder *encoder, const int16_t *pcm,
                            size_t frames, WemWriteCb write_cb,
                            void *user_data);

/* One-shot convenience: profile + PCM in, container bytes out via
 * write_cb. */
WemError wem_encode_pcm16_interleaved(const WemProfile *profile,
                                      const int16_t *pcm, size_t frames,
                                      WemWriteCb write_cb, void *user_data);

/* Streaming session: Init -> push* -> Finish -> free. `write_cb` is
 * required (terminal container bytes); `packet_cb` may be NULL to
 * discard intermediate packets. */
WemError wem_session_new(const WemProfile *profile, WemWriteCb write_cb,
                         WemPacketCb packet_cb, void *user_data,
                         WemSession **out_session);
WemError wem_session_push(WemSession *session, const uint8_t *data,
                          size_t len);
WemError wem_session_finish(WemSession *session, WemMeta *out_meta);
void wem_session_free(WemSession *session);

/*
 * 4. CROSS-LANGUAGE INTEGRATION RULES
 *
 *  - This header is the single authority for the cross-language
 *    contract. A new language integrates by writing a thin shell over
 *    these symbols (e.g. Go via cgo, as in examples/go-cgo); it never
 *    routes same-process calls through RPC or serialization, and never
 *    adds language-specific special cases to the kernel.
 *  - Shells must map errors 1:1 (WemError -> the language's exception/
 *    error type) and mirror the lifecycle: no extra Init, no re-entrant
 *    Finish, no chunk-boundary-dependent behavior.
 *  - Shells must not embed numerics or profile logic: byte-exactness is
 *    defined by the kernel and pinned by the golden contracts.
 */

#ifdef __cplusplus
} /* extern "C" */
#endif

#endif /* WEM_H */
