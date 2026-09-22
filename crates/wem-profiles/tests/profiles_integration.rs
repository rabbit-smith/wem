//! Integration tests for `wem-profiles`: the compiled carrier, the bit-exact
//! oracles from the Python reference, and the resolution rules.
//!
//! The profile every one of these tests wants is obtained with
//! [`wem_profiles::compiled_profile_for_selection`] — the one public,
//! name/path/bytes-free entry: it resolves a structured [`WwiseProfile`]
//! against the profile tables compiled into the library. Nothing here names a
//! profile, a profile directory or index bytes, and there is no second source
//! to compare against: the values below are the oracles, pinned as bit
//! patterns.

use wem_profiles::carrier::CompiledProfile;
use wem_profiles::{
    assemble_encoder_profile_resources, compiled_profile_for_selection, resolve_book_id,
    resolve_wem_profile_selection, ProfileKey, T219_COUNT, T282_COUNT, T97_COUNT,
};
use wem_vorbis::setup::{pack_setup, parse_setup};

mod common;

use common::{six_selection, two_channel_selection};

fn profile() -> CompiledProfile {
    compiled_profile_for_selection(six_selection()).expect("installed profile resolves")
}

// ---------------------------------------------------------------------------
// Resolution against the compiled carrier
// ---------------------------------------------------------------------------

#[test]
fn the_carrier_agrees_with_the_registry_resolution() {
    let compiled = profile();
    // The identity is cross-checked between the two resolution paths, never
    // re-typed as a profile name literal; the setup packet both paths hand
    // back is compared as bytes.
    let installed = resolve_wem_profile_selection(six_selection()).expect("6ch/44100 resolves");
    assert_eq!(compiled.label(), installed.label());
    assert_eq!(compiled.key(), installed.key());
    assert_eq!(
        (compiled.key().channels(), compiled.key().sample_rate()),
        (6, 44100)
    );
    assert_eq!(compiled.block_sizes(), [256, 2048]);
    assert_eq!(
        compiled.setup_packet().expect("setup packet"),
        installed.setup_packet().expect("setup packet")
    );
    // The carrier never produces a draft profile: every compiled profile
    // carries its setup packet.
    let model = compiled.encoder_profile().expect("encoder profile");
    assert!(model.setup_available());
    assert!(model.pending_reason().is_none());
}

#[test]
fn setup_parse_and_roundtrip() {
    let packet = profile().setup_packet().expect("setup packet");
    assert_eq!(packet.len(), 201);
    let info = parse_setup(&packet, 6).expect("setup parses");
    assert_eq!(info.nbooks, 39);
    assert_eq!(info.nfloors, 2);
    assert_eq!(info.nresidues, 2);
    assert_eq!(info.nmaps, 2);
    assert_eq!(info.nmodes, 2);
    assert_eq!(info.bit_positions.end, 1608);
    assert_eq!(info.trailing_pad_bits, 2);
    assert!(info.parse_complete);
    assert_eq!(&info.book_ids[..3], [38, 39, 40]);
    assert_eq!(*info.book_ids.last().unwrap(), 213);
    // Repack -> identical bytes (Python round-trip oracle).
    assert_eq!(pack_setup(&info).expect("setup repacks"), packet);
}

#[test]
fn book_id_resolution_oracle() {
    let tables = profile().book_tables().expect("book tables");
    let t97 = tables.get("t97").expect("t97 installed");
    let t219 = tables.get("t219").expect("t219 installed");
    assert_eq!(t97.rows().len(), T97_COUNT);
    assert_eq!(t219.rows().len(), T219_COUNT);

    assert_eq!(resolve_book_id(38, &tables).unwrap().table, "t97");
    assert_eq!(resolve_book_id(38, &tables).unwrap().index, 38);
    assert_eq!(resolve_book_id(96, &tables).unwrap().table, "t97");
    assert_eq!(resolve_book_id(97, &tables).unwrap().table, "t219");
    assert_eq!(resolve_book_id(97, &tables).unwrap().index, 0);
    assert_eq!(resolve_book_id(315, &tables).unwrap().table, "t219");
    assert_eq!(resolve_book_id(315, &tables).unwrap().index, 218);
    assert!(resolve_book_id(-1, &tables).is_err());
    assert!(resolve_book_id(316, &tables).is_err());
}

#[test]
fn two_channel_profile_resolves_t282_books() {
    // The 2ch/48k profile's setup references t97 floor books plus t282
    // residue books (IDs 316..597); resolution must attribute them to the
    // t282 table rather than reject them as out-of-range.
    let compiled =
        compiled_profile_for_selection(two_channel_selection()).expect("2ch profile resolves");
    let tables = compiled.book_tables().expect("book tables");
    let t282 = tables.get("t282").expect("t282 installed");
    assert_eq!(t282.rows().len(), T282_COUNT);
    assert!(tables.get("t219").is_none());
    // Floor book 42 -> t97[42]; residue book 416 -> t282[100]; book 597 ->
    // t282[281]; the t219 range (97..315) is absent and must be rejected.
    assert_eq!(resolve_book_id(42, &tables).unwrap().table, "t97");
    assert_eq!(resolve_book_id(416, &tables).unwrap().table, "t282");
    assert_eq!(resolve_book_id(416, &tables).unwrap().index, 100);
    assert_eq!(resolve_book_id(597, &tables).unwrap().table, "t282");
    assert_eq!(resolve_book_id(597, &tables).unwrap().index, 281);
    assert!(resolve_book_id(100, &tables).is_err());
    // Every book the setup references must attribute to an installed table.
    let setup = parse_setup(&compiled.setup_packet().unwrap(), 2).unwrap();
    for &book_id in setup.book_ids.iter() {
        assert!(
            resolve_book_id(book_id as i64, &tables).is_ok(),
            "book {book_id} resolves"
        );
    }
}

#[test]
fn codebook_oracle_values() {
    let tables = profile().book_tables().expect("book tables");
    let t97 = tables.get("t97").expect("t97 installed");
    let t219 = tables.get("t219").expect("t219 installed");

    // Book 38 (t97[38]): dim=1, entries=8, maptype 0; oracle codewords.
    let resolved = resolve_book_id(38, &tables).unwrap();
    let row = t97.rows()[38].clone();
    assert_eq!(row.dim, 1);
    assert_eq!(row.entries, 8);
    assert_eq!(row.maptype, 0);
    let codebook = wem_profiles::load_codebook(&resolved, &tables).expect("book 38 builds");
    assert_eq!(codebook.codelist, [0, 1, 5, 33, 3, 9, 17, 97]);

    // Book 214 (t219[117]): dim=4, entries=81, maptype 1, quantvals 3.
    let resolved = resolve_book_id(214, &tables).unwrap();
    let row = t219.rows()[117].clone();
    assert_eq!(row.dim, 4);
    assert_eq!(row.entries, 81);
    assert_eq!(row.maptype, 1);
    assert_eq!(row.quantvals, Some(3));
    let codebook = wem_profiles::load_codebook(&resolved, &tables).expect("book 214 builds");
    assert_eq!(codebook.quantvals, 3);
    // First six VQ vectors, low two words each (f32 bit oracles).
    let words = |e: i64| -> [u32; 2] {
        let vec = codebook.vq_values(e).expect("vq vector");
        [(vec[0] as f32).to_bits(), (vec[1] as f32).to_bits()]
    };
    assert_eq!(words(0), [0x0000_0000, 0x0000_0000]);
    assert_eq!(words(1), [0xBF80_0000, 0x0000_0000]);
    assert_eq!(words(2), [0x3F80_0000, 0x0000_0000]);
    assert_eq!(words(3), [0x0000_0000, 0xBF80_0000]);
    assert_eq!(words(4), [0xBF80_0000, 0xBF80_0000]);
    assert_eq!(words(5), [0x3F80_0000, 0xBF80_0000]);
}

// ---------------------------------------------------------------------------
// MDCT looks (static trig banks from the compiled carrier)
// ---------------------------------------------------------------------------

#[test]
fn mdct_look_oracle_values() {
    let looks = profile().mdct_looks().expect("mdct looks");
    let trig4 = |look: &wem_analysis::config::MdctLook| -> [u32; 4] {
        [
            look.trig[0].to_bits(),
            look.trig[1].to_bits(),
            look.trig[2].to_bits(),
            look.trig[3].to_bits(),
        ]
    };
    let scale = |look: &wem_analysis::config::MdctLook| look.scale.to_bits();
    let log2n = |look: &wem_analysis::config::MdctLook| look.log2n;
    assert_eq!(looks.len(), 5);
    assert_eq!(
        trig4(&looks[&128]),
        [0x3F80_0000, 0x8000_0000, 0x3F7E_C46D, 0xBDC8_BD36]
    );
    assert_eq!(scale(&looks[&128]), 0x3D00_0000);
    assert_eq!(log2n(&looks[&128]), 7);
    assert_eq!(
        trig4(&looks[&256]),
        [0x3F80_0000, 0x8000_0000, 0x3F7F_B10F, 0xBD48_FB30]
    );
    assert_eq!(scale(&looks[&256]), 0x3C80_0000);
    assert_eq!(
        trig4(&looks[&512]),
        [0x3F80_0000, 0x8000_0000, 0x3F7F_EC43, 0xBCC9_0AB0]
    );
    assert_eq!(scale(&looks[&512]), 0x3C00_0000);
    assert_eq!(
        trig4(&looks[&1024]),
        [0x3F80_0000, 0x8000_0000, 0x3F7F_FB11, 0xBC49_0E90]
    );
    assert_eq!(scale(&looks[&1024]), 0x3B80_0000);
    assert_eq!(
        trig4(&looks[&2048]),
        [0x3F80_0000, 0x8000_0000, 0x3F7F_FEC4, 0xBBC9_0F88]
    );
    assert_eq!(scale(&looks[&2048]), 0x3B00_0000);

    // Bit-reversal table oracle (n=128).
    assert_eq!(
        looks[&128].bitrev,
        [
            62, 0, 30, 32, 46, 16, 14, 48, 54, 8, 22, 40, 38, 24, 6, 56, 58, 4, 26, 36, 42, 20, 10,
            52, 50, 12, 18, 44, 34, 28, 2, 60,
        ]
    );
}

// ---------------------------------------------------------------------------
// Frozen transcendental tables (bit-pattern restoration)
// ---------------------------------------------------------------------------

#[test]
fn frozen_tables_oracle_values() {
    let frozen = profile()
        .frozen_tables()
        .expect("frozen tables read")
        .expect("the 6ch profile carries frozen tables");

    // coordinate_ln: 128 entries keyed by frequency f64 bits.
    assert_eq!(frozen.coordinate_ln.len(), 128);
    let expected_keys = [
        0x4055_8880_0000_0000u64,
        0x4070_2660_0000_0000,
        0x407A_EAA0_0000_0000,
        0x4082_D770_0000_0000,
        0x4088_3990_0000_0000,
        0x408D_9BB0_0000_0000,
        0x4091_7EE8_0000_0000,
        0x4094_2FF8_0000_0000,
    ];
    let expected_vals = [
        0x4011_D2D4_F14B_8299u64,
        0x4016_37CF_8FF6_C35C,
        0x4018_42E5_6F46_E5E6,
        0x4019_9B71_9CD8_192F,
        0x401A_9CCA_2EA2_041F,
        0x401B_6A46_CD0B_1E3F,
        0x401C_1557_06E4_48C2,
        0x401C_A7E0_0DF2_26A9,
    ];
    for (key, bits) in expected_keys.iter().zip(expected_vals.iter()) {
        let value = frozen
            .coordinate_ln
            .get(key)
            .unwrap_or_else(|| panic!("coordinate_ln missing {key:016X}"));
        assert_eq!(value.to_bits(), *bits, "coordinate_ln value bits");
    }

    // fft_twiddles: (cos, sin) of -2*pi/length as f64 bits.
    assert_eq!(frozen.fft_twiddles.len(), 11);
    let cases: [(i64, u64, u64); 11] = [
        (2, 0xBFF0_0000_0000_0000, 0xBCA1_A626_3314_5C07),
        (4, 0x3C91_A626_3314_5C07, 0xBFF0_0000_0000_0000),
        (8, 0x3FE6_A09E_667F_3BCD, 0xBFE6_A09E_667F_3BCC),
        (16, 0x3FED_906B_CF32_8D46, 0xBFD8_7DE2_A6AE_A963),
        (32, 0x3FEF_6297_CFF7_5CB0, 0xBFC8_F8B8_3C69_A60A),
        (64, 0x3FEF_D88D_A3D1_2526, 0xBFB9_17A6_BC29_B42C),
        (128, 0x3FEF_F621_E379_6D7E, 0xBFA9_1F65_F10D_D814),
        (256, 0x3FEF_FD88_6084_CD0D, 0xBF99_2155_F7A3_667E),
        (512, 0x3FEF_FF62_169B_92DB, 0xBF89_21D1_FCDE_C784),
        (1024, 0x3FEF_FFD8_858E_8A92, 0xBF79_21F0_FE67_0071),
        (2048, 0x3FEF_FFF6_2162_1D02, 0xBF69_21F8_BECC_A4BA),
    ];
    for (length, cos_bits, sin_bits) in cases {
        let (cos, sin) = frozen
            .fft_twiddles
            .get(&length)
            .unwrap_or_else(|| panic!("fft_twiddles missing length {length}"));
        assert_eq!(cos.to_bits(), cos_bits, "cos bits for length {length}");
        assert_eq!(sin.to_bits(), sin_bits, "sin bits for length {length}");
    }

    // window_halves: 256 and 2048 rows.
    assert_eq!(frozen.window_halves.len(), 2);
    assert_eq!(frozen.window_halves[&256].len(), 128);
    assert_eq!(frozen.window_halves[&2048].len(), 1024);
    assert_eq!(
        [
            frozen.window_halves[&256][0].to_bits(),
            frozen.window_halves[&256][1].to_bits(),
            frozen.window_halves[&256][2].to_bits(),
            frozen.window_halves[&256][3].to_bits(),
        ],
        [0x3878_0C05, 0x3A0B_8332, 0x3AC1_BA76, 0x3B3D_CBE2]
    );
}

// ---------------------------------------------------------------------------
// Transient detector tables
// ---------------------------------------------------------------------------

#[test]
fn transient_tables_oracle_values() {
    let tt = profile()
        .transient_tables(None)
        .expect("transient tables read");
    assert_eq!(tt.n, 128);
    assert_eq!(tt.bias.to_bits(), 0xC2A0_0000);
    assert_eq!(tt.window[0].to_bits(), 0x0000_0000);
    assert_eq!(tt.config[0].to_bits(), 0x0000_0008);
    assert_eq!(tt.bands[0].offset, 2);
    assert_eq!(tt.bands[0].weights[0].to_bits(), 0x3EC3_EF16);
    assert_eq!(tt.bands[0].scale.to_bits(), 0x3EC3_EF16);
    // The 6ch profile registers a pre-materialized detector, so it has no
    // record family to materialize from.
    assert!(profile().transient_record_family().is_none());
}

// ---------------------------------------------------------------------------
// Short psychoacoustic seed surface + frozen-ln look
// ---------------------------------------------------------------------------

#[test]
fn short_seed_oracle_values() {
    let ss = profile().short_seed().expect("short seed reads");
    assert_eq!(ss.ath_offset.to_bits(), 0xC2C8_0001);
    assert_eq!(&ss.octave[..3], [-33, 114, 168]);
    assert_eq!(ss.row[0].to_bits(), 0x4100_0000);
    assert_eq!(ss.short_limit, 76);
    assert_eq!(ss.noise_fixed_window, 15);
    assert_eq!(ss.curve_attenuation[0].to_bits(), 0xC1C0_0003);
    assert_eq!(ss.remap_curve_offsets[0], 0);
    assert_eq!(ss.tone_curves.len() * 8 * 58, 7888);

    // Short look built through the frozen ln domain (bit-exact seam).
    let frozen = profile()
        .frozen_tables()
        .expect("frozen reads")
        .expect("frozen tables present");
    let look = wem_analysis::config::make_wwise_psy_look(&ss, None, Some(&frozen.coordinate_ln))
        .expect("short look builds from frozen ln");
    assert_eq!(look.envelope.len(), 128);
    assert_eq!(look.envelope[0].to_bits(), 0x0000_0000);
    let m0 = &look.mask_curves.0;
    assert_eq!(m0[0].to_bits(), 0x0000_0000);
    assert_eq!(m0[1].to_bits(), 0x0000_0000);
}

// ---------------------------------------------------------------------------
// Short psychoacoustic profiles
// ---------------------------------------------------------------------------

#[test]
fn short_profiles_oracle_values() {
    let sps = profile().short_profiles().expect("short profiles read");
    assert_eq!(sps.len(), 2);
    assert_eq!(sps[0].key, "short_look_0");
    assert_eq!(
        sps[0].candidate_bias_by_mode[0].to_bits(),
        0x4034_0000_0000_0000
    );
    assert_eq!(sps[0].curve_cap.to_bits(), 0xC038_0000_6000_0000);
    assert_eq!(sps[0].mask_curves[0][0].to_bits(), 0xC024_0000_8000_0000);
    assert_eq!(sps[0].band_limits, [9, 7, 3]);
}

// ---------------------------------------------------------------------------
// Long psychoacoustic tables and variants
// ---------------------------------------------------------------------------

#[test]
fn long_tables_oracle_values() {
    let lt = profile().long_base().expect("long tables read");
    assert_eq!(
        &lt.analysis_profile_u32[..4],
        [1, 3267887105, 3272343552, 1101004800]
    );
    assert_eq!(
        &lt.seed_outer_u32[..12],
        [
            1024, 374481848, 372291952, 375036752, 883441760, 883449968, 883454072, 4294967062, 5,
            8, 777, 44100
        ]
    );
    assert_eq!(lt.analysis_profile_u32[167], 9999);
    assert_eq!(
        [
            lt.seed_tone_banks[0][0][0].to_bits(),
            lt.seed_tone_banks[0][0][1].to_bits(),
        ],
        [0x0000_0000, 0x4200_0000]
    );

    // Long floor-envelope look scalars.
    let fl = wem_analysis::config::make_long_floor_envelope_look(&lt).expect("floor look");
    assert_eq!(fl.curve_bias.to_bits(), 0x410F_FFFD);
    assert_eq!(fl.curve_cap.to_bits(), 0xC1C0_0003);
    assert_eq!(fl.history_start, 9999);
    assert_eq!(fl.side_gain.to_bits(), 0x3F80_0000);

    // Long floor-seed look scalars.
    let seed = wem_analysis::config::make_wwise_long_seed_look(&lt).expect("seed look");
    assert_eq!(seed.ath_offset.to_bits(), 0xC2C8_0001);
    assert_eq!(seed.ath_floor.to_bits(), 0xC30C_0000);
    assert_eq!(seed.seed_ceiling.to_bits(), 0xC1C0_0003);
    assert_eq!(seed.max_curve_db.to_bits(), 0x42BE_0001);
    assert_eq!(seed.first_octave, -234);
    assert_eq!(seed.shift_octave, 5);
    assert_eq!(seed.eighth_octave_lines, 8);
    assert_eq!(seed.total_octave_lines, 777);
    assert_eq!(seed.group_labels[0], -225);
    assert_eq!(seed.group_labels[1], -77);

    // Long variants 2 and 3.
    let compiled = profile();
    let lm2 = compiled.long_variant(2).expect("mode 2");
    let lm3 = compiled.long_variant(3).expect("mode 3");
    assert_eq!(lm2.analysis_profile_u32[0], 1);
    assert_eq!(lm3.analysis_profile_u32[0], 1);
    assert_eq!(lm2.analysis_curves[0][0].to_bits(), 0xC160_0004);
    assert_eq!(lm2.profile_key, "1024:2");
    assert_eq!(lm3.profile_key, "1024:3");
    // Variant floor-seed surface is inherited, not duplicated.
    assert_eq!(lm2.seed_outer_u32, lt.seed_outer_u32);
    assert!(compiled.long_variant(4).is_err());
}

// ---------------------------------------------------------------------------
// Container metadata
// ---------------------------------------------------------------------------

#[test]
fn container_metadata_matches_profile() {
    let cm = profile().container_metadata();
    assert_eq!(cm.w_format_tag, 65535);
    assert_eq!(cm.n_channels, 6);
    assert_eq!(cm.n_samples_per_sec, 44100);
    assert_eq!(cm.u_blocksize0_pow, 8);
    assert_eq!(cm.u_blocksize1_pow, 11);
}

// ---------------------------------------------------------------------------
// Profile key
// ---------------------------------------------------------------------------

#[test]
fn profile_key_requires_complete_identity() {
    // The recorded setup identity is read off the installed carrier's own key,
    // never re-typed as a literal.
    let identity = profile().key().quality_setup_identity().to_string();
    let key = ProfileKey::new(6, 44100, "2013.2".into(), "5.1".into(), identity.clone())
        .expect("complete identity");
    assert_eq!(key.generation(), "2013.2");
    assert_eq!(key.channel_layout(), "5.1");
    assert_eq!(key.quality_setup_identity(), identity);
    // The human label is derived from the identity, never stored.
    assert_eq!(key.label(), "6ch/44100Hz/2013.2");
    // Non-positive geometry rejected.
    assert!(ProfileKey::new(0, 44100, "2013.2".into(), "5.1".into(), "sha256:x".into(),).is_err());
}

// ---------------------------------------------------------------------------
// Assembly end-to-end
// ---------------------------------------------------------------------------

#[test]
fn assembly_end_to_end() {
    let res =
        assemble_encoder_profile_resources(&profile(), None, None).expect("assembly succeeds");
    assert_eq!(res.setup_packet.len(), 201);
    assert_eq!(res.setup.nbooks, 39);
    assert_eq!(res.codebooks.len(), 39);
    assert!(res.analysis.frozen.is_some());
    let looks = &res.analysis.mdct_looks;
    for required in [128, 256, 2048] {
        assert!(looks.get(&required).is_some(), "look {required}");
    }
    assert_eq!(res.analysis.short_profiles.len(), 2);
    assert_eq!(res.analysis.long_variants.len(), 2);
    assert_eq!(res.analysis.long_floor_looks.len(), 2);
}

#[test]
fn encoder_profile_model_checks() {
    let compiled = profile();
    let profile = compiled.encoder_profile().expect("encoder profile");
    assert_eq!(profile.block_sizes(), [256, 2048]);
    assert_eq!(profile.channels(), 6);
    assert_eq!(profile.sample_rate(), 44100);
    assert_eq!(profile.setup_packet().expect("packet").len(), 201);
    assert_eq!(compiled.key(), profile.key());
}
