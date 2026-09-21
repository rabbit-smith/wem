//! WEM encoder profile domain: identity, registry, and packaged-resource
//! verification.
//!
//! This crate is the only owner of packaged calibration resources, checksum
//! verification, codebook assembly, and calibrated table loaders
//! (docs/reference/architecture.md). Mirrors the Python `wwise_wem/profiles` package.
//!
//! Entry points:
//! * [`WwiseVersion`] / [`WwiseProfile`] — the caller-facing structured
//!   selector (generation + geometry); the only profile selector any binding
//!   exposes
//! * [`bundle_for_selection`] — the one way to obtain the verified
//!   [`ProfileBundle`] for a selection; its signature carries no name, no path
//!   and no profile index/manifest bytes, because the bundle it resolves is
//!   the one compiled into this library
//! * [`ResourceRef`] — checksum-addressed resource identity
//! * [`resolve_wem_profile_selection`] — exact identity lookup from the
//!   structured selector
//! * typed table loaders: [`load_mdct_looks`], [`load_transient_tables`],
//!   [`load_frozen_tables`], [`load_book_table`], and the
//!   [`psychoacoustics`] loaders
//! * [`assemble_encoder_profile_resources`] — full typed codec inputs
//!
//! The intake that turns bytes on disk (or bytes in memory) into a bundle is
//! crate-private: `bundle::load_profile_bundle` and its in-memory twin address
//! profiles by name and are reachable only from inside this crate, so no
//! caller-facing surface can accept a profile name, a profile path, profile
//! index/manifest bytes, or an environment variable.

pub mod assembly;
pub mod book_ids;
pub mod bundle;
pub mod codebooks;
pub(crate) mod data;
pub(crate) mod embedded;
pub mod error;
pub mod frozen;
pub mod key;
pub mod model;
pub mod psychoacoustics;
pub mod quality;
pub mod registry;
pub mod resources;
pub mod selection;
pub mod transform;
pub mod transient;

#[cfg(test)]
mod bytes_loader_tests;
#[cfg(test)]
mod loader_tests;

pub use assembly::{
    assemble_analysis_resources, assemble_encoder_profile_resources, EncoderProfileResources,
};
pub use book_ids::{BookTable, BookTables, T219_COUNT, T282_COUNT, T97_COUNT};
pub use bundle::{ProfileBundle, RuntimeResourceManifest, BUNDLE_SCHEMA, INDEX_SCHEMA};
pub use codebooks::{load_codebook, load_setup_codebooks, resolve_book_id, ResolvedBook};
pub use error::ProfileError;
pub use frozen::load_frozen_tables;
pub use key::{ProfileKey, WWISE2013_6CH_44100_SETUP_IDENTITY, WWISE_GENERATION};
pub use model::{ContainerMetadata, EncoderProfile, ProfileManifestView};
pub use quality::{
    linear_frac, load_quality_curves, normalize_quality_factor, QualityCurves,
    QUALITY_CURVES_INTERPOLATION, QUALITY_CURVES_RESOURCE, QUALITY_CURVES_SCHEMA,
};
pub use registry::{
    bundle_for_selection, embedded_registry, resolve_wem_profile_selection,
    resolve_wem_profile_selection_quality, ProfileRegistry,
};
pub use resources::ResourceRef;
pub use selection::{WwiseProfile, WwiseVersion};
pub use transform::load_mdct_looks;
pub use transient::load_transient_tables;

/// Load one decoded codebook table from a manifest resource.
pub fn load_book_table(name: &str, ref_: &ResourceRef) -> Result<BookTable, ProfileError> {
    BookTable::load(name, ref_)
}
