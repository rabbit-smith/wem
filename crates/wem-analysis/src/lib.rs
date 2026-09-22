//! WEM encoder analysis domain: DSP, transient detection, and psychoacoustic
//! analysis over typed, checksum-verified resource models.
//!
//! This crate is resource-free by design (docs/reference/architecture.md): it never
//! opens package resources and never imports the profiles crate. It receives
//! typed configuration exclusively (see `config`).
//!
//! Mirrors the Python `wwise_wem/analysis` package layout.

/// Typed, immutable inputs consumed by the analysis domain
/// (Python: `analysis/config.py`). The profile layer owns loading; this
/// module owns the shapes those loads must produce.
pub mod config;

pub mod dsp {
    /// Value-semantics ports of the CRT carrier's _CIatan/_CIexp/_CIlog
    /// (binary64 SSE2 callees; carrier tables in `crt90_data`).
    pub mod crt90;
    pub mod crt90_data;
    pub mod lpc;
    /// Geometry materializers for `ath`, `octave`, smoothing intervals, and
    /// mask curves; constants live in `psy_geom_data`, with parity tests.
    pub mod psy_geom;
    pub mod psy_geom_data;
    /// LONG (1024-bin) materializer outputs reusing the SHORT engine with
    /// mode-selected banks (parity: tests/psy_geom_long_parity.rs).
    pub mod psy_geom_long;
    pub mod psy_geom_long_data;
    pub mod spectrum;
    pub mod transform;
    /// x87 register model of the paired 2013.2 build (port of the builder
    /// reference `f32.py`): finite normal-range mul80/add80 with ties-to-even
    /// single rounding, plus f32 store helpers. Parity-locked to the Python
    /// reference via `tests/x87_parity.rs`.
    pub mod x87;
}

/// LPC-padded detector input and window materialization
/// (Python: `analysis/preprocessing/`). The streaming module is the
/// Rust-only incremental feeder for the core streaming session; it
/// shares the batch kernel (LPC, windowing, sample views) and owns no
/// separate numerics.
pub mod preprocessing {
    pub mod conditioner;
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
    /// The session-owned worker pool the long-frame channel waves run in
    /// (see the module docs for why its size is the encode's geometry and not
    /// the host's, and for what a caller may read about it).
    pub mod pool;
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
