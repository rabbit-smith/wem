//! WEM encoder container domain: RIFF/WEM models and codecs.
//!
//! Per docs/architecture.md this crate does not import application,
//! profiles, or analysis code; it works on plain bytes and shared value
//! types only. Mirrors the Python `wwise_wem/container` package.

/// RIFF/WEM container value models (Python: `container/model.py`).
pub mod model {
    // Intentionally empty: container models land with the container port.
}

/// RIFF chunk layout helpers (Python: `container/chunks.py`).
pub mod chunks {
    // Intentionally empty: chunk layout lands with the container port.
}

/// WEM container writer/reader (Python: `container/wem.py`).
pub mod wem {
    // Intentionally empty: the WEM codec lands with the container port.
}
