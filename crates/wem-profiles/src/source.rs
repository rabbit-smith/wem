//! The source interface the assembly layer reads a profile through.
//!
//! Assembly needs typed codec inputs, never resource bytes: what a profile
//! carries is fixed by the compiled carrier
//! ([`crate::generated`]), and the only other implementation of this trait is
//! the development-tree loader the in-crate loader suite exercises. The two
//! cannot drift silently — `crates/wem-profiles/tests/compiled_carrier.rs`
//! compares them table by table.

use std::collections::BTreeMap;

use wem_analysis::config::{
    FrozenMathTables, InputConditionerConfig, MdctLook, ShortPsyProfile, TransientDetectorTables,
    WwisePsyLongTables, WwisePsySeedSurface,
};

use crate::book_ids::BookTables;
use crate::error::ProfileError;
use crate::key::ProfileKey;
use crate::quality::QualityCurves;

/// Everything assembly reads from one encoder profile.
pub trait ProfileSource {
    /// The profile's installed name (a diagnostic label).
    fn name(&self) -> &str;

    /// The profile's exact identity.
    fn key(&self) -> &ProfileKey;

    /// The verified setup packet bytes.
    fn setup_packet(&self) -> Result<Vec<u8>, ProfileError>;

    /// Every static MDCT trig bank, keyed by transform size.
    fn mdct_looks(&self) -> Result<BTreeMap<i64, MdctLook>, ProfileError>;

    /// The codebook tables this profile's setup references.
    fn book_tables(&self) -> Result<BookTables, ProfileError>;

    /// The frozen transcendental tables, when the profile records them.
    fn frozen_tables(&self) -> Result<Option<FrozenMathTables>, ProfileError>;

    /// The transient detector surface for one quality value.
    fn transient_tables(
        &self,
        quality: Option<f64>,
    ) -> Result<TransientDetectorTables, ProfileError>;

    /// The short psychoacoustic seed surface.
    fn short_seed(&self) -> Result<WwisePsySeedSurface, ProfileError>;

    /// The two short-block psychoacoustic profiles.
    fn short_profiles(&self) -> Result<Vec<ShortPsyProfile>, ProfileError>;

    /// The fixed 1024-bin long tables.
    fn long_base(&self) -> Result<WwisePsyLongTables, ProfileError>;

    /// One long analysis mode surface (mode 2 or 3).
    fn long_variant(&self, mode: i64) -> Result<WwisePsyLongTables, ProfileError>;

    /// The optional quality-interpolation curves.
    fn quality_curves(&self) -> Result<Option<QualityCurves>, ProfileError>;

    /// The optional DC-filter configuration.
    fn input_conditioner(&self) -> Result<Option<InputConditionerConfig>, ProfileError>;
}
/// Development seam: the same source served from the recorded resource tree.
///
/// This implementation exists so the in-crate loader suite and the
/// `compiled_carrier` equivalence test can hold the compiled carrier against
/// the documents it was generated from. Shipped encoders never take this
/// path.
impl ProfileSource for crate::bundle::ProfileBundle {
    fn name(&self) -> &str {
        crate::bundle::ProfileBundle::name(self)
    }

    fn key(&self) -> &ProfileKey {
        crate::bundle::ProfileBundle::key(self)
    }

    fn setup_packet(&self) -> Result<Vec<u8>, ProfileError> {
        crate::bundle::ProfileBundle::setup_packet(self)
    }

    fn mdct_looks(&self) -> Result<BTreeMap<i64, MdctLook>, ProfileError> {
        crate::transform::load_mdct_looks(self.runtime_manifest().resource("transform.mdct")?)
    }

    fn book_tables(&self) -> Result<BookTables, ProfileError> {
        let manifest = self.runtime_manifest();
        let t97 =
            crate::book_ids::BookTable::load("t97", manifest.resource("vorbis.codebooks.t97")?)?;
        let t219 = match manifest.resource("vorbis.codebooks.t219") {
            Ok(ref_) => Some(crate::book_ids::BookTable::load("t219", ref_)?),
            Err(_) => None,
        };
        let t282 = match manifest.resource("vorbis.codebooks.t282") {
            Ok(ref_) => Some(crate::book_ids::BookTable::load("t282", ref_)?),
            Err(_) => None,
        };
        Ok(BookTables::new(t97, t219, t282))
    }

    fn frozen_tables(&self) -> Result<Option<FrozenMathTables>, ProfileError> {
        self.runtime_manifest()
            .resources()
            .iter()
            .find(|(name, _)| name == "analysis.frozen-tables")
            .map(|(_, ref_)| crate::frozen::load_frozen_tables(ref_))
            .transpose()
    }

    fn transient_tables(
        &self,
        quality: Option<f64>,
    ) -> Result<TransientDetectorTables, ProfileError> {
        crate::transient::load_transient(
            self.runtime_manifest().resource("analysis.transient")?,
            quality,
        )
    }

    fn short_seed(&self) -> Result<WwisePsySeedSurface, ProfileError> {
        crate::psychoacoustics::config::load_short_seed_surface(
            self.runtime_manifest()
                .resource("psychoacoustics.short-seed")?,
        )
    }

    fn short_profiles(&self) -> Result<Vec<ShortPsyProfile>, ProfileError> {
        crate::psychoacoustics::short_tables::load_short_psy_profiles(
            self.runtime_manifest()
                .resource("psychoacoustics.short-profiles")?,
        )
    }

    fn long_base(&self) -> Result<WwisePsyLongTables, ProfileError> {
        crate::psychoacoustics::long_tables::load_long_psy_tables(
            self.runtime_manifest()
                .resource("psychoacoustics.long-base")?,
        )
    }

    fn long_variant(&self, mode: i64) -> Result<WwisePsyLongTables, ProfileError> {
        let manifest = self.runtime_manifest();
        let base = crate::psychoacoustics::long_tables::load_long_psy_tables(
            manifest.resource("psychoacoustics.long-base")?,
        )?;
        crate::psychoacoustics::long_variants::load_long_variant(
            mode,
            manifest.resource("psychoacoustics.long-modes")?,
            &base,
        )
    }

    fn quality_curves(&self) -> Result<Option<QualityCurves>, ProfileError> {
        self.runtime_manifest()
            .resources()
            .iter()
            .find(|(name, _)| name == crate::quality::QUALITY_CURVES_RESOURCE)
            .map(|(_, ref_)| crate::quality::load_quality_curves(ref_))
            .transpose()
    }

    fn input_conditioner(&self) -> Result<Option<InputConditionerConfig>, ProfileError> {
        let Some((_, ref_)) = self
            .runtime_manifest()
            .resources()
            .iter()
            .find(|(name, _)| name == "analysis.input-conditioner")
        else {
            return Ok(None);
        };
        let payload = ref_.read_json()?;
        let object = payload
            .as_object()
            .ok_or_else(|| ProfileError::ManifestFieldMalformed {
                reason: "input conditioner resource must be an object".to_string(),
            })?;
        if object.get("schema").and_then(|value| value.as_str()) != Some("wem.input-conditioner.v1")
        {
            return Err(ProfileError::ManifestFieldMalformed {
                reason: "unsupported input conditioner schema".to_string(),
            });
        }
        let bits = object
            .get("dc_filter_coefficient_f32_bits")
            .and_then(|value| value.as_u64())
            .filter(|value| *value <= u32::MAX as u64)
            .ok_or_else(|| ProfileError::ManifestFieldMalformed {
                reason: "input conditioner coefficient bits must be a u32".to_string(),
            })? as u32;
        Ok(Some(
            InputConditionerConfig::new(f32::from_bits(bits)).map_err(ProfileError::Analysis)?,
        ))
    }
}
