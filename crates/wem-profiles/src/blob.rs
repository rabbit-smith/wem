//! The profile-table dump: the compiled carrier, handed to other languages.
//!
//! The kernel is the single owner of profile data ([`crate::generated`]), and
//! every other language reads it from the compiled artifact rather than
//! carrying a copy. This module defines that hand-off: one canonical,
//! little-endian, self-describing byte stream holding every compiled profile's
//! typed tables.
//!
//! Rules that make it a carrier and not a re-encoding:
//!
//! * floats travel as their stored IEEE bit patterns, never as decimal text;
//! * integer tables keep the width the codec reads them at (`u32` words stay
//!   `u32`, `i64` fields stay `i64`) — nothing is re-derived on the way out;
//! * the format is versioned and total: a reader that does not recognize a
//!   version rejects the stream instead of guessing.
//!
//! The primary consumer is the pure-Python reference implementation, which is
//! the specification of the encoder's *algorithm and ordering*; the profile
//! values are external facts measured from the paired build, and this stream
//! is how those facts reach it. `docs/findings/profile-as-code.md` records the
//! reasoning and the boundary of that relationship.

use crate::generated;
use crate::tables::{ProfileTables, ResourceTables, TransientTable};

/// Stream magic (`WEMPROF\0`).
pub const BLOB_MAGIC: &[u8; 8] = b"WEMPROF\0";
/// Stream format version.
///
/// Version 2 dropped the stored profile name: the human label is derived from
/// the identity (`ProfileKey::label`), so the stream carries identity and
/// values only. Version 3 dropped the stored setup digest: the key's identity
/// names the packet and the packet's own bytes travel in the stream, so a
/// second copy of its SHA-256 proved nothing.
pub const BLOB_VERSION: u32 = 3;

const KIND_U32: u8 = 0;
const KIND_I64: u8 = 1;
const KIND_F32: u8 = 2;
const KIND_F64: u8 = 3;
const KIND_STR: u8 = 4;
const KIND_U64: u8 = 6;

/// One little-endian writer for the dump.
struct Blob {
    out: Vec<u8>,
    blocks: u32,
}

impl Blob {
    fn new() -> Self {
        Self {
            out: Vec::with_capacity(1 << 20),
            blocks: 0,
        }
    }

    fn u8(&mut self, value: u8) {
        self.out.push(value);
    }

    fn u32(&mut self, value: u32) {
        self.out.extend_from_slice(&value.to_le_bytes());
    }

    fn i64(&mut self, value: i64) {
        self.out.extend_from_slice(&value.to_le_bytes());
    }

    fn text(&mut self, value: &str) {
        self.u32(value.len() as u32);
        self.out.extend_from_slice(value.as_bytes());
    }

    fn raw(&mut self, value: &[u8]) {
        self.u32(value.len() as u32);
        self.out.extend_from_slice(value);
    }

    /// Open one named table block.
    fn block(&mut self, key: &str, kind: u8, count: usize) {
        self.blocks += 1;
        self.text(key);
        self.u8(kind);
        self.u32(count as u32);
    }
}

fn u32_block(out: &mut Blob, key: &str, values: &[u32]) {
    out.block(key, KIND_U32, values.len());
    for value in values {
        out.u32(*value);
    }
}

fn i64_block(out: &mut Blob, key: &str, values: &[i64]) {
    out.block(key, KIND_I64, values.len());
    for value in values {
        out.i64(*value);
    }
}

fn f32_block(out: &mut Blob, key: &str, values: &[f32]) {
    out.block(key, KIND_F32, values.len());
    for value in values {
        out.u32(value.to_bits());
    }
}

fn f64_block(out: &mut Blob, key: &str, values: &[f64]) {
    out.block(key, KIND_F64, values.len());
    for value in values {
        out.out.extend_from_slice(&value.to_bits().to_le_bytes());
    }
}

fn str_block(out: &mut Blob, key: &str, value: &str) {
    // The block header already carries the length, so the payload is the raw
    // UTF-8 bytes: a reader never has to re-read a length prefix.
    out.block(key, KIND_STR, value.len());
    out.out.extend_from_slice(value.as_bytes());
}

fn u64_block(out: &mut Blob, key: &str, values: &[u64]) {
    out.block(key, KIND_U64, values.len());
    for value in values {
        out.out.extend_from_slice(&value.to_le_bytes());
    }
}

/// Render every compiled profile into the canonical dump.
///
/// Deterministic: the profile order is the carrier's order and every block
/// follows the carrier's field order, so two calls produce identical bytes.
pub fn profile_tables_blob() -> Vec<u8> {
    let mut out = Blob::new();
    out.out.extend_from_slice(BLOB_MAGIC);
    out.u32(BLOB_VERSION);
    out.u32(generated::PROFILES.len() as u32);
    for tables in generated::PROFILES {
        write_profile(&mut out, tables);
    }
    out.out
}

fn write_profile(out: &mut Blob, tables: &ProfileTables) {
    out.text(tables.key.generation);
    out.text(tables.key.channel_layout);
    out.text(tables.key.quality_setup_identity);
    out.i64(tables.key.channels);
    out.i64(tables.key.sample_rate);
    out.i64(tables.block_sizes[0]);
    out.i64(tables.block_sizes[1]);
    out.raw(tables.setup_packet);

    // Table blocks go into their own section first: the header carries how
    // many there are, so a reader never has to guess where one profile's
    // tables end.
    let mut section = Blob::new();
    {
        let out = &mut section;

        let metadata = &tables.container_metadata;
        let container: [i64; 22] = [
            metadata.w_format_tag,
            metadata.n_channels,
            metadata.n_samples_per_sec,
            metadata.n_avg_bytes_per_sec,
            metadata.n_block_align,
            metadata.w_bits_per_sample,
            metadata.cb_size,
            metadata.w_reserved0,
            metadata.dw_channel_mask,
            metadata.dw_total_pcm_frames,
            metadata.dw_first_audio_packet_offset,
            metadata.dw_data_payload_size,
            metadata.dw_unknown_0x24,
            metadata.dw_seek_table_size,
            metadata.dw_vorbis_data_offset,
            metadata.u_max_packet_size,
            metadata.u_unknown_0x32,
            metadata.dw_unknown_0x34,
            metadata.dw_unknown_0x38,
            metadata.dw_unknown_0x3c,
            metadata.u_blocksize0_pow,
            metadata.u_blocksize1_pow,
        ];
        i64_block(out, "container.fields", &container);

        let resources: &ResourceTables = &tables.resources;
        i64_block(out, "mdct.count", &[resources.mdct_banks.len() as i64]);
        for bank in resources.mdct_banks {
            str_block(out, &format!("mdct.{}.key", bank.n), &bank.n.to_string());
            f32_block(out, &format!("mdct.{}.trig", bank.n), bank.trig);
        }

        i64_block(
            out,
            "codebook.count",
            &[resources.codebook_tables.len() as i64],
        );
        for table in resources.codebook_tables {
            let name = table.name;
            str_block(out, &format!("codebook.{name}.name"), name);
            let rows = table.rows;
            i64_block(
                out,
                &format!("codebook.{name}.dim"),
                &rows.iter().map(|row| row.dim).collect::<Vec<_>>(),
            );
            i64_block(
                out,
                &format!("codebook.{name}.entries"),
                &rows.iter().map(|row| row.entries).collect::<Vec<_>>(),
            );
            i64_block(
                out,
                &format!("codebook.{name}.maptype"),
                &rows.iter().map(|row| row.maptype).collect::<Vec<_>>(),
            );
            i64_block(
                out,
                &format!("codebook.{name}.q_min"),
                &rows.iter().map(|row| row.q_min).collect::<Vec<_>>(),
            );
            i64_block(
                out,
                &format!("codebook.{name}.q_delta"),
                &rows.iter().map(|row| row.q_delta).collect::<Vec<_>>(),
            );
            i64_block(
                out,
                &format!("codebook.{name}.q_quant"),
                &rows.iter().map(|row| row.q_quant).collect::<Vec<_>>(),
            );
            i64_block(
                out,
                &format!("codebook.{name}.q_sequencep"),
                &rows.iter().map(|row| row.q_sequencep).collect::<Vec<_>>(),
            );
            u32_block(
                out,
                &format!("codebook.{name}.i_present"),
                &rows
                    .iter()
                    .map(|row| u32::from(row.i.is_some()))
                    .collect::<Vec<_>>(),
            );
            i64_block(
                out,
                &format!("codebook.{name}.i"),
                &rows
                    .iter()
                    .map(|row| row.i.unwrap_or(0))
                    .collect::<Vec<_>>(),
            );
            u32_block(
                out,
                &format!("codebook.{name}.quantvals_present"),
                &rows
                    .iter()
                    .map(|row| u32::from(row.quantvals.is_some()))
                    .collect::<Vec<_>>(),
            );
            i64_block(
                out,
                &format!("codebook.{name}.quantvals"),
                &rows
                    .iter()
                    .map(|row| row.quantvals.unwrap_or(0))
                    .collect::<Vec<_>>(),
            );
            let (lengthlist, lengthlist_offsets, lengthlist_present) =
                flatten_optional(rows, |row| row.lengthlist);
            i64_block(out, &format!("codebook.{name}.lengthlist"), &lengthlist);
            u32_block(
                out,
                &format!("codebook.{name}.lengthlist_offsets"),
                &lengthlist_offsets,
            );
            u32_block(
                out,
                &format!("codebook.{name}.lengthlist_present"),
                &lengthlist_present,
            );
            let (quantlist, quantlist_offsets, quantlist_present) =
                flatten_optional(rows, |row| row.quantlist);
            i64_block(out, &format!("codebook.{name}.quantlist"), &quantlist);
            u32_block(
                out,
                &format!("codebook.{name}.quantlist_offsets"),
                &quantlist_offsets,
            );
            u32_block(
                out,
                &format!("codebook.{name}.quantlist_present"),
                &quantlist_present,
            );
        }

        let frozen = resources.frozen;
        u64_block(
            out,
            "frozen.coordinate_ln.in_bits",
            &frozen
                .coordinate_ln
                .iter()
                .map(|pair| pair.0)
                .collect::<Vec<_>>(),
        );
        u64_block(
            out,
            "frozen.coordinate_ln.out_bits",
            &frozen
                .coordinate_ln
                .iter()
                .map(|pair| pair.1)
                .collect::<Vec<_>>(),
        );
        i64_block(
            out,
            "frozen.fft_twiddles.length",
            &frozen
                .fft_twiddles
                .iter()
                .map(|entry| entry.0)
                .collect::<Vec<_>>(),
        );
        u64_block(
            out,
            "frozen.fft_twiddles.cos_bits",
            &frozen
                .fft_twiddles
                .iter()
                .map(|entry| entry.1)
                .collect::<Vec<_>>(),
        );
        u64_block(
            out,
            "frozen.fft_twiddles.sin_bits",
            &frozen
                .fft_twiddles
                .iter()
                .map(|entry| entry.2)
                .collect::<Vec<_>>(),
        );
        i64_block(
            out,
            "frozen.window_halves.size",
            &frozen
                .window_halves
                .iter()
                .map(|entry| entry.0)
                .collect::<Vec<_>>(),
        );
        let mut halves: Vec<f32> = Vec::new();
        let mut half_offsets: Vec<u32> = vec![0];
        for entry in frozen.window_halves {
            halves.extend_from_slice(entry.1);
            half_offsets.push(halves.len() as u32);
        }
        f32_block(out, "frozen.window_halves.words", &halves);
        u32_block(out, "frozen.window_halves.offsets", &half_offsets);

        match &resources.transient {
            TransientTable::Detector(detector) => {
                str_block(out, "transient.kind", "detector");
                i64_block(out, "transient.n", &[detector.n]);
                f32_block(out, "transient.bias", &[detector.bias]);
                f32_block(out, "transient.window", detector.window);
                f32_block(out, "transient.config", detector.config);
                write_bands(out, "transient.bands", detector.bands);
            }
            TransientTable::RecordFamily(family) => {
                str_block(out, "transient.kind", "record-family");
                i64_block(out, "transient.n", &[family.n]);
                i64_block(out, "transient.sample_rate", &[family.sample_rate]);
                f64_block(
                    out,
                    "transient.default_record_index",
                    &[family.default_record_index],
                );
                f64_block(out, "transient.record_index_curve", family.index_curve);
                f64_block(
                    out,
                    "transient.quality_axis_breakpoints",
                    family.breakpoints,
                );
                f32_block(out, "transient.window", family.window);
                write_bands(out, "transient.bands", family.bands);
                for (index, record) in family.records.iter().enumerate() {
                    str_block(
                        out,
                        &format!("transient.record.{index}.file_off"),
                        record.file_off,
                    );
                    u32_block(
                        out,
                        &format!("transient.record.{index}.marker_u32"),
                        &[record.marker_u32],
                    );
                    u32_block(
                        out,
                        &format!("transient.record.{index}.upper_u32"),
                        &record.upper_u32,
                    );
                    u32_block(
                        out,
                        &format!("transient.record.{index}.lower_u32"),
                        &record.lower_u32,
                    );
                    u32_block(
                        out,
                        &format!("transient.record.{index}.carry_u32"),
                        &[record.carry_u32],
                    );
                    u32_block(
                        out,
                        &format!("transient.record.{index}.bias_u32"),
                        &[record.bias_u32],
                    );
                    u32_block(
                        out,
                        &format!("transient.record.{index}.m_u32"),
                        &[record.m_u32],
                    );
                    u32_block(
                        out,
                        &format!("transient.record.{index}.tail_u32"),
                        &[record.tail_u32],
                    );
                    u32_block(
                        out,
                        &format!("transient.record.{index}.config_u32"),
                        &record.config_u32,
                    );
                }
            }
        }

        match resources.quality_curves {
            Some(curves) => {
                f64_block(out, "quality_curves.breakpoints", curves.breakpoints);
                for (name, values) in curves.curves {
                    f64_block(out, &format!("quality_curves.curve.{name}"), values);
                }
                for (name, semantic) in curves.semantics {
                    str_block(out, &format!("quality_curves.semantic.{name}"), semantic);
                }
            }
            None => {
                f64_block(out, "quality_curves.breakpoints", &[]);
            }
        }

        let seed = resources.short_seed;
        i64_block(out, "short_seed.n", &[seed.n]);
        i64_block(out, "short_seed.sample_rate", &[seed.sample_rate]);
        f32_block(out, "short_seed.ath_offset", &[seed.ath_offset]);
        f32_block(out, "short_seed.ath_floor", &[seed.ath_floor]);
        f32_block(out, "short_seed.seed_ceiling", &[seed.seed_ceiling]);
        f32_block(out, "short_seed.max_curve_db", &[seed.max_curve_db]);
        f32_block(out, "short_seed.curve_offset", &[seed.curve_offset]);
        f32_block(out, "short_seed.curve_slope", &[seed.curve_slope]);
        f32_block(out, "short_seed.curve_offset_2", &[seed.curve_offset_2]);
        f32_block(out, "short_seed.curve_attenuation", seed.curve_attenuation);
        f32_block(out, "short_seed.ath", seed.ath);
        i64_block(out, "short_seed.octave", seed.octave);
        i64_block(out, "short_seed.first_octave", &[seed.first_octave]);
        i64_block(out, "short_seed.shift_octave", &[seed.shift_octave]);
        i64_block(
            out,
            "short_seed.eighth_octave_lines",
            &[seed.eighth_octave_lines],
        );
        i64_block(
            out,
            "short_seed.total_octave_lines",
            &[seed.total_octave_lines],
        );
        f32_block(out, "short_seed.tone_curves", seed.tone_curves);
        f32_block(out, "short_seed.row", seed.row);
        f32_block(out, "short_seed.envelope_low", seed.envelope_low);
        f32_block(out, "short_seed.envelope_high", seed.envelope_high);
        f32_block(out, "short_seed.mask_curve", seed.mask_curve);
        i64_block(out, "short_seed.interval_table", seed.interval_table);
        i64_block(out, "short_seed.short_limit", &[seed.short_limit]);
        i64_block(
            out,
            "short_seed.noise_fixed_window",
            &[seed.noise_fixed_window],
        );
        f32_block(
            out,
            "short_seed.regular_curve_bias",
            &[seed.regular_curve_bias],
        );
        f32_block(
            out,
            "short_seed.regular_curve_cap",
            &[seed.regular_curve_cap],
        );
        i64_block(
            out,
            "short_seed.remap_curve_offsets",
            seed.remap_curve_offsets,
        );
        f32_block(
            out,
            "short_seed.remap_low_by_index",
            seed.remap_low_by_index,
        );
        f32_block(
            out,
            "short_seed.remap_high_by_index",
            seed.remap_high_by_index,
        );

        i64_block(
            out,
            "short_profiles.count",
            &[resources.short_profiles.len() as i64],
        );
        for (index, profile) in resources.short_profiles.iter().enumerate() {
            str_block(out, &format!("short_profiles.{index}.key"), profile.key);
            f64_block(
                out,
                &format!("short_profiles.{index}.candidate_bias_by_mode"),
                profile.candidate_bias_by_mode,
            );
            f64_block(
                out,
                &format!("short_profiles.{index}.curve_cap"),
                &[profile.curve_cap],
            );
            i64_block(
                out,
                &format!("short_profiles.{index}.group_enabled"),
                &[profile.group_enabled],
            );
            i64_block(
                out,
                &format!("short_profiles.{index}.candidate_bound"),
                &[profile.candidate_bound],
            );
            i64_block(
                out,
                &format!("short_profiles.{index}.group_span"),
                &[profile.group_span],
            );
            i64_block(
                out,
                &format!("short_profiles.{index}.band_limits"),
                &profile.band_limits,
            );
            f64_block(
                out,
                &format!("short_profiles.{index}.side_gain"),
                &[profile.side_gain],
            );
            i64_block(
                out,
                &format!("short_profiles.{index}.peak_cutoff"),
                &[profile.peak_cutoff],
            );
            f64_block(
                out,
                &format!("short_profiles.{index}.blend_weight"),
                &[profile.blend_weight],
            );
            f64_block(
                out,
                &format!("short_profiles.{index}.mask_curves"),
                profile.mask_curves,
            );
        }

        let long = resources.long_base;
        i64_block(out, "long_base.sample_rate", &[long.sample_rate]);
        i64_block(out, "long_base.n", &[long.n]);
        str_block(out, "long_base.profile_key", long.profile_key);
        u32_block(
            out,
            "long_base.analysis_profile_u32",
            long.analysis_profile_u32,
        );
        u32_block(
            out,
            "long_base.analysis_interval_u32",
            long.analysis_interval_u32,
        );
        f32_block(out, "long_base.analysis_curves", long.analysis_curves);
        f32_block(
            out,
            "long_base.analysis_field_19_curve",
            long.analysis_field_19_curve,
        );
        u32_block(out, "long_base.seed_outer_u32", long.seed_outer_u32);
        u32_block(out, "long_base.seed_profile_u32", long.seed_profile_u32);
        f32_block(out, "long_base.seed_base_curve", long.seed_base_curve);
        u32_block(
            out,
            "long_base.seed_group_labels_u32",
            long.seed_group_labels_u32,
        );
        f32_block(out, "long_base.seed_tone_banks", long.seed_tone_banks);

        i64_block(
            out,
            "long_variants.count",
            &[resources.long_variants.len() as i64],
        );
        for variant in resources.long_variants {
            let mode = variant.mode;
            i64_block(out, &format!("long_variants.{mode}.mode"), &[mode]);
            u32_block(
                out,
                &format!("long_variants.{mode}.analysis_profile_u32"),
                variant.analysis_profile_u32,
            );
            u32_block(
                out,
                &format!("long_variants.{mode}.analysis_interval_u32"),
                variant.analysis_interval_u32,
            );
            f32_block(
                out,
                &format!("long_variants.{mode}.analysis_curves"),
                variant.analysis_curves,
            );
            f32_block(
                out,
                &format!("long_variants.{mode}.analysis_field_19_curve"),
                variant.analysis_field_19_curve,
            );
        }

        match resources.input_conditioner {
            Some(bits) => u32_block(out, "input_conditioner.bits", &[bits]),
            None => u32_block(out, "input_conditioner.bits", &[]),
        }
    }

    out.u32(section.blocks);
    out.out.extend_from_slice(&section.out);
}

fn write_bands(out: &mut Blob, prefix: &str, bands: &[crate::tables::BandTable]) {
    i64_block(out, &format!("{prefix}.count"), &[bands.len() as i64]);
    for (index, band) in bands.iter().enumerate() {
        i64_block(out, &format!("{prefix}.{index}.offset"), &[band.offset]);
        f32_block(out, &format!("{prefix}.{index}.weights"), band.weights);
        f32_block(out, &format!("{prefix}.{index}.scale"), &[band.scale]);
    }
}

/// Flatten one optional per-row list into `(values, offsets, present)`.
fn flatten_optional(
    rows: &'static [crate::tables::CodebookRowTable],
    select: impl Fn(&crate::tables::CodebookRowTable) -> Option<&'static [i64]>,
) -> (Vec<i64>, Vec<u32>, Vec<u32>) {
    let mut values: Vec<i64> = Vec::new();
    let mut offsets: Vec<u32> = vec![0];
    let mut present: Vec<u32> = Vec::with_capacity(rows.len());
    for row in rows {
        match select(row) {
            Some(list) => {
                values.extend_from_slice(list);
                present.push(1);
            }
            None => present.push(0),
        }
        offsets.push(values.len() as u32);
    }
    (values, offsets, present)
}
