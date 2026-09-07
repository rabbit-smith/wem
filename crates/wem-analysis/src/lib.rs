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
    pub mod lpc;
    pub mod spectrum;
    pub mod transform;
}

/// LPC-padded detector input and window materialization
/// (Python: `analysis/preprocessing/`). The streaming module is the
/// Rust-only incremental feeder for the core streaming session; it
/// shares the batch kernel (LPC, windowing, sample views) and owns no
/// separate numerics.
pub mod preprocessing {
    pub mod detector_input;
    pub mod streaming;
    pub mod windowing;
}

/// Transient detector algorithm (Python: `analysis/transient/`).
pub mod transient {
    pub mod detector;
}

/// Remap, seed, envelope, and temporal psychoacoustic analysis
/// (Python: `analysis/psychoacoustics/`).
pub mod psychoacoustics {
    pub mod envelope;
    pub mod pipeline;
    pub mod remap;
    pub mod seed;
    pub mod short;
    pub mod temporal;
}

/// Mutable cross-frame analysis session (Python: `analysis/session.py`).
pub mod session;

/// Analysis-side value model shared with the Vorbis packet encoder
/// (Python: `analysis/model.py`, e.g. `PsyFrame`).
pub mod model;
