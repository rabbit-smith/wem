//! WEM encoder profile domain: identity, selection, and the compiled carrier.
//!
//! Profile data is Rust code: the typed tables under [`generated`] are the
//! single carrier, compiled into every artifact, and there is no second
//! source — no resource tree, no index, no manifest, no digest chain and no
//! filesystem read. This crate is the only owner of the installed calibration
//! data and of codebook assembly (docs/reference/architecture.md).
//!
//! Entry points:
//! * [`WwiseVersion`] / [`WwiseProfile`] — the caller-facing structured
//!   selector (generation + geometry); the only profile selector any binding
//!   exposes
//! * [`compiled_profile_for_selection`] — the one way to obtain the
//!   [`CompiledProfile`] for a selection; its signature carries no name, no
//!   path and no profile index/manifest bytes, because the profile it resolves
//!   is the one compiled into this library
//! * [`resolve_wem_profile_selection`] — the resolved identity (key, setup
//!   packet, container metadata) for a selection
//! * [`ProfileKey`] — exact encoder identity; the human label is derived from
//!   it, never stored
//! * typed table accessors on [`CompiledProfile`]: `mdct_looks`,
//!   `transient_tables`, `frozen_tables`, `book_tables`, and the short/long
//!   psychoacoustic surfaces
//! * [`assemble_encoder_profile_resources`] — full typed codec inputs
//!
//! Nothing here accepts a profile name, a profile path, profile
//! index/manifest bytes, or an environment variable: the only selector is a
//! [`WwiseProfile`].

pub mod assembly;
pub mod blob;
pub mod book_ids;
pub mod carrier;
pub mod codebooks;
pub mod error;
pub mod key;
pub mod model;
pub mod quality;
pub mod registry;
pub mod selection;
pub mod tables;
pub mod transient;

/// The compiled profile carrier: Rust source, generated from the recorded
/// profile tree by `scripts/generate_profile_code.py`.
pub mod generated;

pub use assembly::{
    assemble_analysis_resources, assemble_encoder_profile_resources, EncoderProfileResources,
};
pub use book_ids::{BookTable, BookTables, T219_COUNT, T282_COUNT, T97_COUNT};
pub use carrier::{compiled_profile_for_selection, compiled_profiles, CompiledProfile};
pub use codebooks::{load_codebook, load_setup_codebooks, resolve_book_id, ResolvedBook};
pub use error::ProfileError;
pub use key::{ProfileKey, WWISE_GENERATION};
pub use model::{ContainerMetadata, EncoderProfile};
pub use quality::{
    linear_frac, normalize_quality_factor, QualityCurves, QUALITY_CURVES_INTERPOLATION,
    QUALITY_CURVES_SCHEMA,
};
pub use registry::{
    embedded_registry, resolve_wem_profile_selection, resolve_wem_profile_selection_quality,
    ProfileRegistry,
};
pub use selection::{WwiseProfile, WwiseVersion};
