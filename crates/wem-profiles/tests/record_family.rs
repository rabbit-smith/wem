//! The 2ch/48000 transient record family: materialization, selection rule,
//! and cross-implementation parity pins.
//!
//! The expected f64 record indices and f32 bit patterns below are shared
//! test vectors: the Python reference
//! (reference/wwise_wem_reference/profiles/transient.py,
//! tests/unit/profiles/test_transient_record_family.py) and this module must
//! agree bit for bit. They pin the mechanism adjudicated from the paired
//! encoder build: the record-index curve on the shared quality axis picks a
//! (possibly fractional) index; the floor record supplies every field
//! verbatim; only upper[0..3]/lower[0..3] are interpolated between the
//! adjacent records at the fractional part (f64 lerp, f32 rounding).

#![allow(clippy::excessive_precision)]

use wem_profiles::transient::{load_transient, load_transient_record_family};
use wem_profiles::{linear_frac, load_profile_bundle, normalize_quality_factor, DataDir};

fn data_dir() -> DataDir {
    DataDir::from_profiles_dir(
        std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../..")
            .join("src/wwise_wem/data/profiles")
            .canonicalize()
            .expect("profiles directory resolves"),
    )
}

fn family() -> wem_profiles::transient::TransientRecordFamily {
    let bundle = load_profile_bundle(&data_dir(), Some("wwise2013-2ch-48000"), false)
        .expect("2ch bundle loads");
    let ref_ = bundle
        .runtime_manifest()
        .resource("analysis.transient")
        .expect("transient resource");
    load_transient_record_family(ref_).expect("record family loads")
}

fn f32_bits(v: &f32) -> u32 {
    v.to_bits()
}

#[test]
fn family_structure_matches_the_static_extraction() {
    let fam = family();
    assert_eq!(fam.schema, "wem.transient-record-family.v1");
    assert_eq!(fam.n, 128);
    assert_eq!(fam.sample_rate, 48000);
    assert_eq!(fam.records.len(), 6);
    assert_eq!(fam.index_curve.len(), 13);
    assert_eq!(fam.breakpoints.len(), 13);
    assert_eq!(fam.window_u32.len(), 128);
    assert_eq!(fam.bands.len(), 12);
    // Adjudicated default: record 3 (the High band of the paired build).
    assert_eq!(fam.default_record_index, 3.0);
    // Record stride / offsets as extracted.
    assert_eq!(fam.records[0].file_off, "0xc6b68");
    assert_eq!(fam.records[1].file_off, "0xc6d7c");
    // Provenance words outside the 26-word config block stay byte-faithful
    // to the static extraction (m = f32 -6.0 = 0xC0C00000, tail = 99).
    for record in &fam.records {
        assert_eq!(record.m_u32, 0xC0C0_0000);
        assert_eq!(record.tail_u32, 99);
    }
}

#[test]
fn record_index_curve_pins_match_the_python_reference() {
    let fam = family();
    // Shared f64 pins: (quality, exact index double).
    let pins: &[(f64, f64)] = &[
        (-1.0, 1.0000010000000001),
        (0.0, 2.0),
        (1.0, 2.0000005),
        (2.0, 2.5000005),
        (4.0, 3.0000005),
        (5.5, 3.6000002),
        (7.0, 4.0),
        (8.0, 4.000000999999999),
        (10.0, 5.0),
    ];
    for (quality, expected) in pins {
        let (value, _outside) = linear_frac(
            &fam.breakpoints,
            &fam.index_curve,
            normalize_quality_factor(*quality),
        );
        assert_eq!(
            value, *expected,
            "record index at q={quality} differs from the Python pin"
        );
    }
    // No quality -> the adjudicated default index record #3.
    assert_eq!(fam.default_record_index, 3.0);
}

#[test]
fn default_quality_materializes_record_three_whole() {
    let fam = family();
    let tables = load_transient_tables_from(&fam, None).expect("materialize");
    // Every field verbatim from record 3 (u32 bits, byte-exact).
    let rec3 = &fam.records[3];
    assert_eq!(f32_bits(&tables.bias), rec3.bias_u32);
    assert_eq!(
        tables.config.iter().map(f32_bits).collect::<Vec<_>>(),
        rec3.config_u32.to_vec()
    );
}

#[test]
fn quality_values_materialize_the_pinned_config_words() {
    let fam = family();
    // q = 4.0 -> index 3.0000005: record 3 base + a 5e-7 lerp toward record 4
    // on upper[0..3]/lower[0..3] only.
    let t_q4 = load_transient_tables_from(&fam, Some(4.0)).expect("q4 materialize");
    assert_eq!(f32_bits(&t_q4.config[1]), 1094713342);
    assert_eq!(f32_bits(&t_q4.config[2]), 1092616191);
    assert_eq!(f32_bits(&t_q4.config[3]), 1092616191);
    assert_eq!(f32_bits(&t_q4.config[4]), 1092616190);
    assert_eq!(f32_bits(&t_q4.config[13]), 3248488448);
    assert_eq!(f32_bits(&t_q4.config[14]), 3248488447);
    assert_eq!(f32_bits(&t_q4.config[15]), 3245342718);
    assert_eq!(f32_bits(&t_q4.config[16]), 3245342718);
    // Non-interpolated fields stay verbatim from record 3.
    let rec3 = &fam.records[3];
    assert_eq!(f32_bits(&t_q4.bias), rec3.bias_u32);
    for position in 5..13usize {
        assert_eq!(f32_bits(&t_q4.config[position]), rec3.config_u32[position]);
    }
    for position in 17..25usize {
        assert_eq!(f32_bits(&t_q4.config[position]), rec3.config_u32[position]);
    }
    assert_eq!(f32_bits(&t_q4.config[0]), rec3.marker_u32);
    assert_eq!(f32_bits(&t_q4.config[25]), rec3.carry_u32);

    // q = 1.0 -> index 2.0000005: record 2 base + tiny lerp toward record 3.
    let t_q1 = load_transient_tables_from(&fam, Some(1.0)).expect("q1 materialize");
    assert_eq!(f32_bits(&t_q1.config[1]), 1096810495);

    // q = 7.0 -> index 4.0 exactly: record 4 whole.
    let t_q7 = load_transient_tables_from(&fam, Some(7.0)).expect("q7 materialize");
    let rec4 = &fam.records[4];
    assert_eq!(f32_bits(&t_q7.config[1]), rec4.config_u32[1]);
    assert_eq!(f32_bits(&t_q7.config[2]), rec4.config_u32[2]);
    assert_eq!(f32_bits(&t_q7.config[13]), rec4.config_u32[13]);

    // q = 10.0 -> index 5.0 exactly: record 5 whole (top clamps).
    let t_q10 = load_transient_tables_from(&fam, Some(10.0)).expect("q10 materialize");
    let rec5 = &fam.records[5];
    assert_eq!(f32_bits(&t_q10.config[1]), rec5.config_u32[1]);
    assert_eq!(f32_bits(&t_q10.bias), rec5.bias_u32);
}

#[test]
fn fractional_index_20_interpolates_between_records_two_and_three() {
    let fam = family();
    // q = 2.0 -> index 2.5000005: record 2 base, upper[0..3]/lower[0..3]
    // lerped halfway to record 3.
    let t = load_transient_tables_from(&fam, Some(2.0)).expect("materialize");
    let rec2 = &fam.records[2];
    let rec3 = &fam.records[3];
    // Bias still verbatim from the floor record (2).
    assert_eq!(f32_bits(&t.bias), rec2.bias_u32);
    // Interpolated word must sit strictly between the two records' values.
    let a = f32::from_bits(rec2.config_u32[1]) as f64;
    let b = f32::from_bits(rec3.config_u32[1]) as f64;
    let got = f32::from_bits(f32_bits(&t.config[1])) as f64;
    assert!(
        got < a && got > b,
        "expected an interior lerp: {got} in ({b}, {a})"
    );
}

#[test]
fn materialized_window_matches_the_six_ch_registered_window() {
    // The record-family carries the paired build's static window words,
    // including the [127] endpoint 0x2809aded.
    let fam = family();
    let t2 = load_transient_tables_from(&fam, None).expect("2ch materialize");

    let b6 = load_profile_bundle(&data_dir(), Some("wwise2013-6ch-44100"), false)
        .expect("6ch bundle loads");
    let ref6 = b6
        .runtime_manifest()
        .resource("analysis.transient")
        .expect("6ch transient resource");
    let t6 = load_transient(ref6, None).expect("6ch load");

    assert_eq!(t2.window.len(), 128);
    let same = t2
        .window
        .iter()
        .zip(&t6.window)
        .all(|(a, b)| a.to_bits() == b.to_bits());
    assert!(
        same,
        "2ch materialized window != 6ch registered window bit-for-bit"
    );
    // And the endpoint pin.
    assert_eq!(f32_bits(&t2.window[127]), 0x2809aded);
}

#[test]
fn band_weights_pins_match_the_python_reference() {
    let fam = family();
    let t = load_transient_tables_from(&fam, None).expect("materialize");
    // Band 0 weight[0] and band 0 scale (shared f32 bits pins).
    assert_eq!(f32_bits(&t.bands[0].weights[0]), 1053028118);
    // Twelve bands match the stored static descriptors.
    assert_eq!(t.bands.len(), 12);
    for (band, stored) in t.bands.iter().zip(&fam.bands) {
        assert_eq!(band, stored);
    }
}

/// Materialize from a loaded family (keeps the tests off the manifest path
/// while still exercising the pinned kernel).
fn load_transient_tables_from(
    fam: &wem_profiles::transient::TransientRecordFamily,
    quality: Option<f64>,
) -> Result<wem_analysis::config::TransientDetectorTables, wem_profiles::ProfileError> {
    wem_profiles::transient::materialize_transient_tables(fam, quality)
}
