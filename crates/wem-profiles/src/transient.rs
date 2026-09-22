//! Immutable window and band configuration for transient detection
//! (Python: `wwise_wem_reference.profiles.transient`).
//!
//! The compiled carrier records the static bias/threshold record family from
//! the paired encoder build (`wem.transient-record-family.v1`). The assembly
//! layer materializes one detector table from it per quality value: the
//! record-index curve on the shared quality axis picks a (possibly
//! fractional) record index, the floor record supplies every field verbatim,
//! and only upper[0..3] / lower[0..3] are linearly interpolated between the
//! adjacent records at the fractional part.
//!
//! There is no document decoding left here: the carrier holds the decoded
//! words, and the only fallible step is materialization.

use wem_analysis::config::{TransientBandConfig, TransientDetectorTables};

use crate::error::ProfileError;
use crate::quality::{linear_frac, normalize_quality_factor};

// ---------------------------------------------------------------------------
// wem.transient-record-family.v1: the static record library from the paired
// encoder build, materialized per quality.
//
// Selection/interpolation mechanism (instruction-pinned by the the extraction
// extraction, the internal record, the corresponding locations): the whole record
// at v5 = floor(index) is copied into the runtime block first; only
// upper[0..3] and lower[0..3] are then overwritten with a linear
// interpolation between records v5 and v5+1 at the fractional part. So all
// non-interpolated fields (bias, carry, marker, tail, upper[4..11],
// lower[4..11]) come verbatim from the FLOOR record.
// ---------------------------------------------------------------------------

/// Record-family schema (Python `TRANSIENT_RECORD_FAMILY_SCHEMA`).
pub const TRANSIENT_RECORD_FAMILY_SCHEMA: &str = "wem.transient-record-family.v1";
/// Record count in the paired build.
pub const TRANSIENT_RECORD_COUNT: usize = 6;
/// Words per record (marker + upper 12 + lower 12 + carry).
pub const TRANSIENT_RECORD_WORDS: usize = 26;
/// Points on the shared quality axis (breakpoints / index curve).
pub const TRANSIENT_RECORD_INDEX_POINTS: usize = 13;
/// Detector table window length (n = 128).
pub const TRANSIENT_WINDOW_WORDS: u64 = 128;
/// Band count in the descriptor table.
pub const TRANSIENT_BAND_COUNT: usize = 12;

/// One stored bias/threshold record (u32 bit patterns, byte-exact)
/// (Python `TransientRecord`).
#[derive(Debug, Clone, PartialEq)]
pub struct TransientRecord {
    pub file_off: String,
    pub marker_u32: u32,
    pub upper_u32: [u32; 12],
    pub lower_u32: [u32; 12],
    pub carry_u32: u32,
    pub bias_u32: u32,
    pub m_u32: u32,
    pub tail_u32: u32,
    /// All 26 words in runtime order: marker, upper[0..12), lower[0..12),
    /// carry.
    pub config_u32: [u32; TRANSIENT_RECORD_WORDS],
}

/// The static record library plus its immutable detector surfaces
/// (Python `TransientRecordFamily`).
#[derive(Debug, Clone, PartialEq)]
pub struct TransientRecordFamily {
    pub schema: &'static str,
    pub n: u64,
    pub sample_rate: i64,
    /// Record index used when no quality value is given (adjudicated default:
    /// 3.0, the High band record of the paired build).
    pub default_record_index: f64,
    pub records: Vec<TransientRecord>,
    /// f64 record-index curve on the shared quality axis (13 points).
    pub index_curve: Vec<f64>,
    /// Shared quality-axis breakpoints (13 points, strictly increasing).
    pub breakpoints: Vec<f64>,
    /// Static detector window words from the paired build.
    pub window_u32: Vec<u32>,
    /// Static detector band descriptors from the paired build.
    pub bands: Vec<TransientBandConfig>,
}

/// Materialize one detector table from the record family for one quality
/// (Python `materialize_transient_tables`).
///
/// `quality` is the raw Wwise quality factor (0-10 convention) or `None`
/// for the profile's historical default gear: `None` materializes the
/// family's `default_record_index` record whole (the adjudicated default is
/// record 3). A quality value is normalized onto the shared quality axis,
/// read on the family's own record-index curve with the shared two-step
/// linear kernel, and mapped onto the family by the pinned selection rule:
///
/// * `v5 = floor(index)` (clamped to the last record at the top);
/// * every field except upper[0..3]/lower[0..3] comes verbatim from record
///   `v5` (bias, carry, marker, tail, upper[4..11], lower[4..11]);
/// * upper[0..3]/lower[0..3] are linearly interpolated between records
///   `v5` and `v5+1` at the fractional part (f64 lerp, f32 rounding).
///
/// The window and band surfaces are quality-independent static f32 words from
/// the paired build. Materialization only selects/interpolates record fields.
pub fn materialize_transient_tables(
    family: &TransientRecordFamily,
    quality: Option<f64>,
) -> Result<TransientDetectorTables, ProfileError> {
    let index = match quality {
        None => family.default_record_index,
        Some(quality) if !quality.is_finite() => return Err(ProfileError::QualityValueNonFinite),
        Some(quality) => {
            let (value, _outside) = linear_frac(
                &family.breakpoints,
                &family.index_curve,
                normalize_quality_factor(quality),
            );
            value
        }
    };
    let max_index = (family.records.len() - 1) as f64;
    let index = index.clamp(0.0, max_index);
    let v5 = index.floor() as usize;
    let v5 = v5.min(family.records.len() - 1);
    let frac = if v5 >= family.records.len() - 1 {
        0.0
    } else {
        index - v5 as f64
    };

    let base = &family.records[v5];
    let mut config_words: Vec<u32> = base.config_u32.to_vec();
    if frac > 0.0 {
        let nxt = &family.records[v5 + 1].config_u32;
        for j in 0..4u32 {
            for position in [1 + j, 13 + j] {
                let a = f32::from_bits(config_words[position as usize]) as f64;
                let b = f32::from_bits(nxt[position as usize]) as f64;
                let lerped = (1.0 - frac) * a + frac * b;
                config_words[position as usize] = (lerped as f32).to_bits(); // f32 boundary
            }
        }
    }

    Ok(TransientDetectorTables {
        n: family.n as i64,
        bias: f32::from_bits(base.bias_u32),
        window: family
            .window_u32
            .iter()
            .map(|bits| f32::from_bits(*bits))
            .collect(),
        config: config_words
            .iter()
            .map(|bits| f32::from_bits(*bits))
            .collect(),
        bands: family.bands.clone(),
    })
}
