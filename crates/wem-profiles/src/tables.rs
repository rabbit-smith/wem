//! The shape of the compiled profile carrier.
//!
//! Profile data is Rust code in this kernel: the typed tables under
//! [`crate::generated`] are the single carrier, and every other language reads
//! them from the compiled artifact. This module defines the table shapes and
//! nothing else — no I/O, no decoding, no validation.
//!
//! Two rules keep the carrier exact:
//!
//! * floating-point content is a bit pattern (`f32::from_bits` /
//!   `f64::from_bits`) in the generated source, never a decimal literal;
//! * the generated tables are the *decoded* values the resource loaders used
//!   to produce, not a re-serialization of the recorded documents, so wire
//!   format details (base64, RLE, byte-hex, `u32` masks) are settled once at
//!   generation time and cannot drift at run time.

use crate::model::ContainerMetadata;

/// Profile identity fields, as recorded in the paired build.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ProfileKeyParts {
    pub channels: i64,
    pub sample_rate: i64,
    pub generation: &'static str,
    pub channel_layout: &'static str,
    pub quality_setup_identity: &'static str,
}

/// One static MDCT trig bank: `n + n/4` stored f32 words.
#[derive(Debug, Clone, Copy)]
pub struct MdctBankTable {
    pub n: i64,
    pub trig: &'static [f32],
}

/// One codebook row, byte-exact with the recorded table.
#[derive(Debug, Clone, Copy)]
pub struct CodebookRowTable {
    pub i: Option<i64>,
    pub dim: i64,
    pub entries: i64,
    pub maptype: i64,
    pub q_min: i64,
    pub q_delta: i64,
    pub q_quant: i64,
    pub q_sequencep: i64,
    pub quantvals: Option<i64>,
    pub lengthlist: Option<&'static [i64]>,
    pub quantlist: Option<&'static [i64]>,
}

/// One named codebook table installed for a profile.
#[derive(Debug, Clone, Copy)]
pub struct CodebookTableRef {
    pub name: &'static str,
    pub rows: &'static [CodebookRowTable],
}

/// Frozen transcendental results: `(frequency bits, ln bits)` pairs, FFT stage
/// twiddles, and the block-window halves.
#[derive(Debug, Clone, Copy)]
pub struct FrozenTable {
    /// Frequency f64 bit pattern -> ln result f64 bit pattern.
    pub coordinate_ln: &'static [(u64, u64)],
    /// `(stage length, cos bits, sin bits)`.
    pub fft_twiddles: &'static [(i64, u64, u64)],
    /// `(block size, first window half as f32 bit patterns)`.
    pub window_halves: &'static [(i64, &'static [f32])],
}

/// One transient band descriptor.
#[derive(Debug, Clone, Copy)]
pub struct BandTable {
    pub offset: i64,
    pub weights: &'static [f32],
    pub scale: f32,
}

/// The pre-materialized n=128 detector table.
#[derive(Debug, Clone, Copy)]
pub struct DetectorTable {
    pub n: i64,
    pub bias: f32,
    pub window: &'static [f32],
    pub config: &'static [f32],
    pub bands: &'static [BandTable],
}

/// One stored bias/threshold record (u32 bit patterns, byte-exact).
#[derive(Debug, Clone, Copy)]
pub struct RecordTable {
    pub file_off: &'static str,
    pub marker_u32: u32,
    pub upper_u32: [u32; 12],
    pub lower_u32: [u32; 12],
    pub carry_u32: u32,
    pub bias_u32: u32,
    pub m_u32: u32,
    pub tail_u32: u32,
    /// All 26 words in runtime order: marker, upper[0..12), lower[0..12),
    /// carry.
    pub config_u32: [u32; 26],
}

/// The static record library plus its immutable detector surfaces.
#[derive(Debug, Clone, Copy)]
pub struct RecordFamilyTable {
    pub n: i64,
    pub sample_rate: i64,
    pub default_record_index: f64,
    pub records: &'static [RecordTable],
    pub index_curve: &'static [f64],
    pub breakpoints: &'static [f64],
    pub window: &'static [f32],
    pub bands: &'static [BandTable],
}

/// Which transient mechanism the profile records.
#[derive(Debug, Clone, Copy)]
pub enum TransientTable {
    /// A pre-materialized detector table (quality-independent).
    Detector(DetectorTable),
    /// The static record family, materialized per quality.
    RecordFamily(&'static RecordFamilyTable),
}

/// The optional per-profile quality-interpolation curves.
#[derive(Debug, Clone, Copy)]
pub struct QualityCurvesTable {
    pub breakpoints: &'static [f64],
    /// `(curve name, values)` in stable (sorted) order.
    pub curves: &'static [(&'static str, &'static [f64])],
    /// Curve name -> mechanism semantic, covering `curves` exactly.
    pub semantics: &'static [(&'static str, &'static str)],
}

/// The short psychoacoustic seed surface.
#[derive(Debug, Clone, Copy)]
pub struct ShortSeedTable {
    pub n: i64,
    pub sample_rate: i64,
    pub ath_offset: f32,
    pub ath_floor: f32,
    pub seed_ceiling: f32,
    pub max_curve_db: f32,
    pub curve_offset: f32,
    pub curve_slope: f32,
    pub curve_offset_2: f32,
    pub curve_attenuation: &'static [f32],
    pub ath: &'static [f32],
    pub octave: &'static [i64],
    pub first_octave: i64,
    pub shift_octave: i64,
    pub eighth_octave_lines: i64,
    pub total_octave_lines: i64,
    /// Flattened `17 * 8 * 58` tone table (band, level, record).
    pub tone_curves: &'static [f32],
    pub row: &'static [f32],
    pub envelope_low: &'static [f32],
    pub envelope_high: &'static [f32],
    pub mask_curve: &'static [f32],
    pub interval_table: &'static [i64],
    pub short_limit: i64,
    pub noise_fixed_window: i64,
    pub regular_curve_bias: f32,
    pub regular_curve_cap: f32,
    pub remap_curve_offsets: &'static [i64],
    pub remap_low_by_index: &'static [f32],
    pub remap_high_by_index: &'static [f32],
}

/// One short-block psychoacoustic profile. Its floats are f64, exactly as the
/// reference stores them.
#[derive(Debug, Clone, Copy)]
pub struct ShortProfileTable {
    pub key: &'static str,
    pub candidate_bias_by_mode: &'static [f64],
    pub curve_cap: f64,
    pub group_enabled: i64,
    pub candidate_bound: i64,
    pub group_span: i64,
    pub band_limits: [i64; 3],
    pub side_gain: f64,
    pub peak_cutoff: i64,
    pub blend_weight: f64,
    /// Flattened `3 * 128` mask curves.
    pub mask_curves: &'static [f64],
}

/// The fixed 1024-bin long psychoacoustic tables.
#[derive(Debug, Clone, Copy)]
pub struct LongTable {
    pub sample_rate: i64,
    pub n: i64,
    pub profile_key: &'static str,
    pub analysis_profile_u32: &'static [u32],
    pub analysis_interval_u32: &'static [u32],
    /// Flattened `3 * 1024` curves.
    pub analysis_curves: &'static [f32],
    pub analysis_field_19_curve: &'static [f32],
    pub seed_outer_u32: &'static [u32],
    pub seed_profile_u32: &'static [u32],
    pub seed_base_curve: &'static [f32],
    pub seed_group_labels_u32: &'static [u32],
    /// Flattened `17 * 8 * 58` tone banks.
    pub seed_tone_banks: &'static [f32],
}

/// One long analysis mode surface (the floor-seed half is shared with the
/// base table).
#[derive(Debug, Clone, Copy)]
pub struct LongVariantTable {
    pub mode: i64,
    pub analysis_profile_u32: &'static [u32],
    pub analysis_interval_u32: &'static [u32],
    /// Flattened `3 * 1024` curves.
    pub analysis_curves: &'static [f32],
    pub analysis_field_19_curve: &'static [f32],
}

/// Every typed table one profile carries.
#[derive(Debug, Clone, Copy)]
pub struct ResourceTables {
    pub mdct_banks: &'static [MdctBankTable],
    pub codebook_tables: &'static [CodebookTableRef],
    pub frozen: &'static FrozenTable,
    pub transient: TransientTable,
    pub quality_curves: Option<&'static QualityCurvesTable>,
    pub short_seed: &'static ShortSeedTable,
    pub short_profiles: &'static [ShortProfileTable],
    pub long_base: &'static LongTable,
    pub long_variants: &'static [LongVariantTable],
    /// DC-filter coefficient as an f32 bit pattern.
    pub input_conditioner: Option<u32>,
}

/// The complete compiled profile: identity, container geometry, setup packet
/// bytes and every typed table.
///
/// There is no stored profile name: the human label is derived from [`Self::key`]
/// (`ProfileKey::label`), so the carrier holds identity and values only.
#[derive(Debug, Clone, Copy)]
pub struct ProfileTables {
    pub key: ProfileKeyParts,
    pub setup_packet: &'static [u8],
    pub container_metadata: ContainerMetadata,
    pub block_sizes: [i64; 2],
    pub resources: ResourceTables,
}
