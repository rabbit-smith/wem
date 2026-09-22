/*
 * wem.h — WEM encoder kernel: C ABI core surface.
 *
 * This header is the interface the language shells mirror: the C ABI of
 * the WEM encoder kernel. The Rust kernel (crates/) implements it; every
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
 * and callback behavior are likewise part of the interface: a change that
 * breaks a conforming client is a new major revision of this header, not an
 * edit.
 *
 * ABI revision 2 replaced the `profile_name` + `data_dir` argument pair
 * of `wem_encoder_new`, `wem_encode_pcm16_interleaved` and
 * `wem_session_new` with one `const WemProfile *` selection, so that
 * PROFILE SELECTION below is the entire profile surface: no entry
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
 * geometry fields after a 32-bit version code) is fixed; fields are
 * appended, never reordered.
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
 *   - A handle can also be killed by a defect inside the library: a call
 *     that reports WEM_ERR_INTERNAL is terminal for the handle it ran in,
 *     and every later call on that handle fails with WEM_ERR_STATE_ERROR
 *     ("Panics" in section 2 is the authority on which calls those are).
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
 *   WEM_ERR_STATE_ERROR         — malformed call, or a terminal handle;
 *                                 see "one code, three situations" below
 *   WEM_ERR_GEOMETRY_MISMATCH   — PCM geometry disagrees with the
 *                                 selection
 *   WEM_ERR_INPUT_TOO_SHORT     — fewer than 4096 PCM frames
 *   WEM_ERR_FORMAT_UNSUPPORTED  — sample layout, or a WemVersion code,
 *                                 not supported by this revision
 *   WEM_ERR_INTERNAL            — internal fault: a defect in this
 *                                 library, never a rejection of the
 *                                 caller's input (a kernel panic can
 *                                 never unwind across this boundary; it
 *                                 surfaces as this code — see "Panics")
 *
 * One code, three situations: WEM_ERR_STATE_ERROR covers
 *
 *   (a) a malformed call — NULL where a value is required (a NULL
 *       WemProfile, pcm, out-pointer or required callback), or a
 *       non-positive WemProfile geometry;
 *   (b) a call outside a handle's lifecycle — push or finish on a session
 *       that already finished, or any call on a handle a defect has killed;
 *   (c) the handle this call needed was never handed out, because the Init
 *       that would have produced it failed (the out-pointer it returned was
 *       NULL, which is what the call then rejected).
 *
 * These are not three different recoveries, which is why they share one
 * code and why this header does not split them:
 *
 *   - The call did not run in every case. A handle the call had is released
 *     with the matching *_free like any other (NULL is a no-op), and no
 *     later call resumes it; a call without a handle is simply rewritten.
 *   - The caller already holds the fact that separates them: whether it
 *     passed NULL, and what its own earlier calls returned. A defect is
 *     reported by the call that hits it — WEM_ERR_INTERNAL, once — never
 *     retroactively as this code, and finish() is documented as terminal.
 *   - The outcomes that *are* different recoveries are never folded in
 *     here: WEM_ERR_GEOMETRY_MISMATCH, WEM_ERR_INPUT_TOO_SHORT,
 *     WEM_ERR_FORMAT_UNSUPPORTED and WEM_ERR_PROFILE_NOT_FOUND all leave a
 *     live handle usable for the next call, and WEM_ERR_INTERNAL is the
 *     defect.
 *
 * Splitting (a), (b) and (c) into separate codes would change what an
 * existing client observes for the same call, so it is a new revision of
 * this header, not an edit to this one. A client that wants to react
 * differently should key off the call it made, not off this code.
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

/*
 * Panics.
 *
 * A kernel panic means this library broke an invariant. It never means the
 * caller passed something bad: a malformed argument, an unsupported layout
 * or an unsatisfiable selection has its own code in the table above. A
 * panic never unwinds across this boundary — every entry catches it and
 * reports WEM_ERR_INTERNAL.
 *
 * What that costs the caller is decided by the entry, not by the fault:
 *
 *   - One-shot entries carry no state across calls. A caught panic in
 *     wem_encode_pcm16_interleaved, wem_encoder_new or wem_session_new
 *     rejects that call and hands out no handle: nothing else changed, so
 *     calling again is allowed.
 *   - A handle carries state across calls, so a defect that runs through
 *     one is terminal. A caught panic in wem_encoder_encode,
 *     wem_session_push or wem_session_finish reports WEM_ERR_INTERNAL for
 *     that call and marks the handle dead: every later call on it returns
 *     WEM_ERR_STATE_ERROR, whatever it asks for. A dead handle is released
 *     with the matching *_free like any other (NULL is a no-op).
 *     A rejection carrying any other code is not terminal: it refuses that
 *     call and leaves the handle usable.
 *
 * Two outcomes are terminal by their own rule rather than by a panic:
 * wem_session_finish ends its session whatever it returns, and a callback
 * returning anything other than WEM_OK aborts the work it was delivering
 * (a packet callback's abort marks its session failed).
 *
 * Catching a panic is what turns it into a WEM_ERR_INTERNAL return value,
 * and catching only works while panics unwind: the library must not be
 * built with `panic = "abort"`, where every caught panic would become a
 * process abort instead. (wem-capi refuses to compile in that
 * configuration.)
 */

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
 *  - `user_data` is an opaque client pointer, passed back verbatim.
 *
 *  OUT-PARAMETERS: WHAT EVERY EXIT WRITES
 *
 *  A C caller cannot tell an unwritten out-parameter from one holding
 *  garbage, so each one is specified for every exit, not only for success:
 *
 *  - `wem_encoder_new` and `wem_session_new` write their out-pointer on
 *    every exit that can reach it: the handle on WEM_OK, NULL on every
 *    failure. The value is therefore always readable and always safe to
 *    hand to the matching *_free, whatever the return code says. (A NULL
 *    out-pointer is itself the malformed call WEM_ERR_STATE_ERROR, and
 *    there is nothing to write.)
 *  - `wem_session_finish` writes `*out_meta` when and only when it returns
 *    WEM_OK. On every other return the struct is left exactly as the caller
 *    had it, and its contents are not a summary of anything: there is no
 *    empty WemMeta, because a zero length and a 64-character lowercase-hex
 *    digest have no value that means "no container was produced". The
 *    return code is what says whether the summary exists, and a caller must
 *    not read `*out_meta` unless that code was WEM_OK.
 *  - A handle returned through an out-pointer is owned by the client and
 *    released with the matching *_free (NULL is a no-op).
 */

/* Profile-resolved shareable encoder (concurrent encodes OK).
 * `*out_encoder` is written on every exit: the handle on WEM_OK, NULL on
 * every failure (section 3). */
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
 * discard intermediate packets. `*out_session` is written on every exit:
 * the handle on WEM_OK, NULL on every failure (section 3). */
WemError wem_session_new(const WemProfile *profile, WemWriteCb write_cb,
                         WemPacketCb packet_cb, void *user_data,
                         WemSession **out_session);
WemError wem_session_push(WemSession *session, const uint8_t *data,
                          size_t len);
/* Finish: complete the encode, deliver the container bytes through the
 * session's write callback, and fill `*out_meta` — written when and only
 * when this returns WEM_OK; untouched on every other return (section 3).
 * Terminal: whatever it returns, the session is then released, not reused. */
WemError wem_session_finish(WemSession *session, WemMeta *out_meta);
void wem_session_free(WemSession *session);

/*
 * 4. CROSS-LANGUAGE INTEGRATION RULES
 *
 *  - This header is the single authority for the cross-language
 *    interface. A new language integrates by writing a thin shell over
 *    these symbols (e.g. Go via cgo, as in examples/go-cgo); it never
 *    routes same-process calls through RPC or serialization, and never
 *    adds language-specific special cases to the kernel.
 *  - Shells must map errors 1:1 (WemError -> the language's exception/
 *    error type) and mirror the lifecycle: no extra Init, no re-entrant
 *    Finish, no chunk-boundary-dependent behavior.
 *  - Shells must not embed numerics or profile logic: byte-exactness is
 *    defined by the kernel and checked by the parity suites.
 */

/*
 * APPENDIX: MIGRATING FROM REVISION 1
 *
 * Informative, not interface: the normative text is above.
 *
 * The three profile-taking entries lost their `profile_name` + `data_dir`
 * argument pair and gained one borrowed `const WemProfile *`:
 *
 *   revision 1                                    revision 2
 *   ----------                                    ----------
 *   wem_encoder_new(name, dir, &enc)              wem_encoder_new(&prof, &enc)
 *
 *   wem_encode_pcm16_interleaved(                 wem_encode_pcm16_interleaved(
 *       name, dir, pcm, frames, wb, ud)               &prof, pcm, frames, wb, ud)
 *
 *   wem_session_new(                              wem_session_new(
 *       name, dir, wb, pb, ud, &session)              &prof, wb, pb, ud, &session)
 *
 * `wem_encoder_encode`, `wem_encoder_free`, `wem_session_push`,
 * `wem_session_finish` and `wem_session_free` are unchanged, as is every
 * WemError value, the lifecycle, and the reply framing.
 *
 * Building the selection: the generation is the WEM_WWISE_2013 code and the
 * two geometry fields are the channel count and sample rate the client was
 * about to encode anyway. A client that used to name a profile it installed
 * can read exactly those three numbers from that profile's manifest —
 * `key.generation` ("2013.2" names WEM_WWISE_2013), `key.channels`,
 * `key.sample_rate`. A NULL or empty `data_dir` simply disappears: the
 * library always uses the profiles compiled into it, which is why there is
 * no directory argument left to pass.
 *
 * The struct is borrowed only for the duration of the call, so a stack value
 * is enough:
 *
 *   WemProfile profile = { WEM_WWISE_2013, 6, 44100 };
 *   WemEncoder *encoder = NULL;
 *   WemError code = wem_encoder_new(&profile, &encoder);
 *
 * A selection naming a configuration this library does not carry is
 * WEM_ERR_PROFILE_NOT_FOUND, and never falls back to another one.
 */

#ifdef __cplusplus
} /* extern "C" */
#endif

#endif /* WEM_H */
