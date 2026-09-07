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
 *   1. LIFECYCLE
 *   2. ERROR CODES
 *   3. MEMORY OWNERSHIP
 *   4. CROSS-LANGUAGE INTEGRATION RULES
 *
 * Evolution: WemError values are stable across all revisions — new
 * codes are appended, never renumbered or reused. Lifecycle semantics
 * and callback behavior are likewise contract: a change that breaks a
 * conforming client is a new major revision of this header, not an edit.
 */
#ifndef WEM_H
#define WEM_H

#include <stdint.h>
#include <stddef.h>

#ifdef __cplusplus
extern "C" {
#endif

/*
 * 1. LIFECYCLE
 *
 * The kernel exposes two equivalent encoders over the same profile:
 *
 *  (a) ONE-SHOT:
 *      wem_encode_pcm16_interleaved(...)      — profile + PCM in,
 *      container bytes out via the write callback. Or via a handle:
 *      wem_encoder_new / wem_encoder_encode / wem_encoder_free.
 *
 *  (b) STREAMING:
 *      wem_session_new(...)    — Init: open on one installed profile
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
 *   WEM_ERR_PROFILE_NOT_FOUND   — no installed profile matches the
 *                                 requested reference
 *   WEM_ERR_STATE_ERROR         — lifecycle violation, or a malformed
 *                                 argument (NULL where a value is
 *                                 required)
 *   WEM_ERR_GEOMETRY_MISMATCH   — PCM geometry disagrees with the
 *                                 profile
 *   WEM_ERR_INPUT_TOO_SHORT     — fewer than 4096 PCM frames
 *   WEM_ERR_FORMAT_UNSUPPORTED  — sample layout not supported by this
 *                                 revision
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
 *    bytes (channels per the selected profile) for the duration of the
 *    call; the kernel never writes into client PCM memory.
 *  - Handles returned through `out_...` pointers are owned by the
 *    client and released with the matching *_free (NULL is a no-op).
 *  - `user_data` is an opaque client pointer, passed back verbatim.
 */

/* Profile-resolved shareable encoder (concurrent encodes OK). */
WemError wem_encoder_new(const char *profile_name, const char *data_dir,
                         WemEncoder **out_encoder);
void wem_encoder_free(WemEncoder *encoder);
WemError wem_encoder_encode(const WemEncoder *encoder, const int16_t *pcm,
                            size_t frames, WemWriteCb write_cb,
                            void *user_data);

/* One-shot convenience: profile + PCM in, container bytes out via
 * write_cb. `data_dir` scopes the kernel's WEM_DATA_DIR override;
 * NULL/"" uses the kernel default (environment or repository layout). */
WemError wem_encode_pcm16_interleaved(const char *profile_name,
                                      const char *data_dir,
                                      const int16_t *pcm, size_t frames,
                                      WemWriteCb write_cb, void *user_data);

/* Streaming session: Init -> push* -> Finish -> free. `write_cb` is
 * required (terminal container bytes); `packet_cb` may be NULL to
 * discard intermediate packets. */
WemError wem_session_new(const char *profile_name, const char *data_dir,
                         WemWriteCb write_cb, WemPacketCb packet_cb,
                         void *user_data, WemSession **out_session);
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
