//! Build typed runtime aggregates from a profile's logical manifest
//! (Python: `profiles/assembly.py`).

use std::collections::BTreeMap;

use wem_analysis::config::{
    make_long_floor_envelope_look, make_wwise_psy_look, AnalysisProfileResources,
    LongFloorEnvelopeLook,
};
use wem_vorbis::codebook::Codebook;
use wem_vorbis::setup::{parse_setup, SetupInfo};

use crate::book_ids::BookTable;
use crate::bundle::ProfileBundle;
use crate::codebooks::load_setup_codebooks;
use crate::error::ProfileError;
use crate::frozen::load_frozen_tables;
use crate::psychoacoustics::config::load_short_seed_surface;
use crate::psychoacoustics::long_tables::load_long_psy_tables;
use crate::psychoacoustics::long_variants::load_long_variant;
use crate::psychoacoustics::short_tables::load_short_psy_profiles;
use crate::transform::load_mdct_looks;
use crate::transient::load_transient_tables;

/// Complete immutable codec inputs assembled from one profile manifest
/// (Python `EncoderProfileResources`).
#[derive(Debug, Clone)]
pub struct EncoderProfileResources {
    pub analysis: AnalysisProfileResources,
    pub setup_packet: Vec<u8>,
    pub setup: SetupInfo,
    pub codebooks: Vec<Codebook>,
}

/// Resolve MDCT/transient/psy inputs through stable manifest logical names
/// (Python `assemble_analysis_resources`).
pub fn assemble_analysis_resources(
    bundle: &ProfileBundle,
) -> Result<AnalysisProfileResources, ProfileError> {
    let manifest = bundle.runtime_manifest();

    let short_surface = load_short_seed_surface(manifest.resource("psychoacoustics.short-seed")?)?;
    let long_base = load_long_psy_tables(manifest.resource("psychoacoustics.long-base")?)?;
    let long_modes_ref = manifest.resource("psychoacoustics.long-modes")?.clone();
    let mut long_variants = BTreeMap::new();
    for mode in [2, 3] {
        long_variants.insert(mode, load_long_variant(mode, &long_modes_ref, &long_base)?);
    }

    // The frozen tables are optional in the manifest; when present they are
    // loaded and injected into the short look.
    let frozen = manifest
        .resources()
        .iter()
        .find(|(name, _)| name == "analysis.frozen-tables")
        .map(|(_, ref_)| load_frozen_tables(ref_))
        .transpose()?;

    let mdct_looks = load_mdct_looks(manifest.resource("transform.mdct")?)?;
    let transient = load_transient_tables(manifest.resource("analysis.transient")?)?;
    let short_profiles =
        load_short_psy_profiles(manifest.resource("psychoacoustics.short-profiles")?)?;

    let short_look = make_wwise_psy_look(
        &short_surface,
        None,
        frozen.as_ref().map(|t| &t.coordinate_ln),
    )
    .map_err(ProfileError::Analysis)?;

    let mut long_floor_looks = BTreeMap::new();
    for (mode, table) in &long_variants {
        let look: LongFloorEnvelopeLook =
            make_long_floor_envelope_look(table).map_err(ProfileError::Analysis)?;
        long_floor_looks.insert(*mode, look);
    }

    AnalysisProfileResources::new(
        mdct_looks,
        transient,
        short_profiles,
        short_surface,
        short_look,
        long_base,
        long_variants,
        long_floor_looks,
        frozen,
    )
    .map_err(ProfileError::Analysis)
}

/// Resolve every analysis and Vorbis input from one profile identity
/// (Python `assemble_encoder_profile_resources`).
pub fn assemble_encoder_profile_resources(
    bundle: &ProfileBundle,
    setup_packet: Option<&[u8]>,
) -> Result<EncoderProfileResources, ProfileError> {
    // Argument evaluation order mirrors the Python implementation.
    let packet = match setup_packet {
        Some(bytes) => bytes.to_vec(),
        None => bundle.setup_packet()?,
    };
    let setup = parse_setup(&packet, bundle.key().channels()).map_err(ProfileError::Bit)?;

    let manifest = bundle.runtime_manifest();
    let t97 = BookTable::load("t97", manifest.resource("vorbis.codebooks.t97")?)?;
    let t219 = BookTable::load("t219", manifest.resource("vorbis.codebooks.t219")?)?;
    let tables = crate::book_ids::BookTables::new(t97, t219);

    let analysis = assemble_analysis_resources(bundle)?;
    let codebooks = load_setup_codebooks(&setup.book_ids, &tables)?;

    Ok(EncoderProfileResources {
        analysis,
        setup_packet: packet,
        setup,
        codebooks,
    })
}
