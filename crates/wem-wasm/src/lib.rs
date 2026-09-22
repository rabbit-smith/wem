//! wem-wasm — the wasm-bindgen shell of the WEM encoder kernel.
//!
//! A **parallel language shell** over the same kernel interface as the C ABI
//! (`include/wem.h`), for browser and web-worker use (crates/AGENTS.md,
//! "C ABI surface"): same lifecycle, same error classes, same bytes; this
//! shell owns no numerics and no profile logic.
//!
//! # Profile selection (ABI revision 3)
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
//! # Shell mapping (1:1 with include/wem.h)
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
//!   terminal container bytes and the statistics observed while assembling
//!   them — a JS `Uint8Array` carries its own length, so no length and no
//!   digest of it are handed back beside it.
//! * **Decoding** (`wem_decoder_*`, include/wem.h section 5):
//!   [`WemDecoder`] — Init -> push* -> Finish -> free, the mirror of the
//!   encode session with the data direction reversed. There is no profile
//!   selection: a WEM is self-describing, and the geometry arrives on the step
//!   that parses the container's setup packet. Each step's output is returned
//!   (`header` / `pcm`), the way the encode session returns its packets, and a
//!   refusal travels on the step beside the samples that step produced —
//!   exactly the order in which the C ABI delivers them (`pcm_cb`, then the
//!   return code).
//! * **Error codes**: every fallible export throws a JS `Error` whose
//!   `code` property (and message prefix) is the stable `WEM_ERR_*` string
//!   of the C ABI error table — append-only, never renumbered.
//!
//! # Panics
//!
//! `wasm32-unknown-unknown` does not unwind, so this shell cannot catch a
//! panic and has nothing to return in its place: a panic becomes a trap
//! (`unreachable`), which terminates the wasm instance. The caller receives no
//! `WEM_ERR_*` code, no container bytes and nothing to retry — the module must
//! be re-instantiated. That is not the web equivalent of the C ABI's
//! `WEM_ERR_INTERNAL` boundary: the C ABI and the PyO3 shell catch the panic
//! and hand back a value, and here there is no value to hand back.
//!
//! What follows is therefore a requirement on the kernel, not a promise this
//! shell can keep: with no way to convert a panic into a typed rejection,
//! every input-derived path reachable from here must be provably panic-free.
//! Malformed input is answered by the typed errors of the table above; a panic
//! on such a path is a defect whose only outcome in this shell is a dead
//! instance, and it is fixed in the kernel rather than papered over here.
//!
//! # No DOM
//!
//! Only `wasm-bindgen` / `js-sys` are used; nothing here touches the DOM,
//! so the shell runs unchanged in browsers, web workers, and Node.

use wasm_bindgen::prelude::*;

use wem_core::decoder::{DecodeSession, DecodeStep};
use wem_core::encoder::{Encoder, Pcm16};
use wem_core::error::{DecoderError, EncoderError};
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
/// The decode surface's malformed-input class (include/wem.h section 2): never
/// returned by an encode entry.
const WEM_ERR_INPUT_MALFORMED: &str = "WEM_ERR_INPUT_MALFORMED";

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

/// The stable error code of one kernel *decode* failure (include/wem.h
/// section 5), the same four classes `crates/wem-capi` maps `DecoderError`
/// onto. The match has no `_` arm on purpose: a variant added to
/// `DecoderError` fails this build until its class is written.
fn decoder_error_code(error: &DecoderError) -> &'static str {
    match error {
        DecoderError::Container(_)
        | DecoderError::Setup { .. }
        | DecoderError::SetupPadding { .. }
        | DecoderError::Truncated { .. }
        | DecoderError::MissingSetup { .. }
        | DecoderError::BlockSizeMismatch { .. }
        | DecoderError::Packet { .. }
        | DecoderError::ResidueBitstreamDefect { .. }
        | DecoderError::FrameCountMismatch { .. } => WEM_ERR_INPUT_MALFORMED,
        DecoderError::NotWwiseVorbis { .. }
        | DecoderError::ConfigurationUnsupported { .. }
        | DecoderError::SetupNotCarried { .. } => WEM_ERR_FORMAT_UNSUPPORTED,
        DecoderError::StateError { .. } => WEM_ERR_STATE_ERROR,
        DecoderError::Floor1 { .. } | DecoderError::Internal(_) => WEM_ERR_INTERNAL,
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
/// This is the whole caller-facing profile selection of ABI revision 3: how
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
    JsValue::from(obj)
}

/// One encoded container (one-shot or terminal streaming) as a JS object:
/// the WEM bytes and the statistics observed while assembling them.
///
/// Nothing derived from the bytes travels beside them: a `Uint8Array` knows
/// its own `byteLength`, and a digest is the caller's to compute from it.
fn encode_result_object(result: &wem_core::EncodeResult) -> JsValue {
    let obj = js_sys::Object::new();
    let data: Vec<u8> = result.data.clone();
    set(&obj, "data", JsValue::from(data));
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
    // The WAV lends its bytes; JS must own them, so this hands over a copy
    // as the `Uint8Array` payload (the `Vec` is moved into the JS value, not
    // duplicated Rust-side).
    set(
        &obj,
        "pcm",
        JsValue::from(wav16.interleaved_le_bytes().to_vec()),
    );
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
    /// `{ data, stats }` instead of via the write
    /// callback — callback-style output is the C ABI's way of dodging
    /// container size limits; a wasm memory transfer is the equivalent).
    ///
    /// The PCM carries no geometry of its own: it is read at the selection's
    /// sample rate and channel count, exactly like the C ABI entry whose
    /// interface requires the client to pass matching PCM. Use
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
/// single-threaded ownership, matching the C ABI surface), selected by one
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
    /// container (`{ data, stats }`).
    ///
    /// Terminal: after this call (success or error) the session must be
    /// released, not reused.
    pub fn finish(&mut self) -> Result<JsValue, JsValue> {
        let result = self.session.finish().map_err(|error| reject(&error))?;
        Ok(encode_result_object(&result))
    }

    /// The PCM frame count accumulated so far (streaming observability).
    ///
    /// Returned as a JS number (`f64`), the type every other count in this
    /// shell uses, and exact for every integer below 2^53. The count cannot
    /// approach that: the streaming session records per-frame state (one
    /// `i64` mode and one `FramePlan` per frame), so a wasm32 instance's
    /// address space caps the count at a few hundred million frames — 8 bytes
    /// per frame alone reaches 4 GiB long before 2^53.
    ///
    /// That cap is an argument from the session's own bookkeeping, not an
    /// invariant this signature can enforce, which is why the surface is the
    /// widest exactly-representable JS integer rather than a narrowing cast
    /// that merely assumes the bound holds.
    pub fn pcm_frames(&self) -> f64 {
        self.session.pcm_frames() as f64
    }

    /// Release the session (safe before or after `finish`).
    pub fn free(self) {}
}

// ---------------------------------------------------------------------------
// Streaming decode session (Init -> push* -> Finish -> free)
// ---------------------------------------------------------------------------

/// One decode step as a JS object
/// (`{ header, channels, frames, pcm, error }`).
///
/// * `header` — the one-time announcement this step resolved
///   (`{ channels, sampleRate, setup }`), `null` on every other step. It is the
///   mirror of the C ABI's `header_cb`, which fires exactly once before any
///   PCM, and it is the only place the geometry appears.
/// * `channels` / `frames` — the interleave width (`0` before the header is
///   known) and the frame count of `pcm`.
/// * `pcm` — this step's samples as one `Float32Array`, interleaved f32 at ±1.0
///   full scale. Bounded by the bytes this push carried, never by the stream.
///   (The C ABI hands the same samples over in fixed-size `pcm_cb` blocks
///   because a bare pointer has no length; a typed array carries its own, so
///   the shell returns the step's samples as one value.)
/// * `error` — `null` when the step consumed everything it was given, and
///   otherwise `{ code, message }`: the class include/wem.h section 5 documents
///   for that refusal plus the kernel's own diagnostic. The refusal travels on
///   the step because the C ABI delivers the samples the earlier packets
///   completed *and then* returns the code; throwing first would drop the
///   prefix the kernel delivered.
fn decode_step_object(step: DecodeStep) -> JsValue {
    let obj = js_sys::Object::new();
    match step.header.as_ref() {
        Some(header) => {
            let announced = js_sys::Object::new();
            set(
                &announced,
                "channels",
                JsValue::from_f64(f64::from(header.channels)),
            );
            set(
                &announced,
                "sampleRate",
                JsValue::from_f64(f64::from(header.sample_rate)),
            );
            // The declared frame count, beside the geometry. The C ABI's
            // WemHeaderCb announces it and the PyO3 shell exposes it; this
            // shell omitted it, which made the same session report a different
            // header depending on the language that opened it. A frame count is
            // far below 2^53, so the f64 JS number is exact.
            set(
                &announced,
                "totalFrames",
                JsValue::from_f64(header.total_frames as f64),
            );
            set(
                &announced,
                "setup",
                JsValue::from(header.setup_packet.clone()),
            );
            set(&obj, "header", JsValue::from(announced));
        }
        None => set(&obj, "header", JsValue::NULL),
    }
    set(
        &obj,
        "channels",
        JsValue::from_f64(f64::from(step.channels)),
    );
    set(&obj, "frames", JsValue::from_f64(step.frames() as f64));
    set(
        &obj,
        "pcm",
        JsValue::from(js_sys::Float32Array::from(step.pcm.as_slice())),
    );
    match step.outcome {
        Ok(()) => set(&obj, "error", JsValue::NULL),
        Err(error) => {
            let refusal = js_sys::Object::new();
            set(
                &refusal,
                "code",
                JsValue::from_str(decoder_error_code(&error)),
            );
            set(&refusal, "message", JsValue::from_str(&error.to_string()));
            set(&obj, "error", JsValue::from(refusal));
        }
    }
    JsValue::from(obj)
}

/// Whether the step's PCM can be split into complete interleaved frames.
fn decode_step_pcm_is_valid(step: &DecodeStep) -> bool {
    step.pcm.is_empty()
        || (step.channels != 0 && step.pcm.len().is_multiple_of(step.channels as usize))
}

/// One streaming decode session (the wasm mirror of `wem_decoder_new` /
/// `wem_decoder_push` / `wem_decoder_finish`; single-threaded ownership,
/// matching the C ABI surface).
///
/// There is no profile selection: a WEM is self-describing, the configuration
/// is resolved from the container's own geometry, and a container whose setup
/// packet is not the one this build carries is refused with
/// `WEM_ERR_FORMAT_UNSUPPORTED`.
#[wasm_bindgen]
pub struct WemDecoder {
    session: DecodeSession,
    /// `Finish` has run: the session is released, not reused, whatever the call
    /// returned (include/wem.h section 5).
    finished: bool,
    /// A defect killed this handle: every later call throws
    /// `WEM_ERR_STATE_ERROR` without touching the kernel again.
    failed: bool,
}

impl Default for WemDecoder {
    /// The same Init a caller writes as `new WemDecoder()`; the kernel's own
    /// `DecodeSession` carries the same impl.
    fn default() -> Self {
        Self::new()
    }
}

#[wasm_bindgen]
impl WemDecoder {
    /// Init a decode session (`wem_decoder_new`). There is no selection
    /// argument: the geometry arrives on the step that parses the container's
    /// setup packet.
    #[wasm_bindgen(constructor)]
    pub fn new() -> Self {
        Self {
            session: DecodeSession::new(),
            finished: false,
            failed: false,
        }
    }

    /// Push one chunk of WEM bytes (`wem_decoder_push`) and return what it
    /// completed.
    ///
    /// Chunk boundaries never affect the emitted samples; an empty chunk is a
    /// no-op. Throws `WEM_ERR_STATE_ERROR` on a call outside the lifecycle.
    pub fn push(&mut self, data: &[u8]) -> Result<JsValue, JsValue> {
        if self.finished {
            return Err(js_error(
                WEM_ERR_STATE_ERROR,
                "chunks are not allowed after Finish",
            ));
        }
        if self.failed {
            return Err(js_error(
                WEM_ERR_STATE_ERROR,
                "the decoder is unusable: a kernel defect already killed it",
            ));
        }
        let step = self.session.push_bytes(data);
        self.deliver(step)
    }

    /// Mark the end of the WEM bytes and complete the decode
    /// (`wem_decoder_finish`), returning the last frames.
    ///
    /// Terminal whatever it returns: after this call the session is released,
    /// not reused. On success the session has delivered exactly the frame count
    /// the container declares.
    pub fn finish(&mut self) -> Result<JsValue, JsValue> {
        if self.finished {
            return Err(js_error(
                WEM_ERR_STATE_ERROR,
                "Finish completes exactly once",
            ));
        }
        if self.failed {
            return Err(js_error(
                WEM_ERR_STATE_ERROR,
                "the decoder is unusable: a kernel defect already killed it",
            ));
        }
        let step = self.session.finish();
        // Terminal whatever it returned, like the C ABI's `wem_decoder_finish`.
        self.finished = true;
        self.deliver(step)
    }

    /// Release the session (safe before or after `finish`).
    pub fn free(self) {}

    /// Report one step, applying the C ABI's rule for a defect: its output is
    /// not delivered, it throws `WEM_ERR_INTERNAL`, and the handle is terminal.
    /// Every other outcome is returned on the step, which is what lets the
    /// samples a refused step completed still reach the caller.
    fn deliver(&mut self, step: DecodeStep) -> Result<JsValue, JsValue> {
        if let Err(error) = step.outcome.as_ref() {
            if decoder_error_code(error) == WEM_ERR_INTERNAL {
                self.failed = true;
                return Err(js_error(WEM_ERR_INTERNAL, &error.to_string()));
            }
        }
        if !decode_step_pcm_is_valid(&step) {
            // PCM without whole-frame geometry cannot be interpreted, and
            // handing it over under a guessed interleave would be worse than
            // reporting the invariant (the C ABI's `deliver_decode_step`
            // refuses it too).
            self.failed = true;
            return Err(js_error(
                WEM_ERR_INTERNAL,
                "kernel defect: a decode step delivered PCM without whole-frame geometry",
            ));
        }
        Ok(decode_step_object(step))
    }
}

#[cfg(test)]
mod tests {
    //! The error table is part of the cross-language interface; pin it here
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
    fn decoder_error_codes_follow_the_wem_h_table() {
        // One representative per row of include/wem.h section 5, the same
        // mapping `crates/wem-capi` applies; the exhaustive match in
        // `decoder_error_code` is what makes a new variant a build failure
        // until its class is written.
        for (error, code) in [
            (
                DecoderError::SetupPadding {
                    end_bit: 0,
                    total_bits: 8,
                    pad_bits: 8,
                    pad_value: 1,
                },
                WEM_ERR_INPUT_MALFORMED,
            ),
            (
                DecoderError::Truncated {
                    stream_offset: 0,
                    need: "a packet payload",
                },
                WEM_ERR_INPUT_MALFORMED,
            ),
            (
                DecoderError::BlockSizeMismatch {
                    container: [256, 2048],
                    profile: [128, 1024],
                },
                WEM_ERR_INPUT_MALFORMED,
            ),
            (
                DecoderError::MissingSetup { data_size: 0 },
                WEM_ERR_INPUT_MALFORMED,
            ),
            (
                DecoderError::FrameCountMismatch {
                    declared: 2,
                    synthesized: 1,
                },
                WEM_ERR_INPUT_MALFORMED,
            ),
            (
                DecoderError::NotWwiseVorbis { format_tag: 0 },
                WEM_ERR_FORMAT_UNSUPPORTED,
            ),
            (
                DecoderError::StateError {
                    message: "x".into(),
                },
                WEM_ERR_STATE_ERROR,
            ),
            (
                DecoderError::Internal(wem_core::InternalError::Invariant { message: "x" }),
                WEM_ERR_INTERNAL,
            ),
        ] {
            assert_eq!(decoder_error_code(&error), code, "{error}");
        }
    }

    #[test]
    fn incomplete_injected_decode_pcm_is_rejected() {
        for (pcm, channels) in [(vec![0.0], 0), (vec![0.0, 1.0, 2.0], 2)] {
            let step = DecodeStep {
                header: None,
                pcm,
                channels,
                outcome: Ok(()),
            };
            assert!(
                !decode_step_pcm_is_valid(&step),
                "PCM must have nonzero geometry and a whole number of frames"
            );
        }
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

    /// The frame count is a JS number, not a narrowed int: the signature is
    /// checked here so narrowing it back to `i32` fails this build, and the
    /// conversion it performs is checked to be lossless across the whole range
    /// a JS number represents integers exactly.
    #[test]
    fn pcm_frames_is_a_js_number_and_cannot_truncate() {
        fn returns_js_number(_: fn(&WemSession) -> f64) {}
        returns_js_number(WemSession::pcm_frames);

        // Every count below 2^53 survives the conversion exactly, including
        // the first one an i32 could not have carried.
        for count in [
            0i64,
            1,
            4096,
            i64::from(i32::MAX),
            i64::from(i32::MAX) + 1,
            (1i64 << 53) - 1,
        ] {
            assert_eq!(
                count as f64 as i64, count,
                "{count} is below 2^53 and must be exact as a JS number"
            );
        }
    }
}
