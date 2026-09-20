//! wem-wasm — the wasm-bindgen shell of the WEM encoder kernel.
//!
//! A **parallel language shell** over the same kernel contract as the C ABI
//! (`include/wem.h`), for browser and web-worker use (crates/AGENTS.md,
//! "C ABI contract"): same lifecycle, same error classes, same bytes; this
//! shell owns no numerics and no profile logic.
//!
//! # Contract mapping (1:1 with include/wem.h)
//!
//! * **One-shot** (`wem_encoder_*` / `wem_encode_pcm16_interleaved`):
//!   [`WemEncoder`] — the shareable encoder handle. Profile data arrives as
//!   bytes (no filesystem on wasm32-unknown-unknown): the constructor takes
//!   the raw `index.json` bytes plus every profile-resource file as
//!   `(path, bytes)` pairs and drives the kernel's bytes entry
//!   (`wem_core::Encoder::from_profile_bytes_named`, which SHA-256 verifies every
//!   logical resource on load).
//! * **Streaming** (`wem_session_*`): [`WemSession`] — Init -> push* ->
//!   Finish -> free. `new` is the Init (bytes entry
//!   `StreamSession::for_profile_ref_bytes`, same verification); `push`
//!   emits the packets that just completed (seq 0 = setup packet, then
//!   audio packets in encoding order) as a returned array instead of a C
//!   callback — the JS wrapper can translate that into callbacks; the
//!   kernel reply framing is unchanged. `finish` returns the terminal
//!   container bytes plus the `WemMeta` summary (`totalLen`, `sha256Hex`).
//! * **Error codes**: every fallible export throws a JS `Error` whose
//!   `code` property (and message prefix) is the stable `WEM_ERR_*` string
//!   of the C ABI error table — append-only, never renumbered.
//!
//! # Panics
//!
//! Kernel invariants are unreachable for well-formed inputs (the kernel
//! returns typed errors on every input-derived path). On
//! wasm32-unknown-unknown the default `panic = "abort"` applies, so a
//! genuine invariant violation terminates the wasm instance instead of
//! unwinding — the web equivalent of the C ABI's `WEM_ERR_INTERNAL`
//! boundary.
//!
//! # No DOM
//!
//! Only `wasm-bindgen` / `js-sys` are used; nothing here touches the DOM,
//! so the shell runs unchanged in browsers, web workers, and Node.

use wasm_bindgen::prelude::*;
use wasm_bindgen::JsCast;

use wem_core::encoder::{Encoder, Pcm16};
use wem_core::error::EncoderError;
use wem_core::stream::{ProfileRef, StreamSession};
use wem_core::usecases::wav::parse_pcm16;

// Stable error-code strings (include/wem.h section 2; 1:1 with WemError).
// `WEM_OK` documents the full table; no kernel error maps to it.
#[allow(dead_code)]
const WEM_OK: &str = "WEM_OK";
const WEM_ERR_PROFILE_NOT_FOUND: &str = "WEM_ERR_PROFILE_NOT_FOUND";
const WEM_ERR_STATE_ERROR: &str = "WEM_ERR_STATE_ERROR";
const WEM_ERR_GEOMETRY_MISMATCH: &str = "WEM_ERR_GEOMETRY_MISMATCH";
const WEM_ERR_INPUT_TOO_SHORT: &str = "WEM_ERR_INPUT_TOO_SHORT";
const WEM_ERR_FORMAT_UNSUPPORTED: &str = "WEM_ERR_FORMAT_UNSUPPORTED";
const WEM_ERR_INTERNAL: &str = "WEM_ERR_INTERNAL";

/// The stable error code of one kernel failure (the WemError discriminants).
fn error_code(error: &EncoderError) -> &'static str {
    match error {
        EncoderError::ProfileNotFound { .. } => WEM_ERR_PROFILE_NOT_FOUND,
        EncoderError::StateError { .. } => WEM_ERR_STATE_ERROR,
        EncoderError::GeometryMismatch { .. } => WEM_ERR_GEOMETRY_MISMATCH,
        EncoderError::InputTooShort { .. } => WEM_ERR_INPUT_TOO_SHORT,
        EncoderError::FormatUnsupported { .. } => WEM_ERR_FORMAT_UNSUPPORTED,
        EncoderError::Internal(_) => WEM_ERR_INTERNAL,
    }
}

/// One thrown JS error: `message` = "<CODE>: <kernel detail>",
/// `code` = the stable WEM_ERR_* string.
fn js_error(code: &'static str, detail: &str) -> JsValue {
    let err = js_sys::Error::new(&format!("{code}: {detail}"));
    let _ = js_sys::Reflect::set(&err, &JsValue::from_str("code"), &JsValue::from_str(code));
    JsValue::from(err)
}

/// Map one kernel error to its thrown form.
fn reject(error: &EncoderError) -> JsValue {
    js_error(error_code(error), &error.to_string())
}

fn set(obj: &js_sys::Object, key: &str, value: JsValue) {
    let _ = js_sys::Reflect::set(obj, &JsValue::from_str(key), &value);
}

/// The profile-file bytes argument: a JS `Map` (path -> bytes) or a plain
/// object (path -> bytes), where bytes are `Uint8Array` or `ArrayBuffer`.
/// Paths are profiles-dir-relative POSIX keys, exactly as the kernel's
/// bytes entry documents.
fn collect_files(files: &JsValue) -> Result<Vec<(String, Vec<u8>)>, JsValue> {
    let malformed = || {
        js_error(
            WEM_ERR_STATE_ERROR,
            "profile files must be a Map or object of bytes",
        )
    };
    let bad_key = || js_error(WEM_ERR_STATE_ERROR, "profile file path must be a string");

    // A JS Map (path -> bytes).
    if files.has_type::<js_sys::Map<JsValue, JsValue>>() {
        let map = files
            .dyn_ref::<js_sys::Map<JsValue, JsValue>>()
            .ok_or_else(malformed)?;
        let mut out = Vec::with_capacity(map.size() as usize);
        let iter = map.entries();
        loop {
            let step = iter.next().map_err(|_| malformed())?;
            if step.done() {
                break;
            }
            let pair_value = step.value();
            let pair = pair_value
                .dyn_ref::<js_sys::Array>()
                .ok_or_else(malformed)?;
            let key_js: JsValue = pair.get(0);
            let value_js: JsValue = pair.get(1);
            let key = key_js.as_string().ok_or_else(bad_key)?;
            out.push((key, to_bytes(&value_js)?));
        }
        return Ok(out);
    }

    // A plain object (path -> bytes).
    if !files.is_object() || files.is_null() || files.is_undefined() {
        return Err(malformed());
    }
    let keys = js_sys::Reflect::own_keys(files).map_err(|_| malformed())?;
    let mut out = Vec::with_capacity(keys.length() as usize);
    for i in 0..keys.length() {
        let key_js: JsValue = keys.get(i);
        let key = key_js.as_string().ok_or_else(bad_key)?;
        let value = js_sys::Reflect::get(files, &key_js).map_err(|_| malformed())?;
        out.push((key, to_bytes(&value)?));
    }
    Ok(out)
}

/// One profile-file payload (Uint8Array or ArrayBuffer) to Vec<u8>.
fn to_bytes(value: &JsValue) -> Result<Vec<u8>, JsValue> {
    let malformed = || {
        js_error(
            WEM_ERR_STATE_ERROR,
            "profile file bytes must be Uint8Array or ArrayBuffer",
        )
    };

    if let Some(arr) = value.dyn_ref::<js_sys::Uint8Array>() {
        return Ok(arr.to_vec());
    }
    if value.has_type::<js_sys::ArrayBuffer>() {
        // A Uint8Array view over the buffer (same bytes; owned copy out).
        let global: JsValue = js_sys::global().into();
        let ctor = js_sys::Reflect::get(&global, &JsValue::from_str("Uint8Array"))
            .map_err(|_| malformed())?;
        let ctor = ctor.dyn_ref::<js_sys::Function>().ok_or_else(malformed)?;
        let args = js_sys::Array::new();
        args.push(value);
        let view = js_sys::Reflect::construct(ctor, &args).map_err(|_| malformed())?;
        return Ok(view
            .dyn_ref::<js_sys::Uint8Array>()
            .ok_or_else(malformed)?
            .to_vec());
    }
    Err(malformed())
}

// ---------------------------------------------------------------------------
// Shared result objects
// ---------------------------------------------------------------------------

/// The kernel EncodeStats as a JS object (camelCase).
fn stats_object(stats: &wem_core::EncodeStats) -> JsValue {
    let obj = js_sys::Object::new();
    set(
        &obj,
        "pcmFrames",
        JsValue::from_f64(stats.pcm_frames as f64),
    );
    set(&obj, "channels", JsValue::from_f64(stats.channels as f64));
    set(
        &obj,
        "audioPackets",
        JsValue::from_f64(stats.audio_packets as f64),
    );
    set(
        &obj,
        "shortPackets",
        JsValue::from_f64(stats.short_packets as f64),
    );
    set(
        &obj,
        "longPackets",
        JsValue::from_f64(stats.long_packets as f64),
    );
    set(&obj, "bytes", JsValue::from_f64(stats.bytes as f64));
    set(
        &obj,
        "metadataSource",
        JsValue::from_str(&stats.metadata_source),
    );
    JsValue::from(obj)
}

/// One encoded container (one-shot or terminal streaming) as a JS object:
/// the WEM bytes plus the terminal summary.
fn encode_result_object(result: &wem_core::EncodeResult) -> JsValue {
    let obj = js_sys::Object::new();
    let data: Vec<u8> = result.data.clone();
    set(&obj, "data", JsValue::from(data));
    set(
        &obj,
        "totalLen",
        JsValue::from_f64(result.data.len() as f64),
    );
    set(&obj, "sha256Hex", JsValue::from_str(&result.sha256()));
    set(&obj, "stats", stats_object(&result.stats));
    JsValue::from(obj)
}

// ---------------------------------------------------------------------------
// WAV input (kernel parser: signed-16 PCM only)
// ---------------------------------------------------------------------------

/// Parse one signed-16 PCM WAV from bytes (the kernel's `parse_pcm16`;
/// the JS layer never re-implements container parsing).
///
/// Returns `{ sampleRate, channels, frames, pcm }` with `pcm` the
/// interleaved little-endian signed-16 PCM bytes (the streaming wire form).
#[wasm_bindgen]
pub fn wem_parse_wav(wav: &[u8]) -> Result<JsValue, JsValue> {
    let wav16 = parse_pcm16(wav).map_err(|error| reject(&error))?;
    let obj = js_sys::Object::new();
    set(
        &obj,
        "sampleRate",
        JsValue::from_f64(wav16.sample_rate() as f64),
    );
    set(&obj, "channels", JsValue::from_f64(wav16.channels() as f64));
    set(&obj, "frames", JsValue::from_f64(wav16.frames() as f64));
    set(&obj, "pcm", JsValue::from(wav16.interleaved_le_bytes()));
    Ok(JsValue::from(obj))
}

// ---------------------------------------------------------------------------
// One-shot encoder handle (shareable)
// ---------------------------------------------------------------------------

/// One profile-resolved, shareable encoder built entirely from bytes
/// (the wasm mirror of `wem_encoder_new`; the profile bundle travels as
/// bytes — the kernel verifies every resource's SHA-256 on load and
/// selects the requested profile).
#[wasm_bindgen]
pub struct WemEncoder {
    encoder: Encoder,
}

#[wasm_bindgen]
impl WemEncoder {
    /// Init from one in-memory profile bundle.
    ///
    /// - `profile_name`: the profile key in `index.json`;
    /// - `profile_index`: the raw `index.json` bytes;
    /// - `files`: a `Map`/object of profiles-dir-relative path -> bytes
    ///   (e.g. `"wwise2013-6ch-44100/manifest.json"`,
    ///   `"wwise2013-6ch-44100/vorbis/setup.bin"`).
    ///
    /// Throws `WEM_ERR_PROFILE_NOT_FOUND` / `WEM_ERR_STATE_ERROR` /
    /// `WEM_ERR_INTERNAL` on malformed or failing bundles.
    #[wasm_bindgen(constructor)]
    pub fn new(profile_name: &str, profile_index: &[u8], files: JsValue) -> Result<Self, JsValue> {
        let files = collect_files(&files)?;
        let encoder = Encoder::from_profile_bytes_named(profile_name, profile_index, files)
            .map_err(|error| reject(&error))?;
        Ok(Self { encoder })
    }

    /// Encode one interleaved signed-16 PCM buffer (the wasm mirror of
    /// `wem_encoder_encode`; the container bytes come back inline as
    /// `{ data, totalLen, sha256Hex, stats }` instead of via the write
    /// callback — callback-style output is the C ABI's way of dodging
    /// container size limits; a wasm memory transfer is the equivalent).
    pub fn encode_pcm16_interleaved(&self, pcm: &[u8]) -> Result<JsValue, JsValue> {
        let profile = self.encoder.profile();
        let pcm16 =
            Pcm16::from_interleaved_le(profile.sample_rate(), profile.channels() as usize, pcm)
                .map_err(|error| reject(&error))?;
        let result = self
            .encoder
            .encode_pcm(&pcm16)
            .map_err(|error| reject(&error))?;
        Ok(encode_result_object(&result))
    }

    /// The selected profile identity + geometry
    /// (`{ name, setupSha256, channels, sampleRate }`).
    pub fn profile_info(&self) -> JsValue {
        let profile = self.encoder.profile();
        let obj = js_sys::Object::new();
        set(&obj, "name", JsValue::from_str(profile.name()));
        set(
            &obj,
            "setupSha256",
            JsValue::from_str(profile.setup_sha256()),
        );
        set(
            &obj,
            "channels",
            JsValue::from_f64(profile.channels() as f64),
        );
        set(
            &obj,
            "sampleRate",
            JsValue::from_f64(profile.sample_rate() as f64),
        );
        JsValue::from(obj)
    }

    /// Release the handle.
    pub fn free(self) {}
}

// ---------------------------------------------------------------------------
// Streaming session (Init -> push* -> Finish -> free)
// ---------------------------------------------------------------------------

/// One streaming encode session built entirely from bytes
/// (the wasm mirror of `wem_session_new`; single-threaded ownership,
/// matching the C ABI contract).
#[wasm_bindgen]
pub struct WemSession {
    session: StreamSession,
}

#[wasm_bindgen]
impl WemSession {
    /// Init the session on one profile carried as bytes
    /// (`StreamSession::for_profile_ref_bytes`).
    ///
    /// - `profile_index` / `files`: as in [`WemEncoder::new`];
    /// - `setup_sha256`: the hard identity assertion (must equal the
    ///   bundle's setup SHA-256, lowercase hex — see
    ///   [`WemEncoder::profile_info`] `setupSha256`);
    /// - `profile_name`: the soft name cross-check; `""` skips it.
    ///
    /// Throws on `PROFILE_NOT_FOUND` (digest mismatch / unknown),
    /// `STATE_ERROR` (name mismatch, malformed bundle), or `INTERNAL`.
    #[wasm_bindgen(constructor)]
    pub fn new(
        profile_index: &[u8],
        files: JsValue,
        setup_sha256: &str,
        profile_name: &str,
    ) -> Result<Self, JsValue> {
        let files = collect_files(&files)?;
        let ref_ = if profile_name.is_empty() {
            ProfileRef {
                setup_sha256: setup_sha256.to_string(),
                name: None,
            }
        } else {
            ProfileRef::with_name(setup_sha256, profile_name)
        };
        let session = StreamSession::for_profile_ref_bytes(&ref_, profile_index, files)
            .map_err(|error| reject(&error))?;
        Ok(Self { session })
    }

    /// Push one chunk of interleaved little-endian signed-16 PCM bytes.
    ///
    /// Returns the packets that just completed, in reply order (the first
    /// packet ever returned is the setup packet, then audio packets in
    /// encoding order). Chunk boundaries never affect the output bytes.
    pub fn push(&mut self, pcm: &[u8]) -> Result<JsValue, JsValue> {
        let packets = self
            .session
            .push_pcm_chunk(pcm)
            .map_err(|error| reject(&error))?;
        let arr = js_sys::Array::new();
        for packet in packets {
            let bytes: Vec<u8> = packet.data;
            arr.push(&JsValue::from(bytes));
        }
        Ok(JsValue::from(arr))
    }

    /// Finish the stream: complete the encode and return the terminal
    /// container (`{ data, totalLen, sha256Hex, stats }`).
    ///
    /// Terminal: after this call (success or error) the session must be
    /// released, not reused.
    pub fn finish(&mut self) -> Result<JsValue, JsValue> {
        let result = self.session.finish().map_err(|error| reject(&error))?;
        Ok(encode_result_object(&result))
    }

    /// The PCM frame count accumulated so far (streaming observability).
    pub fn pcm_frames(&self) -> i32 {
        self.session.pcm_frames() as i32
    }

    /// Release the session (safe before or after `finish`).
    pub fn free(self) {}
}

#[cfg(test)]
mod tests {
    //! The error table is part of the cross-language contract; pin it here
    //! (the C ABI crate carries the same test over numeric discriminants).

    use super::*;

    #[test]
    fn error_codes_follow_the_wem_h_table() {
        assert_eq!(
            error_code(&EncoderError::ProfileNotFound {
                requested: "x".into()
            }),
            WEM_ERR_PROFILE_NOT_FOUND
        );
        assert_eq!(
            error_code(&EncoderError::StateError {
                message: "x".into()
            }),
            WEM_ERR_STATE_ERROR
        );
        assert_eq!(
            error_code(&EncoderError::GeometryMismatch {
                message: "x".into()
            }),
            WEM_ERR_GEOMETRY_MISMATCH
        );
        assert_eq!(
            error_code(&EncoderError::InputTooShort { want: 4096, got: 0 }),
            WEM_ERR_INPUT_TOO_SHORT
        );
        assert_eq!(
            error_code(&EncoderError::FormatUnsupported {
                message: "x".into()
            }),
            WEM_ERR_FORMAT_UNSUPPORTED
        );
        assert_eq!(
            error_code(&EncoderError::Internal(
                wem_core::InternalError::Invariant { message: "x" }
            )),
            WEM_ERR_INTERNAL
        );
    }
}
