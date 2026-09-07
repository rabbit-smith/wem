//! WEM encoder analysis domain: DSP, transient detection, and psychoacoustic
//! analysis over typed, checksum-verified resource models.
//!
//! This crate is resource-free by design (docs/architecture.md): it never
//! opens package resources and never imports the profiles crate. It receives
//! typed configuration exclusively (see `config`).
//!
//! Mirrors the Python `wwise_wem/analysis` package layout.

/// Typed, immutable inputs consumed by the analysis domain
/// (Python: `analysis/config.py`). The profile layer owns loading; this
/// module owns the shapes those loads must produce.
pub mod config;

pub mod dsp {
    pub mod transform;
}

/// LPC-padded detector input and window materialization
/// (Python: `analysis/preprocessing/`).
pub mod preprocessing {
    // Intentionally empty: preprocessing lands with the analysis port.
}

/// Transient detector algorithm (Python: `analysis/transient/`).
pub mod transient {
    // Intentionally empty: the detector lands with the analysis port.
}

/// Remap, seed, envelope, and temporal psychoacoustic analysis
/// (Python: `analysis/psychoacoustics/`).
pub mod psychoacoustics {
    // Intentionally empty: the psychoacoustic algorithms land with the analysis port.
}

/// Mutable cross-frame analysis session (Python: `analysis/session.py`).
pub mod session {
    // Intentionally empty: the session lands with the analysis port.
}

/// Analysis-side value model shared with the Vorbis packet encoder
/// (Python: `analysis/model.py`, e.g. `PsyFrame`).
pub mod model {
    // Intentionally empty: `PsyFrame` lands with the analysis port.
}
