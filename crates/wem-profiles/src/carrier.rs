//! The compiled profile carrier: the kernel's single source of profile data.
//!
//! Every shipped encoder reads its profile values from here. The tables under
//! [`crate::generated`] are Rust source, compiled into the artifact; this
//! module turns them into the typed objects the codec consumes, and resolves
//! a caller's [`WwiseProfile`] selection against them. There is no filesystem
//! read, no manifest, no digest chain and no resource tree.
//!
//! The readers below are total: the generated values were checked when they
//! were generated, so nothing here re-validates shape. The only fallible step
//! is `make_*`-style materialization, which can only fail on a geometry
//! disagreement that would be a build-time bug.

use std::collections::BTreeMap;

use wem_analysis::config::{
    FrozenMathTables, InputConditionerConfig, MdctLook, ShortPsyProfile, TransientBandConfig,
    TransientDetectorTables, WwisePsyLongTables, WwisePsySeedSurface, TONE_BAND_COUNT,
    TONE_LEVEL_COUNT,
};
use wem_analysis::dsp::transform::make_mdct_look;

use crate::book_ids::{BookTable, BookTables};
use crate::error::ProfileError;
use crate::generated;
use crate::key::ProfileKey;
use crate::model::EncoderProfile;
use crate::quality::{QualityCurves, QUALITY_CURVES_SCHEMA};
use crate::selection::WwiseProfile;
use crate::tables::{
    CodebookRowTable, LongTable, ProfileTables, ShortProfileTable, ShortSeedTable, TransientTable,
};
use crate::transient::{
    materialize_transient_tables, TransientRecord, TransientRecordFamily,
    TRANSIENT_RECORD_FAMILY_SCHEMA,
};

/// One compiled profile, resolved from the tables in this library.
#[derive(Debug, Clone)]
pub struct CompiledProfile {
    tables: &'static ProfileTables,
    key: ProfileKey,
}

impl CompiledProfile {
    /// Wrap one compiled table set.
    pub fn new(tables: &'static ProfileTables) -> Result<Self, ProfileError> {
        let parts = &tables.key;
        let key = ProfileKey::new(
            parts.channels,
            parts.sample_rate,
            parts.generation.to_string(),
            parts.channel_layout.to_string(),
            parts.quality_setup_identity.to_string(),
        )?;
        Ok(Self { tables, key })
    }

    /// The carrier tables behind this profile.
    pub fn tables(&self) -> &'static ProfileTables {
        self.tables
    }

    /// The container metadata recorded for this profile.
    pub fn container_metadata(&self) -> &'static crate::model::ContainerMetadata {
        &self.tables.container_metadata
    }

    /// The recorded block geometry.
    pub fn block_sizes(&self) -> [i64; 2] {
        self.tables.block_sizes
    }

    /// The identity as the public value model spells it.
    pub fn encoder_profile(&self) -> Result<EncoderProfile, ProfileError> {
        EncoderProfile::new(
            self.key.clone(),
            Some(self.tables.setup_packet.to_vec()),
            None,
            self.tables.block_sizes,
            self.tables.container_metadata,
            true,
            None,
        )
    }

    /// The profile's exact identity.
    pub fn key(&self) -> &ProfileKey {
        &self.key
    }

    /// The human label for this profile, derived from its key: a profile
    /// carries no stored name.
    pub fn label(&self) -> String {
        self.key.label()
    }

    /// The verified setup packet bytes.
    pub fn setup_packet(&self) -> Result<Vec<u8>, ProfileError> {
        Ok(self.tables.setup_packet.to_vec())
    }

    /// Every static MDCT trig bank, keyed by transform size.
    pub fn mdct_looks(&self) -> Result<BTreeMap<i64, MdctLook>, ProfileError> {
        let mut looks = BTreeMap::new();
        for bank in self.tables.resources.mdct_banks {
            let look = make_mdct_look(bank.n, Some(bank.trig)).map_err(ProfileError::Analysis)?;
            looks.insert(bank.n, look);
        }
        Ok(looks)
    }

    pub fn book_tables(&self) -> Result<BookTables, ProfileError> {
        let find = |name: &str| {
            self.tables
                .resources
                .codebook_tables
                .iter()
                .find(|entry| entry.name == name)
        };
        let t97 = match find("t97") {
            Some(entry) => BookTable::from_generated(entry.name, entry.rows)?,
            None => {
                return Err(ProfileError::UnknownBookTable {
                    table: "t97".to_string(),
                })
            }
        };
        let optional = |name: &str| -> Result<Option<BookTable>, ProfileError> {
            match find(name) {
                Some(entry) => Ok(Some(BookTable::from_generated(entry.name, entry.rows)?)),
                None => Ok(None),
            }
        };
        Ok(BookTables::new(t97, optional("t219")?, optional("t282")?))
    }

    pub fn frozen_tables(&self) -> Result<Option<FrozenMathTables>, ProfileError> {
        let table = self.tables.resources.frozen;
        let mut coordinate_ln = std::collections::HashMap::with_capacity(table.coordinate_ln.len());
        for &(in_bits, out_bits) in table.coordinate_ln {
            coordinate_ln.insert(in_bits, f64::from_bits(out_bits));
        }
        let mut fft_twiddles = std::collections::HashMap::with_capacity(table.fft_twiddles.len());
        for &(length, cos_bits, sin_bits) in table.fft_twiddles {
            fft_twiddles.insert(length, (f64::from_bits(cos_bits), f64::from_bits(sin_bits)));
        }
        let mut window_halves = std::collections::HashMap::with_capacity(table.window_halves.len());
        for &(size, half) in table.window_halves {
            window_halves.insert(size, half.to_vec());
        }
        Ok(Some(FrozenMathTables {
            coordinate_ln,
            fft_twiddles,
            window_halves,
        }))
    }

    /// The static record family this profile records, when its transient
    /// mechanism is the record family (materialization input; Python
    /// `TransientRecordFamily`). A profile that registers a pre-materialized
    /// detector table has none.
    pub fn transient_record_family(&self) -> Option<TransientRecordFamily> {
        match &self.tables.resources.transient {
            TransientTable::Detector(_) => None,
            TransientTable::RecordFamily(family) => Some(TransientRecordFamily {
                schema: TRANSIENT_RECORD_FAMILY_SCHEMA,
                n: family.n as u64,
                sample_rate: family.sample_rate,
                default_record_index: family.default_record_index,
                records: family
                    .records
                    .iter()
                    .map(|record| TransientRecord {
                        file_off: record.file_off.to_string(),
                        marker_u32: record.marker_u32,
                        upper_u32: record.upper_u32,
                        lower_u32: record.lower_u32,
                        carry_u32: record.carry_u32,
                        bias_u32: record.bias_u32,
                        m_u32: record.m_u32,
                        tail_u32: record.tail_u32,
                        config_u32: record.config_u32,
                    })
                    .collect(),
                index_curve: family.index_curve.to_vec(),
                breakpoints: family.breakpoints.to_vec(),
                window_u32: family.window.iter().map(|word| word.to_bits()).collect(),
                bands: bands(family.bands),
            }),
        }
    }

    /// The transient detector table for one quality value: either the
    /// pre-materialized detector the profile registers, or the record family
    /// materialized at that quality.
    pub fn transient_tables(
        &self,
        quality: Option<f64>,
    ) -> Result<TransientDetectorTables, ProfileError> {
        match self.tables.resources.transient {
            TransientTable::Detector(detector) => Ok(TransientDetectorTables {
                n: detector.n,
                bias: detector.bias,
                window: detector.window.to_vec(),
                config: detector.config.to_vec(),
                bands: bands(detector.bands),
            }),
            TransientTable::RecordFamily(_) => {
                let family = self
                    .transient_record_family()
                    .expect("the record-family arm produced this value");
                materialize_transient_tables(&family, quality)
            }
        }
    }

    pub fn short_seed(&self) -> Result<WwisePsySeedSurface, ProfileError> {
        Ok(short_seed(self.tables.resources.short_seed))
    }

    pub fn short_profiles(&self) -> Result<Vec<ShortPsyProfile>, ProfileError> {
        Ok(self
            .tables
            .resources
            .short_profiles
            .iter()
            .map(short_profile)
            .collect())
    }

    pub fn long_base(&self) -> Result<WwisePsyLongTables, ProfileError> {
        Ok(long_table(self.tables.resources.long_base))
    }

    pub fn long_variant(&self, mode: i64) -> Result<WwisePsyLongTables, ProfileError> {
        let base = self.long_base()?;
        let Some(variant) = self
            .tables
            .resources
            .long_variants
            .iter()
            .find(|entry| entry.mode == mode)
        else {
            return Err(ProfileError::LongVariantModeInvalid { mode });
        };
        Ok(WwisePsyLongTables {
            sample_rate: base.sample_rate,
            n: base.n,
            profile_key: format!("1024:{mode}"),
            analysis_profile_u32: variant.analysis_profile_u32.to_vec(),
            analysis_interval_u32: variant.analysis_interval_u32.to_vec(),
            analysis_curves: reshape(variant.analysis_curves, 3, 1024),
            analysis_field_19_curve: variant.analysis_field_19_curve.to_vec(),
            seed_outer_u32: base.seed_outer_u32.clone(),
            seed_profile_u32: base.seed_profile_u32.clone(),
            seed_base_curve: base.seed_base_curve.clone(),
            seed_group_labels_u32: base.seed_group_labels_u32.clone(),
            seed_tone_banks: base.seed_tone_banks.clone(),
        })
    }

    pub fn quality_curves(&self) -> Result<Option<QualityCurves>, ProfileError> {
        let Some(table) = self.tables.resources.quality_curves else {
            return Ok(None);
        };
        let curves: BTreeMap<String, Vec<f64>> = table
            .curves
            .iter()
            .map(|(name, values)| ((*name).to_string(), values.to_vec()))
            .collect();
        let semantics: BTreeMap<String, String> = table
            .semantics
            .iter()
            .map(|(name, semantic)| ((*name).to_string(), (*semantic).to_string()))
            .collect();
        Ok(Some(QualityCurves::new(
            QUALITY_CURVES_SCHEMA.to_string(),
            table.breakpoints.to_vec(),
            curves,
            semantics,
        )?))
    }

    pub fn input_conditioner(&self) -> Result<Option<InputConditionerConfig>, ProfileError> {
        match self.tables.resources.input_conditioner {
            Some(bits) => Ok(Some(
                InputConditionerConfig::new(f32::from_bits(bits))
                    .map_err(ProfileError::Analysis)?,
            )),
            None => Ok(None),
        }
    }
}

/// The compiled profile one structured selection denotes.
///
/// Resolution is exactly [`crate::ProfileRegistry::resolve_selection`] over
/// the compiled tables: generation, channels and sample rate all participate,
/// and an unsatisfiable or ambiguous selection is an error — never a
/// first-match pick.
pub fn compiled_profile_for_selection(
    selection: WwiseProfile,
) -> Result<CompiledProfile, ProfileError> {
    let matches: Vec<&'static ProfileTables> = generated::PROFILES
        .iter()
        .copied()
        .filter(|tables| {
            tables.key.generation == selection.version().generation()
                && (tables.key.channels, tables.key.sample_rate)
                    == (selection.channels(), selection.sample_rate())
        })
        .collect();
    if matches.is_empty() {
        return Err(ProfileError::NoProfileForSelection {
            version: selection.version().label().to_string(),
            channels: selection.channels(),
            sample_rate: selection.sample_rate(),
            installed: describe_installed(),
        });
    }
    if matches.len() != 1 {
        let names = matches
            .iter()
            .map(|tables| {
                crate::key::profile_description(
                    tables.key.channels,
                    tables.key.sample_rate,
                    tables.key.generation,
                    tables.key.channel_layout,
                    tables.key.quality_setup_identity,
                )
            })
            .collect::<Vec<_>>()
            .join(", ");
        return Err(ProfileError::AmbiguousProfileSelection {
            version: selection.version().label().to_string(),
            channels: selection.channels(),
            sample_rate: selection.sample_rate(),
            names,
        });
    }
    CompiledProfile::new(matches[0])
}

/// Every compiled profile, in deterministic (sorted) order.
pub fn compiled_profiles() -> Result<Vec<CompiledProfile>, ProfileError> {
    generated::PROFILES
        .iter()
        .map(|tables| CompiledProfile::new(tables))
        .collect()
}

fn describe_installed() -> String {
    let mut described: Vec<String> = generated::PROFILES
        .iter()
        .map(|tables| {
            crate::key::profile_label(
                tables.key.channels,
                tables.key.sample_rate,
                tables.key.generation,
            )
        })
        .collect();
    described.sort();
    described.join(", ")
}

fn bands(rows: &'static [crate::tables::BandTable]) -> Vec<TransientBandConfig> {
    rows.iter()
        .map(|band| TransientBandConfig {
            offset: band.offset,
            weights: band.weights.to_vec(),
            scale: band.scale,
        })
        .collect()
}

fn reshape(values: &[f32], rows: usize, width: usize) -> Vec<Vec<f32>> {
    (0..rows)
        .map(|row| values[row * width..(row + 1) * width].to_vec())
        .collect()
}

fn reshape_f64(values: &[f64], rows: usize, width: usize) -> Vec<Vec<f64>> {
    (0..rows)
        .map(|row| values[row * width..(row + 1) * width].to_vec())
        .collect()
}

/// The short psychoacoustic seed surface, reshaped from the flat carrier.
pub(crate) fn short_seed(table: &ShortSeedTable) -> WwisePsySeedSurface {
    let bands = TONE_BAND_COUNT as usize;
    let levels = TONE_LEVEL_COUNT as usize;
    let records = 58usize;
    let tone_curves = (0..bands)
        .map(|band| {
            (0..levels)
                .map(|level| {
                    let start = (band * levels + level) * records;
                    table.tone_curves[start..start + records].to_vec()
                })
                .collect()
        })
        .collect();
    WwisePsySeedSurface {
        n: table.n,
        sample_rate: table.sample_rate,
        ath_offset: table.ath_offset,
        ath_floor: table.ath_floor,
        seed_ceiling: table.seed_ceiling,
        max_curve_db: table.max_curve_db,
        curve_offset: table.curve_offset,
        curve_slope: table.curve_slope,
        curve_offset_2: table.curve_offset_2,
        curve_attenuation: table.curve_attenuation.to_vec(),
        ath: table.ath.to_vec(),
        octave: table.octave.to_vec(),
        first_octave: table.first_octave,
        shift_octave: table.shift_octave,
        eighth_octave_lines: table.eighth_octave_lines,
        total_octave_lines: table.total_octave_lines,
        tone_curves,
        row: table.row.to_vec(),
        envelope_low: table.envelope_low.to_vec(),
        envelope_high: table.envelope_high.to_vec(),
        mask_curve: table.mask_curve.to_vec(),
        interval_table: table.interval_table.to_vec(),
        short_limit: table.short_limit,
        noise_fixed_window: table.noise_fixed_window,
        regular_curve_bias: table.regular_curve_bias,
        regular_curve_cap: table.regular_curve_cap,
        remap_curve_offsets: table.remap_curve_offsets.to_vec(),
        remap_low_by_index: table.remap_low_by_index.to_vec(),
        remap_high_by_index: table.remap_high_by_index.to_vec(),
    }
}

/// One short psychoacoustic profile, reshaped from the flat carrier.
pub(crate) fn short_profile(table: &ShortProfileTable) -> ShortPsyProfile {
    ShortPsyProfile {
        key: table.key.to_string(),
        candidate_bias_by_mode: table.candidate_bias_by_mode.to_vec(),
        curve_cap: table.curve_cap,
        group_enabled: table.group_enabled,
        candidate_bound: table.candidate_bound,
        group_span: table.group_span,
        band_limits: table.band_limits,
        side_gain: table.side_gain,
        peak_cutoff: table.peak_cutoff,
        blend_weight: table.blend_weight,
        mask_curves: reshape_f64(table.mask_curves, 3, 128),
    }
}

/// One fixed 1024-bin long table, reshaped from the flat carrier.
pub(crate) fn long_table(table: &LongTable) -> WwisePsyLongTables {
    let bands = TONE_BAND_COUNT as usize;
    let levels = TONE_LEVEL_COUNT as usize;
    let records = 58usize;
    let tone_banks = (0..bands)
        .map(|band| {
            (0..levels)
                .map(|level| {
                    let start = (band * levels + level) * records;
                    table.seed_tone_banks[start..start + records].to_vec()
                })
                .collect()
        })
        .collect();
    WwisePsyLongTables {
        sample_rate: table.sample_rate,
        n: table.n,
        profile_key: table.profile_key.to_string(),
        analysis_profile_u32: table.analysis_profile_u32.to_vec(),
        analysis_interval_u32: table.analysis_interval_u32.to_vec(),
        analysis_curves: reshape(table.analysis_curves, 3, 1024),
        analysis_field_19_curve: table.analysis_field_19_curve.to_vec(),
        seed_outer_u32: table.seed_outer_u32.to_vec(),
        seed_profile_u32: table.seed_profile_u32.to_vec(),
        seed_base_curve: table.seed_base_curve.to_vec(),
        seed_group_labels_u32: table.seed_group_labels_u32.to_vec(),
        seed_tone_banks: tone_banks,
    }
}

/// Convert one compiled codebook row into the codec's row type.
pub(crate) fn codebook_row(row: &CodebookRowTable) -> wem_vorbis::codebook::CodebookRow {
    wem_vorbis::codebook::CodebookRow {
        i: row.i,
        dim: row.dim,
        entries: row.entries,
        lengthlist: row.lengthlist.map(<[i64]>::to_vec),
        maptype: row.maptype,
        q_min: row.q_min,
        q_delta: row.q_delta,
        q_quant: row.q_quant,
        q_sequencep: row.q_sequencep,
        quantlist: row.quantlist.map(<[i64]>::to_vec),
        quantvals: row.quantvals,
    }
}
