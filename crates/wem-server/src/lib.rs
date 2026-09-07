//! wem-server — the tonic gRPC shell for the wwise.v1 WEM encode contract.
//!
//! This crate is a thin wrapper over [`wem_core::StreamSession`]:
//!
//! * `ListProfiles` reports the installed profiles (name /
//!   `setup_sha256` / `manifest_sha256`) as the version handshake.
//! * `Encode` maps the `Init` -> `chunk`* -> `Finish` request lifecycle
//!   onto `StreamSession::init_profile` / `push_pcm_chunk` / `finish`,
//!   forwarding each emitted packet immediately (the service layer never
//!   buffers or reorders — chunk-boundary invariance is guaranteed by the
//!   kernel, and the reply stream order is the kernel's emission order:
//!   seq 0 = setup packet, then audio packets in encoding order).
//! * Every [`wem_core::EncoderError`] variant maps 1:1 onto the
//!   `EncoderErrorCode` enum; lifecycle violations are terminal
//!   `STATE_ERROR` frames — nothing panics.
//!
//! The generated contract modules are exposed as [`crate::wwise`].

pub mod service;

/// Generated wwise.v1 contract (from `proto/wwise/v1/*.proto`, OUT_DIR).
pub mod wwise {
    // Allow lints tripped by the generated code: proto comments are rendered
    // as doc comments that `doc_lazy_continuation` rejects. Scoped here so
    // the allowance never covers hand-written code.
    #![allow(clippy::doc_lazy_continuation)]
    tonic::include_proto!("wwise.v1");
}
