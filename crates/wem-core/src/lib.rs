//! WEM encoder orchestration layer.
//!
//! This is the only crate allowed to assemble profiles, analysis, packet
//! encoding, and containers into the complete WAV-to-WEM use case
//! (docs/architecture.md). Mirrors the Python `wwise_wem/application`
//! package plus the package-root API shell.

/// End-to-end encoding session (Python: `application/encoder.py`).
pub mod encoder {
    // Intentionally empty: the encoder lands with the P2-4 port.
}

/// Use-case orchestration and compatibility adapters.
pub mod usecases {
    // Intentionally empty: use cases land with the P2-4 port.
}
