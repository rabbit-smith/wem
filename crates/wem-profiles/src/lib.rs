//! WEM encoder profile domain: identity, registry, and packaged-resource
//! loading and verification.
//!
//! This crate is the only owner of packaged calibration resources, checksum
//! verification, codebook assembly, and calibrated table loaders
//! (docs/architecture.md). Mirrors the Python `wwise_wem/profiles` package.
//!
//! Entry points:
//! * [`DataDir`] — where the profile tree lives (WEM_DATA_DIR / repo layout)
//! * [`load_profile_bundle`] — index -> manifest -> verified resources
//! * [`load_profile_bundle_from_bytes`] — the same assembly from in-memory
//!   bytes (threadless targets; no filesystem, shared validation)
//! * [`ResourceRef`] — checksum-addressed resource identity
//! * [`ResourceBackend`] — filesystem or in-memory byte source
//! * [`load_wem_profile`] / [`resolve_wem_profile`] — exact identity lookup
//! * typed table loaders: [`load_mdct_looks`], [`load_transient_tables`],
//!   [`load_frozen_tables`], [`load_book_table`], and the
//!   [`psychoacoustics`] loaders
//! * [`assemble_encoder_profile_resources`] — full typed codec inputs

pub mod assembly;
pub mod book_ids;
pub mod bundle;
pub mod codebooks;
pub mod data;
pub mod error;
pub mod frozen;
pub mod key;
pub mod model;
pub mod psychoacoustics;
pub mod registry;
pub mod resources;
pub mod transform;
pub mod transient;

pub use assembly::{
    assemble_analysis_resources, assemble_encoder_profile_resources, EncoderProfileResources,
};
pub use book_ids::{BookTable, BookTables, T219_COUNT, T97_COUNT};
pub use bundle::{
    load_profile_bundle, load_profile_bundle_from_bytes, ProfileBundle, RuntimeResourceManifest,
    BUNDLE_SCHEMA, INDEX_SCHEMA,
};
pub use codebooks::{load_codebook, load_setup_codebooks, resolve_book_id, ResolvedBook};
pub use data::DataDir;
pub use error::ProfileError;
pub use frozen::load_frozen_tables;
pub use key::{ProfileKey, WWISE2013_6CH_44100_SETUP_IDENTITY, WWISE_GENERATION};
pub use model::{ContainerMetadata, EncoderProfile, ProfileManifestView};
pub use registry::{installed_registry, load_wem_profile, resolve_wem_profile, ProfileRegistry};
pub use resources::{normalize_resource_path, ResourceBackend, ResourceRef};
pub use transform::load_mdct_looks;
pub use transient::load_transient_tables;

/// Load one decoded codebook table from a manifest resource.
pub fn load_book_table(name: &str, ref_: &ResourceRef) -> Result<BookTable, ProfileError> {
    BookTable::load(name, ref_)
}
