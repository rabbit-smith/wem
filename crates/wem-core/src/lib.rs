//! WEM encoder orchestration layer.
//!
//! This is the only crate allowed to assemble profiles, analysis, packet
//! encoding, and containers into the complete WAV-to-WEM use case
//! (docs/architecture.md). Mirrors the Python `wwise_wem/application`
//! package plus the package-root API shell.
//!
//! Public surface:
//! * [`encoder::Encoder`] — one-shot PCM-to-WEM encode
//!   (Python `application/encoder.py`)
//! * [`encoder::Pcm16`] / [`encoder::EncodeResult`] — typed I/O models
//! * [`pack::pack_analysis_frame`] — frame-level floor-fit + packet
//!   assembly (Python `pack_analysis_frame`, relocated here)
//! * [`stream::StreamSession`] — the wwise.v1 streaming lifecycle
//!   (Init -> chunk* -> Finish), shared by the future gRPC and PyO3 shells
//! * [`usecases::wav`] — minimal signed-16 PCM WAV reader

pub mod encoder;
pub mod error;
pub mod pack;
pub mod stream;

pub mod usecases {
    pub mod wav;
}

pub use encoder::{
    resolve_profile_by_geometry, ContainerPlan, EncodeResult, EncodeStats, Encoder, Pcm16,
    MIN_PCM_FRAMES,
};
pub use error::{EncoderError, InternalError};
pub use pack::{pack_analysis_frame, EncodedPacket};
pub use stream::{ProfileRef, StreamSession};
