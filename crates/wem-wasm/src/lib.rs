//! wem-wasm — the wasm-bindgen shell of the WEM encoder kernel.
//!
//! A **parallel language shell** over the same kernel contract as the C ABI
//! (`include/wem.h`), for browser and web-worker use (crates/AGENTS.md,
//! "C ABI contract"): same lifecycle, same error classes, same bytes; this
//! shell owns no numerics and no profile logic.
//!
//! # Profile selection (ABI revision 2)
//!
//! Every constructor takes the structured selection of `include/wem.h`
//! ("PROFILE SELECTION"): one Wwise generation plus the PCM geometry. The
//! profile bundle is compiled into the wasm module (the kernel's embedded
//! bundle), so nothing is downloaded, fetched or passed in: there is no
//! profile index, manifest, resource bytes, profile name or data directory
//! on this surface, and no entry touches the filesystem.
//!
//! * `version` — a stable `WemVersion` code (`0` = Wwise 2013), the label
//!   `"2013"`, the full generation `"2013.2"`, or `null`/`undefined` for
//!   **auto-selection**: the unique installed generation whose configuration
//!   satisfies the given geometry (in the browser flow, the geometry of the
//!   WAV the user picked).
//! * `channels` / `sample_rate` — the PCM geometry to encode.
//!
//! Failures map 1:1 onto the `include/wem.h` error table: an unknown version
//! code is `WEM_ERR_FORMAT_UNSUPPORTED`, a non-positive geometry is
//! `WEM_ERR_STATE_ERROR`, a selection no compiled configuration satisfies is
//! `WEM_ERR_PROFILE_NOT_FOUND`. A selection is never silently substituted by
//! a default, and an ambiguous auto-selection is rejected rather than
//! resolved by table order.
//!
//! # Contract mapping (1:1 with include/wem.h)
//!
//! * **One-shot** (`wem_encoder_*` / `wem_encode_pcm16_interleaved`):
//!   [`WemEncoder`] — the shareable encoder handle.
//!   [`WemEncoder::encode_pcm16_interleaved`] is the `wem_encoder_encode`
//!   mirror (PCM in, container bytes out); [`WemEncoder::encode_wav`] is the
//!   shell convenience that runs the kernel's WAV parse first, so the
//!   `GEOMETRY_MISMATCH` check stays in the kernel.
//! * **Streaming** (`wem_session_*`): [`WemSession`] — Init -> push* ->
//!   Finish -> free. `push` emits the packets that just completed (seq 0 =
//!   setup packet, then audio packets in encoding order) as a returned array
//!   instead of a C callback — the JS wrapper can translate that into
//!   callbacks; the kernel reply framing is unchanged. `finish` returns the
//!   terminal container bytes plus the `WemMeta` summary (`totalLen`,
//!   `sha256Hex`).
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

use wem_core::encoder::{Encoder, Pcm16};
use wem_core::error::EncoderError;
use wem_core::stream::StreamSession;
use wem_core::usecases::wav::parse_pcm16;
use wem_core::{WwiseProfile, WwiseVersion};

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

// ---------------------------------------------------------------------------
// Structured profile selection (include/wem.h, "PROFILE SELECTION")
// ---------------------------------------------------------------------------

/// A resolution failure carried as (stable error code, kernel detail).
type SelectionFailure = (&'static str, String);

/// Map one kernel encoder failure onto its stable code plus detail.
fn encoder_failure(error: &EncoderError) -> SelectionFailure {
    (error_code(error), error.to_string())
}

/// A constructor's version argument: an explicit generation, or
/// auto-selection from the PCM geometry.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum VersionArg {
    /// A named Wwise generation (`WemVersion`).
    Explicit(WwiseVersion),
    /// No generation named: resolve the one the geometry selects.
    Auto,
}

/// Decode a stable cross-language `WemVersion` code.
///
/// A code outside the table is not a value of this revision and never falls
/// back to a default generation (`WEM_ERR_FORMAT_UNSUPPORTED`).
fn version_from_code(code: u32) -> Result<WwiseVersion, SelectionFailure> {
    WwiseVersion::from_code(code).map_err(|error| (WEM_ERR_FORMAT_UNSUPPORTED, error.to_string()))
}

/// Decode a user-facing version spelling: the label (`"2013"`) or the full
/// generation (`"2013.2"`).
fn version_from_text(text: &str) -> Result<WwiseVersion, SelectionFailure> {
    WwiseVersion::parse(text).map_err(|error| (WEM_ERR_FORMAT_UNSUPPORTED, error.to_string()))
}

/// Decode a numeric version argument (a JS number arrives as `f64`; the
/// ABI's `WemVersion` is a 32-bit code).
fn version_from_number(value: f64) -> Result<WwiseVersion, SelectionFailure> {
    if !value.is_finite() || value.fract() != 0.0 || value < 0.0 || value > f64::from(u32::MAX) {
        return Err((
            WEM_ERR_FORMAT_UNSUPPORTED,
            format!("unknown Wwise version code {value}"),
        ));
    }
    version_from_code(value as u32)
}

/// Build the structured selection for one named generation.
///
/// A non-positive geometry is a malformed argument, never a niche of the
/// version table (`WEM_ERR_STATE_ERROR`).
fn selection_from(
    version: WwiseVersion,
    channels: i64,
    sample_rate: i64,
) -> Result<WwiseProfile, SelectionFailure> {
    WwiseProfile::new(version, channels, sample_rate)
        .map_err(|error| (WEM_ERR_STATE_ERROR, error.to_string()))
}

/// Auto-selection: open the unique installed generation whose configuration
/// satisfies the geometry.
///
/// The generation is never guessed. When more than one installed generation
/// satisfies the geometry the selection is ambiguous and rejected with
/// `WEM_ERR_PROFILE_NOT_FOUND`, matching the kernel's rule that an ambiguous
/// selection is not resolved by load order; when none does, the same code
/// reports an unsatisfiable selection.
///
/// Auto-selection reads no profile table of its own: the kernel's own
/// resolution (`open`) is what accepts or rejects each candidate generation,
/// so this helper owns no profile logic.
fn open_auto<T>(
    channels: i64,
    sample_rate: i64,
    open: impl Fn(WwiseProfile) -> Result<T, EncoderError>,
) -> Result<(WwiseProfile, T), SelectionFailure> {
    let mut resolved: Option<(WwiseProfile, T)> = None;
    for version in WwiseVersion::ALL {
        // Geometry validation is generation-independent: a non-positive
        // geometry is the same malformed argument whatever the table holds.
        let selection = selection_from(version, channels, sample_rate)?;
        match open(selection) {
            Ok(opened) => {
                if resolved.is_some() {
                    return Err((
                        WEM_ERR_PROFILE_NOT_FOUND,
                        format!(
                            "Wwise generation is ambiguous for {channels}ch/{sample_rate}Hz; \
                             name one explicitly"
                        ),
                    ));
                }
                resolved = Some((selection, opened));
            }
            // No installed configuration satisfies the geometry for this
            // generation: a normal auto-selection outcome, not a failure.
            Err(EncoderError::ProfileNotFound { .. }) => {}
            Err(error) => return Err(encoder_failure(&error)),
        }
    }
    resolved.ok_or_else(|| {
        (
            WEM_ERR_PROFILE_NOT_FOUND,
            format!("no installed profile satisfies {channels}ch/{sample_rate}Hz"),
        )
    })
}

/// Open one one-shot encoder over a constructor's selection.
fn open_encoder(
    arg: VersionArg,
    channels: i64,
    sample_rate: i64,
) -> Result<(WwiseProfile, Encoder), SelectionFailure> {
    match arg {
        VersionArg::Explicit(version) => {
            let selection = selection_from(version, channels, sample_rate)?;
            let encoder = Encoder::new(selection).map_err(|error| encoder_failure(&error))?;
            Ok((selection, encoder))
        }
        VersionArg::Auto => open_auto(channels, sample_rate, Encoder::new),
    }
}

/// Open one streaming session over a constructor's selection (Init).
fn open_session(
    arg: VersionArg,
    channels: i64,
    sample_rate: i64,
) -> Result<(WwiseProfile, StreamSession), SelectionFailure> {
    match arg {
        VersionArg::Explicit(version) => {
            let selection = selection_from(version, channels, sample_rate)?;
            let session =
                StreamSession::for_selection(selection).map_err(|error| encoder_failure(&error))?;
            Ok((selection, session))
        }
        VersionArg::Auto => open_auto(channels, sample_rate, StreamSession::for_selection),
    }
}

/// Read one JS version argument: a `WemVersion` code, a label/generation
/// string, or `null`/`undefined` for auto-selection.
fn read_version_arg(version: &JsValue) -> Result<VersionArg, JsValue> {
    if version.is_null() || version.is_undefined() {
        return Ok(VersionArg::Auto);
    }
    let decoded = if let Some(number) = version.as_f64() {
        version_from_number(number)
    } else if let Some(text) = version.as_string() {
        version_from_text(&text)
    } else {
        Err((
            WEM_ERR_STATE_ERROR,
            "version must be a Wwise version code (0), a label (\"2013\"), \
             or null for auto-selection"
                .to_string(),
        ))
    };
    decoded
        .map(VersionArg::Explicit)
        .map_err(|(code, detail)| js_error(code, &detail))
}

/// Read one constructor's `(version, channels, sample_rate)` triple and open
/// the encoder it selects.
fn read_encoder(
    version: &JsValue,
    channels: i32,
    sample_rate: i32,
) -> Result<(WwiseProfile, Encoder), JsValue> {
    let arg = read_version_arg(version)?;
    open_encoder(arg, i64::from(channels), i64::from(sample_rate))
        .map_err(|(code, detail)| js_error(code, &detail))
}

/// Read one constructor's `(version, channels, sample_rate)` triple and open
/// the streaming session it selects.
fn read_session(
    version: &JsValue,
    channels: i32,
    sample_rate: i32,
) -> Result<(WwiseProfile, StreamSession), JsValue> {
    let arg = read_version_arg(version)?;
    open_session(arg, i64::from(channels), i64::from(sample_rate))
        .map_err(|(code, detail)| js_error(code, &detail))
}

// ---------------------------------------------------------------------------
// Shared result objects
// ---------------------------------------------------------------------------

/// One resolved profile selection as a JS object
/// (`{ versionCode, version, generation, channels, sampleRate, description }`).
///
/// This is the whole caller-facing profile contract of ABI revision 2: how
/// the kernel stores and addresses the configuration behind it (profile
/// name, resource paths, digests) is internal and stays on the kernel side.
fn selection_object(selection: &WwiseProfile) -> JsValue {
    let obj = js_sys::Object::new();
    set(
        &obj,
        "versionCode",
        JsValue::from_f64(selection.version().code() as f64),
    );
    set(
        &obj,
        "version",
        JsValue::from_str(selection.version().label()),
    );
    set(
        &obj,
        "generation",
        JsValue::from_str(selection.version().generation()),
    );
    set(
        &obj,
        "channels",
        JsValue::from_f64(selection.channels() as f64),
    );
    set(
        &obj,
        "sampleRate",
        JsValue::from_f64(selection.sample_rate() as f64),
    );
    set(
        &obj,
        "description",
        JsValue::from_str(&selection.describe()),
    );
    JsValue::from(obj)
}

/// The compiled-in `WemVersion` table (include/wem.h): every selectable
/// Wwise generation, in stable table order.
#[wasm_bindgen]
pub fn wem_versions() -> JsValue {
    let arr = js_sys::Array::new();
    for version in WwiseVersion::ALL {
        let obj = js_sys::Object::new();
        set(&obj, "code", JsValue::from_f64(version.code() as f64));
        set(&obj, "label", JsValue::from_str(version.label()));
        set(&obj, "generation", JsValue::from_str(version.generation()));
        arr.push(&obj);
    }
    JsValue::from(arr)
}

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
/// This is where a browser caller takes the geometry it passes to a
/// constructor for auto-selection.
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

/// One profile-resolved, shareable encoder (the wasm mirror of
/// `wem_encoder_new`), selected by one structured profile selection against
/// the profile bundle compiled into this module.
#[wasm_bindgen]
pub struct WemEncoder {
    selection: WwiseProfile,
    encoder: Encoder,
}

#[wasm_bindgen]
impl WemEncoder {
    /// Init from one structured profile selection.
    ///
    /// - `version`: `0` (Wwise 2013), `"2013"`, `"2013.2"`, or `null` /
    ///   `undefined` to auto-select the installed generation from the
    ///   geometry;
    /// - `channels` / `sample_rate`: the PCM geometry to encode (> 0).
    ///
    /// Throws `WEM_ERR_FORMAT_UNSUPPORTED` on an unknown version code,
    /// `WEM_ERR_STATE_ERROR` on a non-positive geometry, and
    /// `WEM_ERR_PROFILE_NOT_FOUND` when no compiled configuration satisfies
    /// the selection.
    #[wasm_bindgen(constructor)]
    pub fn new(version: JsValue, channels: i32, sample_rate: i32) -> Result<WemEncoder, JsValue> {
        let (selection, encoder) = read_encoder(&version, channels, sample_rate)?;
        Ok(Self { selection, encoder })
    }

    /// The resolved selection
    /// (`{ versionCode, version, generation, channels, sampleRate, description }`)
    /// — which generation auto-selection picked, and the geometry it encodes.
    pub fn selection(&self) -> JsValue {
        selection_object(&self.selection)
    }

    /// Encode one interleaved signed-16 PCM buffer (the wasm mirror of
    /// `wem_encoder_encode`; the container bytes come back inline as
    /// `{ data, totalLen, sha256Hex, stats }` instead of via the write
    /// callback — callback-style output is the C ABI's way of dodging
    /// container size limits; a wasm memory transfer is the equivalent).
    ///
    /// The PCM carries no geometry of its own: it is read at the selection's
    /// sample rate and channel count, exactly like the C ABI entry whose
    /// contract requires the client to pass matching PCM. Use
    /// [`WemEncoder::encode_wav`] for a WAV, whose own geometry the kernel
    /// then cross-checks (`WEM_ERR_GEOMETRY_MISMATCH`).
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

    /// Parse one signed-16 PCM WAV and encode it in one call (kernel
    /// `parse_pcm16` + `encode_pcm`).
    ///
    /// Throws `WEM_ERR_FORMAT_UNSUPPORTED` when the WAV is not signed-16 PCM
    /// and `WEM_ERR_GEOMETRY_MISMATCH` when its geometry disagrees with the
    /// selection — the check is the kernel's, never re-implemented here.
    pub fn encode_wav(&self, wav: &[u8]) -> Result<JsValue, JsValue> {
        let wav16 = parse_pcm16(wav).map_err(|error| reject(&error))?;
        let pcm16 = wav16.to_pcm16().map_err(|error| reject(&error))?;
        let result = self
            .encoder
            .encode_pcm(&pcm16)
            .map_err(|error| reject(&error))?;
        Ok(encode_result_object(&result))
    }

    /// Release the handle.
    pub fn free(self) {}
}

// ---------------------------------------------------------------------------
// Streaming session (Init -> push* -> Finish -> free)
// ---------------------------------------------------------------------------

/// One streaming encode session (the wasm mirror of `wem_session_new`;
/// single-threaded ownership, matching the C ABI contract), selected by one
/// structured profile selection against the compiled-in profile bundle.
#[wasm_bindgen]
pub struct WemSession {
    selection: WwiseProfile,
    session: StreamSession,
}

#[wasm_bindgen]
impl WemSession {
    /// Init the session on one structured profile selection; `version`,
    /// `channels` and `sample_rate` are as in [`WemEncoder::new`].
    ///
    /// Throws `WEM_ERR_FORMAT_UNSUPPORTED` on an unknown version code,
    /// `WEM_ERR_STATE_ERROR` on a non-positive geometry, and
    /// `WEM_ERR_PROFILE_NOT_FOUND` when no compiled configuration satisfies
    /// the selection.
    #[wasm_bindgen(constructor)]
    pub fn new(version: JsValue, channels: i32, sample_rate: i32) -> Result<WemSession, JsValue> {
        let (selection, session) = read_session(&version, channels, sample_rate)?;
        Ok(Self { selection, session })
    }

    /// The resolved selection — see [`WemEncoder::selection`].
    pub fn selection(&self) -> JsValue {
        selection_object(&self.selection)
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
    //! (the C ABI crate carries the same test over numeric discriminants),
    //! together with the selection mapping of `include/wem.h`
    //! ("PROFILE SELECTION").

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

    #[test]
    fn version_codes_and_labels_follow_the_wem_version_table() {
        assert_eq!(version_from_code(0), Ok(WwiseVersion::Wwise2013));
        assert_eq!(
            version_from_code(1),
            Err((
                WEM_ERR_FORMAT_UNSUPPORTED,
                "unknown Wwise version code 1".to_string()
            ))
        );
        assert_eq!(version_from_text("2013"), Ok(WwiseVersion::Wwise2013));
        assert_eq!(version_from_text("2013.2"), Ok(WwiseVersion::Wwise2013));
        assert_eq!(
            version_from_text("2011").map_err(|(code, _)| code),
            Err(WEM_ERR_FORMAT_UNSUPPORTED)
        );
        // A JS number arrives as f64: only exact 32-bit codes are in domain.
        assert_eq!(version_from_number(0.0), Ok(WwiseVersion::Wwise2013));
        for value in [-1.0, 1.5, 2013.0, f64::NAN, f64::INFINITY] {
            assert_eq!(
                version_from_number(value).map_err(|(code, _)| code),
                Err(WEM_ERR_FORMAT_UNSUPPORTED),
                "value {value} must not name a generation"
            );
        }
    }

    #[test]
    fn selection_rejects_non_positive_geometry_as_state_error() {
        for (channels, sample_rate) in [(0, 44100), (6, 0), (-2, 48000)] {
            let failure = selection_from(WwiseVersion::Wwise2013, channels, sample_rate)
                .expect_err("non-positive geometry is a malformed argument");
            assert_eq!(failure.0, WEM_ERR_STATE_ERROR);
            assert_eq!(
                failure.1,
                "selected channels and sample rate must be positive"
            );
        }
        let selection = selection_from(WwiseVersion::Wwise2013, 6, 44100)
            .expect("positive geometry is in domain");
        assert_eq!(selection.describe(), "6ch/44100Hz/2013");
        assert_eq!(selection.version().code(), 0);
    }

    #[test]
    fn unsatisfiable_selection_is_profile_not_found() {
        // 2ch/44100 is not a compiled configuration (the installed pair is
        // 6ch/44100 and 2ch/48000).
        let selection = selection_from(WwiseVersion::Wwise2013, 2, 44100)
            .expect("positive geometry is in domain");
        let error = Encoder::new(selection)
            .map(|_| ())
            .expect_err("2ch/44100 is not installed");
        assert_eq!(error_code(&error), WEM_ERR_PROFILE_NOT_FOUND);

        let failure = open_encoder(VersionArg::Auto, 8, 44100)
            .map(|_| ())
            .expect_err("no installed generation satisfies 8ch/44100");
        assert_eq!(failure.0, WEM_ERR_PROFILE_NOT_FOUND);
    }

    #[test]
    fn auto_selection_resolves_the_compiled_geometry() {
        let (selection, _encoder) = open_encoder(VersionArg::Auto, 6, 44100)
            .expect("6ch/44100 is the compiled default configuration");
        assert_eq!(selection.describe(), "6ch/44100Hz/2013");

        let (selection, _session) = open_session(VersionArg::Auto, 2, 48000)
            .expect("2ch/48000 is a compiled configuration");
        assert_eq!(selection.describe(), "2ch/48000Hz/2013");
        assert_eq!(selection.version().generation(), "2013.2");
    }

    #[test]
    fn explicit_selection_opens_the_named_generation() {
        let (selection, _encoder) =
            open_encoder(VersionArg::Explicit(WwiseVersion::Wwise2013), 6, 44100)
                .expect("explicit 2013 selection resolves");
        assert_eq!(selection.version().label(), "2013");
    }
}
