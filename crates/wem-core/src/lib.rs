//! WEM encoder orchestration layer.
//!
//! This is the only crate allowed to assemble profiles, analysis, packet
//! encoding, and containers into the complete WAV-to-WEM use case
//! (docs/reference/architecture.md). Mirrors the Python `wwise_wem/application`
//! package plus the package-root API shell.
//!
//! Public surface:
//! * [`encoder::Encoder`] — one-shot PCM-to-WEM encode
//!   (Python `application/encoder.py`); the caller-facing entry is
//!   [`encoder::Encoder::new`] over a structured [`WwiseProfile`]
//! * [`decoder::DecodeSession`] — the streaming decode lifecycle
//!   (Init -> chunk* -> Finish) with the data direction reversed: WEM bytes
//!   in, interleaved f32 PCM out, exactly `dw_total_pcm_frames` frames
//! * [`decoder::DecodeStep`] / [`decoder::DecodedHeader`] — one decode step's
//!   output and the one-time header announcement
//! * [`WwiseProfile`] / [`WwiseVersion`] — the structured profile selector
//!   (re-exported from `wem-profiles` for the bindings that depend only on
//!   this crate)
//! * [`encoder::Pcm16`] / [`encoder::EncodeResult`] — typed I/O models
//! * [`pack::pack_analysis_frame`] — frame-level floor-fit and packet
//!   diagnostics
//! * [`stream::StreamSession`] — the core streaming lifecycle
//!   (Init -> chunk* -> Finish) with true incremental emission and
//!   bounded input memory; shared by the C ABI, PyO3 and
//!   wasm-bindgen shells
//! * [`stream::StreamPacket`] — one emitted reply packet
//! * [`usecases::wav`] — minimal signed-16 PCM WAV reader

pub mod decoder;
pub mod encoder;
pub mod error;
pub mod pack;
pub mod stream;

pub mod usecases {
    pub mod wav;
}

pub use decoder::{DecodeSession, DecodeStep, DecodedHeader};
pub use encoder::{ContainerPlan, EncodeResult, EncodeStats, Encoder, Pcm16, MIN_PCM_FRAMES};
pub use error::{DecoderError, EncoderError, InternalError};
pub use pack::{pack_analysis_frame, EncodedPacket};
pub use stream::{StreamPacket, StreamSession};
pub use wem_profiles::blob::{profile_tables_blob, BLOB_MAGIC, BLOB_VERSION};
pub use wem_profiles::selection::{WwiseProfile, WwiseVersion};
