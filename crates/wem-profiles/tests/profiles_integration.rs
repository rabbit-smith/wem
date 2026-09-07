//! Integration tests for `wem-profiles`: real-asset loading, bit-exact
//! oracles from the Python reference, and rejection-condition parity.

use std::path::{Path, PathBuf};

use wem_profiles::psychoacoustics::{
    config::load_short_seed_surface, long_tables::load_long_psy_tables,
    long_variants::load_long_variant, short_tables::load_short_psy_profiles,
};
use wem_profiles::{
    assemble_encoder_profile_resources, load_book_table, load_frozen_tables, load_mdct_looks,
    load_profile_bundle, load_transient_tables, normalize_resource_path, resolve_book_id,
    ContainerMetadata, DataDir, ProfileError, ProfileKey, ResourceRef, T219_COUNT, T97_COUNT,
};
use wem_vorbis::setup::{pack_setup, parse_setup};

fn repo_root() -> PathBuf {
    // crates/wem-profiles -> repo root (two levels up from the manifest dir).
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .expect("crates dir")
        .parent()
        .expect("repo root")
        .to_path_buf()
}

fn data_dir() -> DataDir {
    DataDir::from_profiles_dir(repo_root().join("src/wwise_wem/data/profiles"))
}

fn bundle() -> wem_profiles::ProfileBundle {
    load_profile_bundle(&data_dir(), None, false).expect("installed profile loads")
}

// ---------------------------------------------------------------------------
// Real-asset loading and verification
// ---------------------------------------------------------------------------

#[test]
fn verify_all_on_real_assets() {
    let bundle = load_profile_bundle(&data_dir(), None, true).expect("verify_all passes");
    assert_eq!(bundle.name(), "wwise2013-6ch-44100");
    assert_eq!(
        (bundle.key().channels(), bundle.key().sample_rate()),
        (6, 44100)
    );
    assert_eq!(bundle.block_sizes(), [256, 2048]);
    let setup_ref = bundle.setup().expect("vorbis.setup");
    assert_eq!(
        setup_ref.sha256(),
        wem_profiles::WWISE2013_6CH_44100_SETUP_IDENTITY
            .strip_prefix("sha256:")
            .unwrap()
    );
    // All ten logical resources verify.
    assert_eq!(bundle.runtime_manifest().resources().len(), 10);
}

#[test]
fn setup_parse_golden_and_roundtrip() {
    let packet = bundle().setup_packet().expect("setup packet");
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
    assert_eq!(pack_setup(&info), packet);
}

#[test]
fn book_id_resolution_oracle() {
    let b = bundle();
    let manifest = b.runtime_manifest();
    let t97 = load_book_table("t97", manifest.resource("vorbis.codebooks.t97").unwrap())
        .expect("t97 loads");
    let t219 = load_book_table("t219", manifest.resource("vorbis.codebooks.t219").unwrap())
        .expect("t219 loads");
    assert_eq!(t97.rows().len(), T97_COUNT);
    assert_eq!(t219.rows().len(), T219_COUNT);
    let tables = wem_profiles::BookTables::new(t97, t219);

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
fn codebook_golden_oracles() {
    let b = bundle();
    let manifest = b.runtime_manifest();
    let t97 = load_book_table("t97", manifest.resource("vorbis.codebooks.t97").unwrap()).unwrap();
    let t219 =
        load_book_table("t219", manifest.resource("vorbis.codebooks.t219").unwrap()).unwrap();
    let tables = wem_profiles::BookTables::new(t97, t219);

    // Book 38 (t97[38]): dim=1, entries=8, maptype 0; oracle codewords.
    let resolved = resolve_book_id(38, &tables).unwrap();
    let row = tables.get("t97").unwrap().rows()[38].clone();
    assert_eq!(row.dim, 1);
    assert_eq!(row.entries, 8);
    assert_eq!(row.maptype, 0);
    let codebook = wem_profiles::load_codebook(&resolved, &tables).expect("book 38 builds");
    assert_eq!(codebook.codelist, [0, 1, 5, 33, 3, 9, 17, 97]);

    // Book 214 (t219[117]): dim=4, entries=81, maptype 1, quantvals 3.
    let resolved = resolve_book_id(214, &tables).unwrap();
    let row = tables.get("t219").unwrap().rows()[117].clone();
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
// MDCT looks (static trig banks from the profile payload)
// ---------------------------------------------------------------------------

#[test]
fn mdct_look_golden_oracles() {
    let looks = load_mdct_looks(
        bundle()
            .runtime_manifest()
            .resource("transform.mdct")
            .unwrap(),
    )
    .expect("mdct looks load");
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
fn frozen_tables_golden_oracles() {
    let frozen = load_frozen_tables(
        bundle()
            .runtime_manifest()
            .resource("analysis.frozen-tables")
            .unwrap(),
    )
    .expect("frozen tables load");

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
fn transient_tables_golden_oracle() {
    let tt = load_transient_tables(
        bundle()
            .runtime_manifest()
            .resource("analysis.transient")
            .unwrap(),
    )
    .expect("transient tables load");
    assert_eq!(tt.n, 128);
    assert_eq!(tt.bias.to_bits(), 0xC2A0_0000);
    assert_eq!(tt.window[0].to_bits(), 0x0000_0000);
    assert_eq!(tt.config[0].to_bits(), 0x0000_0008);
    assert_eq!(tt.bands[0].offset, 2);
    assert_eq!(tt.bands[0].weights[0].to_bits(), 0x3EC3_EF16);
    assert_eq!(tt.bands[0].scale.to_bits(), 0x3EC3_EF16);
}

// ---------------------------------------------------------------------------
// Short psychoacoustic seed surface + frozen-ln look
// ---------------------------------------------------------------------------

#[test]
fn short_seed_golden_oracle() {
    let b = bundle();
    let manifest = b.runtime_manifest();
    let ss = load_short_seed_surface(manifest.resource("psychoacoustics.short-seed").unwrap())
        .expect("short seed loads");
    assert_eq!(ss.ath_offset.to_bits(), 0xC2C8_0001);
    assert_eq!(&ss.octave[..3], [-33, 114, 168]);
    assert_eq!(ss.row[0].to_bits(), 0x4100_0000);
    assert_eq!(ss.short_limit, 76);
    assert_eq!(ss.noise_fixed_window, 15);
    assert_eq!(ss.curve_attenuation[0].to_bits(), 0xC1C0_0003);
    assert_eq!(ss.remap_curve_offsets[0], 0);
    assert_eq!(ss.tone_curves.len() * 8 * 58, 7888);

    // Short look built through the frozen ln domain (bit-exact seam).
    let frozen = load_frozen_tables(manifest.resource("analysis.frozen-tables").unwrap()).unwrap();
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
fn short_profiles_golden_oracle() {
    let sps = load_short_psy_profiles(
        bundle()
            .runtime_manifest()
            .resource("psychoacoustics.short-profiles")
            .unwrap(),
    )
    .expect("short profiles load");
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
fn long_tables_golden_oracle() {
    let lt = load_long_psy_tables(
        bundle()
            .runtime_manifest()
            .resource("psychoacoustics.long-base")
            .unwrap(),
    )
    .expect("long tables load");
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
    let b = bundle();
    let modes_ref = b
        .runtime_manifest()
        .resource("psychoacoustics.long-modes")
        .unwrap();
    let lm2 = load_long_variant(2, modes_ref, &lt).expect("mode 2");
    let lm3 = load_long_variant(3, modes_ref, &lt).expect("mode 3");
    assert_eq!(lm2.analysis_profile_u32[0], 1);
    assert_eq!(lm3.analysis_profile_u32[0], 1);
    assert_eq!(lm2.analysis_curves[0][0].to_bits(), 0xC160_0004);
    assert_eq!(lm2.profile_key, "1024:2");
    assert_eq!(lm3.profile_key, "1024:3");
    // Variant floor-seed surface is inherited, not duplicated.
    assert_eq!(lm2.seed_outer_u32, lt.seed_outer_u32);
    assert!(load_long_variant(4, modes_ref, &lt).is_err());
}

// ---------------------------------------------------------------------------
// Container metadata
// ---------------------------------------------------------------------------

#[test]
fn container_metadata_golden_and_rejections() {
    let b = bundle();
    let cm = b.container_metadata();
    assert_eq!(cm.w_format_tag, 65535);
    assert_eq!(cm.n_channels, 6);
    assert_eq!(cm.n_samples_per_sec, 44100);
    assert_eq!(cm.u_blocksize0_pow, 8);
    assert_eq!(cm.u_blocksize1_pow, 11);

    // Boolean is not an integer (Python rejects bools).
    let mut map = cm.to_fmt_map(0);
    *map.get_mut("nChannels").unwrap() = serde_json::Value::Bool(true);
    assert!(ContainerMetadata::from_fmt_map(&map).is_err());
    // Negative value rejected.
    let mut map = cm.to_fmt_map(0);
    *map.get_mut("wFormatTag").unwrap() = serde_json::Value::from(-1);
    assert!(ContainerMetadata::from_fmt_map(&map).is_err());
    // Missing field rejected.
    let mut map = cm.to_fmt_map(0);
    map.remove("uMaxPacketSize");
    assert!(ContainerMetadata::from_fmt_map(&map).is_err());
    // Non-positive geometry rejected.
    let mut map = cm.to_fmt_map(0);
    *map.get_mut("nChannels").unwrap() = serde_json::Value::from(0);
    assert!(ContainerMetadata::from_fmt_map(&map).is_err());
}

// ---------------------------------------------------------------------------
// Profile key
// ---------------------------------------------------------------------------

#[test]
fn profile_key_default_identity_and_rejections() {
    let key = ProfileKey::new(6, 44100).expect("default identity fills");
    assert_eq!(key.generation(), Some("2013.2"));
    assert_eq!(key.channel_layout(), Some("5.1"));
    assert_eq!(
        key.quality_setup_identity(),
        Some(wem_profiles::WWISE2013_6CH_44100_SETUP_IDENTITY)
    );
    // Other geometry without a full identity is rejected.
    assert!(ProfileKey::new(2, 48000).is_err());
    // Non-positive geometry rejected.
    assert!(ProfileKey::new(0, 44100).is_err());
}

// ---------------------------------------------------------------------------
// Assembly end-to-end
// ---------------------------------------------------------------------------

#[test]
fn assembly_end_to_end() {
    let res = assemble_encoder_profile_resources(&bundle(), None).expect("assembly succeeds");
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

// ---------------------------------------------------------------------------
// Resource path normalization rejections
// ---------------------------------------------------------------------------

#[test]
fn resource_path_normalization_rejections() {
    assert!(normalize_resource_path("vorbis/setup.bin").is_ok());
    assert!(normalize_resource_path("").is_err());
    assert!(normalize_resource_path("vorbis\\setup.bin").is_err());
    assert!(normalize_resource_path("/abs/setup.bin").is_err());
    assert!(normalize_resource_path("./vorbis/setup.bin").is_err());
    assert!(normalize_resource_path("../escape.bin").is_err());
    assert!(normalize_resource_path("vorbis/../escape.bin").is_err());
    assert!(normalize_resource_path("vorbis//setup.bin").is_err());
}

// ---------------------------------------------------------------------------
// Cross-platform key-shape invariant
// ---------------------------------------------------------------------------

#[test]
fn resource_keys_are_canonical_slash_form() {
    // Key-shape invariant (holds on every platform): every logical resource
    // key of a loaded bundle is a profiles-dir-relative POSIX path. The
    // shared validator rejects backslashes, so any platform separator
    // leaking into key construction (std::path's display() on Windows)
    // fails here — asserted at the code level, no OS reproduction needed.
    let bundle = load_profile_bundle(&data_dir(), None, false).expect("profile loads");
    let manifest = bundle.runtime_manifest();
    assert_eq!(manifest.ref_path(), "wwise2013-6ch-44100/manifest.json");
    for (name, ref_) in manifest.resources() {
        assert!(
            !ref_.path().contains('\\'),
            "resource key of {name} must stay slash-separated"
        );
        assert!(
            normalize_resource_path(ref_.path()).is_ok(),
            "resource key of {name} must stay valid against the shared rules"
        );
        assert!(
            ref_.path()
                .strip_prefix("wwise2013-6ch-44100/")
                .is_some_and(|rel| !rel.is_empty()),
            "resource key of {name} is profiles-dir-relative"
        );
    }
    // The manifest view's keys are the same canonical keys, parent-relative.
    let view = bundle
        .to_encoder_profile()
        .expect("encoder profile")
        .runtime_manifest()
        .expect("manifest view");
    for key in view.files.keys() {
        assert!(!key.contains('\\'), "manifest view key {key:?}");
    }
    assert!(view.files.contains_key("vorbis/setup.bin"));
}

// ---------------------------------------------------------------------------
// Tamper detection with synthetic profile trees
// ---------------------------------------------------------------------------

fn temp_tree(tag: &str) -> PathBuf {
    let dir =
        std::env::temp_dir().join(format!("wem-profiles-test-{}-{}", tag, std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(dir.join("wwise2013-6ch-44100")).expect("temp dirs");
    dir
}

fn sha256_hex(payload: &[u8]) -> String {
    use sha2::{Digest, Sha256};
    let digest = Sha256::digest(payload);
    const DIGITS: &[u8; 16] = b"0123456789abcdef";
    let mut out = String::new();
    for &b in digest.as_slice() {
        out.push(DIGITS[(b >> 4) as usize] as char);
        out.push(DIGITS[(b & 0xF) as usize] as char);
    }
    out
}

fn write(path: &Path, payload: &[u8]) {
    std::fs::write(path, payload).expect("write temp resource");
}

/// Build a minimal valid profile tree and return the DataDir.
fn valid_tree(tag: &str) -> DataDir {
    let root = temp_tree(tag);
    let profile_dir = root.join("wwise2013-6ch-44100");

    let setup_payload = b"setup-payload";
    write(&profile_dir.join("vorbis_setup.bin"), setup_payload);
    let setup_sha = sha256_hex(setup_payload);

    let index = serde_json::json!({
        "schema": "wwise-wem.profile-index.v1",
        "default": "wwise2013-6ch-44100",
        "profiles": {
            "wwise2013-6ch-44100": {
                "manifest": "wwise2013-6ch-44100/manifest.json",
                "sha256": format!("pending-{tag}")
            }
        }
    });
    let manifest = serde_json::json!({
        "schema": "wwise-wem.profile-manifest.v1",
        "name": "wwise2013-6ch-44100",
        "key": {
            "generation": "2013.2",
            "channels": 6,
            "sample_rate": 44100,
            "channel_layout": "5.1",
            "quality_setup_identity": format!("sha256:{setup_sha}")
        },
        "block_sizes": [4, 8],
        "container_metadata": {
            "wFormatTag": 65535, "nChannels": 6, "nSamplesPerSec": 44100,
            "nAvgBytesPerSec": 0, "nBlockAlign": 0, "wBitsPerSample": 0,
            "cbSize": 48, "wReserved0": 0, "dwChannelMask": 63,
            "dwTotalPCMFrames": 0, "dwFirstAudioPacketOffset": 0,
            "dwDataPayloadSize": 0, "dwUnknown_0x24": 0, "dwSeekTableSize": 0,
            "dwVorbisDataOffset": 0, "uMaxPacketSize": 0, "uUnknown_0x32": 0,
            "dwUnknown_0x34": 0, "dwUnknown_0x38": 0, "dwUnknown_0x3C": 0,
            "uBlocksize0Pow": 2, "uBlocksize1Pow": 3
        },
        "resources": {
            "vorbis.setup": {
                "path": "vorbis_setup.bin",
                "sha256": setup_sha
            }
        }
    });
    let manifest_bytes = serde_json::to_vec_pretty(&manifest).unwrap();
    let manifest_sha = sha256_hex(&manifest_bytes);
    write(&profile_dir.join("manifest.json"), &manifest_bytes);

    let mut index = serde_json::to_value(&index).unwrap();
    index["profiles"]["wwise2013-6ch-44100"]["sha256"] = serde_json::Value::String(manifest_sha);
    write(
        &root.join("index.json"),
        &serde_json::to_vec(&index).unwrap(),
    );

    DataDir::from_profiles_dir(root)
}

#[test]
fn synthetic_tree_loads_and_verifies() {
    let data = valid_tree("ok");
    let bundle = load_profile_bundle(&data, None, true).expect("synthetic tree loads");
    assert_eq!(bundle.name(), "wwise2013-6ch-44100");
    let setup = bundle.setup_packet().expect("setup payload");
    assert_eq!(setup, b"setup-payload");
}

#[test]
fn tamper_detection_sha_mismatch() {
    let data = valid_tree("sha");
    // Mutate the resource payload after the manifest digest was recorded.
    let file = data
        .profiles_dir()
        .join("wwise2013-6ch-44100/vorbis_setup.bin");
    std::fs::write(&file, b"setup-payload-MUTATED").unwrap();
    let err = load_profile_bundle(&data, None, true).unwrap_err();
    match err {
        ProfileError::ShaMismatch {
            expected, actual, ..
        } => {
            assert_ne!(expected, actual);
        }
        other => panic!("expected ShaMismatch, got {other:?}"),
    }
}

#[test]
fn tamper_detection_truncated_resource() {
    let data = valid_tree("trunc");
    let file = data
        .profiles_dir()
        .join("wwise2013-6ch-44100/vorbis_setup.bin");
    std::fs::write(&file, b"short").unwrap();
    let err = load_profile_bundle(&data, None, true).unwrap_err();
    assert!(
        matches!(err, ProfileError::ShaMismatch { .. }),
        "got {err:?}"
    );
}

#[test]
fn tamper_detection_path_traversal_in_manifest() {
    let root = temp_tree("trav");
    let profile_dir = root.join("wwise2013-6ch-44100");
    let setup_payload = b"setup";
    let setup_sha = sha256_hex(setup_payload);
    write(&profile_dir.join("setup.bin"), setup_payload);

    let manifest = serde_json::json!({
        "schema": "wwise-wem.profile-manifest.v1",
        "name": "wwise2013-6ch-44100",
        "key": {
            "generation": "2013.2", "channels": 6, "sample_rate": 44100,
            "channel_layout": "5.1",
            "quality_setup_identity": format!("sha256:{setup_sha}")
        },
        "block_sizes": [4, 8],
        "container_metadata": {
            "wFormatTag": 65535, "nChannels": 6, "nSamplesPerSec": 44100,
            "nAvgBytesPerSec": 0, "nBlockAlign": 0, "wBitsPerSample": 0,
            "cbSize": 48, "wReserved0": 0, "dwChannelMask": 63,
            "dwTotalPCMFrames": 0, "dwFirstAudioPacketOffset": 0,
            "dwDataPayloadSize": 0, "dwUnknown_0x24": 0, "dwSeekTableSize": 0,
            "dwVorbisDataOffset": 0, "uMaxPacketSize": 0, "uUnknown_0x32": 0,
            "dwUnknown_0x34": 0, "dwUnknown_0x38": 0, "dwUnknown_0x3C": 0,
            "uBlocksize0Pow": 2, "uBlocksize1Pow": 3
        },
        "resources": {
            "vorbis.setup": {
                "path": "../../escape/setup.bin",
                "sha256": setup_sha
            }
        }
    });
    let manifest_bytes = serde_json::to_vec_pretty(&manifest).unwrap();
    let manifest_sha = sha256_hex(&manifest_bytes);
    write(&profile_dir.join("manifest.json"), &manifest_bytes);

    let index = serde_json::json!({
        "schema": "wwise-wem.profile-index.v1",
        "default": "wwise2013-6ch-44100",
        "profiles": {
            "wwise2013-6ch-44100": {
                "manifest": "wwise2013-6ch-44100/manifest.json",
                "sha256": manifest_sha
            }
        }
    });
    write(
        &root.join("index.json"),
        &serde_json::to_vec(&index).unwrap(),
    );

    let err = load_profile_bundle(&DataDir::from_profiles_dir(root), None, true).unwrap_err();
    assert!(
        matches!(err, ProfileError::UnsafePath { .. }),
        "got {err:?}"
    );
}

#[test]
fn tamper_detection_index_sha_mismatch() {
    let root = temp_tree("idx");
    let profile_dir = root.join("wwise2013-6ch-44100");
    let manifest = serde_json::json!({
        "schema": "wwise-wem.profile-manifest.v1",
        "name": "wrong-name",
        "key": {
            "generation": "2013.2", "channels": 6, "sample_rate": 44100,
            "channel_layout": "5.1",
            "quality_setup_identity": "sha256:0000000000000000000000000000000000000000000000000000000000000000"
        },
        "block_sizes": [4, 8],
        "container_metadata": {
            "wFormatTag": 65535, "nChannels": 6, "nSamplesPerSec": 44100,
            "nAvgBytesPerSec": 0, "nBlockAlign": 0, "wBitsPerSample": 0,
            "cbSize": 48, "wReserved0": 0, "dwChannelMask": 63,
            "dwTotalPCMFrames": 0, "dwFirstAudioPacketOffset": 0,
            "dwDataPayloadSize": 0, "dwUnknown_0x24": 0, "dwSeekTableSize": 0,
            "dwVorbisDataOffset": 0, "uMaxPacketSize": 0, "uUnknown_0x32": 0,
            "dwUnknown_0x34": 0, "dwUnknown_0x38": 0, "dwUnknown_0x3C": 0,
            "uBlocksize0Pow": 2, "uBlocksize1Pow": 3
        },
        "resources": {}
    });
    let manifest_bytes = serde_json::to_vec(&manifest).unwrap();
    write(&profile_dir.join("manifest.json"), &manifest_bytes);
    // Index carries a deliberately wrong manifest sha256.
    let index = serde_json::json!({
        "schema": "wwise-wem.profile-index.v1",
        "default": "wwise2013-6ch-44100",
        "profiles": {
            "wwise2013-6ch-44100": {
                "manifest": "wwise2013-6ch-44100/manifest.json",
                "sha256": "0000000000000000000000000000000000000000000000000000000000000000"
            }
        }
    });
    write(
        &root.join("index.json"),
        &serde_json::to_vec(&index).unwrap(),
    );

    let err = load_profile_bundle(&DataDir::from_profiles_dir(root), None, false).unwrap_err();
    assert!(
        matches!(err, ProfileError::ShaMismatch { .. }),
        "got {err:?}"
    );
}

#[test]
fn tamper_detection_schema_strings() {
    // Bad index schema.
    let root = temp_tree("schema-index");
    let index = serde_json::json!({
        "schema": "wwise-wem.profile-index.v0",
        "default": "wwise2013-6ch-44100",
        "profiles": {}
    });
    write(
        &root.join("index.json"),
        &serde_json::to_vec(&index).unwrap(),
    );
    let err = load_profile_bundle(&DataDir::from_profiles_dir(root), None, false).unwrap_err();
    assert!(
        matches!(err, ProfileError::UnsupportedIndexSchema { .. }),
        "got {err:?}"
    );

    // Unknown profile in the index.
    let err = load_profile_bundle(&data_dir(), Some("does-not-exist"), false).unwrap_err();
    assert!(
        matches!(err, ProfileError::ProfileNotInIndex { .. }),
        "got {err:?}"
    );
}

#[test]
fn resource_ref_rejections() {
    let data = data_dir();
    // Bad digest shapes.
    assert!(ResourceRef::new(
        data.clone(),
        "wwise2013-6ch-44100/vorbis/setup.bin",
        "ABCDEF"
    )
    .is_err());
    assert!(ResourceRef::new(
        data.clone(),
        "wwise2013-6ch-44100/vorbis/setup.bin",
        "g000000000000000000000000000000000000000000000000000000000000000"
    )
    .is_err());
    // Uppercase digest is normalized to lowercase.
    let ref_ = ResourceRef::new(
        data.clone(),
        "wwise2013-6ch-44100/vorbis/setup.bin",
        &wem_profiles::WWISE2013_6CH_44100_SETUP_IDENTITY
            .strip_prefix("sha256:")
            .unwrap()
            .to_uppercase(),
    )
    .expect("uppercase digest normalizes");
    assert_eq!(
        ref_.sha256(),
        wem_profiles::WWISE2013_6CH_44100_SETUP_IDENTITY
            .strip_prefix("sha256:")
            .unwrap()
    );
    // Read verifies the digest.
    let bytes = ref_.read_bytes().expect("verified read");
    assert_eq!(bytes.len(), 201);

    // Missing resource.
    let missing = ResourceRef::new(
        data,
        "wwise2013-6ch-44100/does-not-exist.bin",
        &"0".repeat(64),
    )
    .expect("ref constructs");
    assert!(matches!(
        missing.read_bytes().unwrap_err(),
        ProfileError::MissingResource { .. }
    ));
}

// ---------------------------------------------------------------------------
// Registry behaviour
// ---------------------------------------------------------------------------

#[test]
fn installed_registry_resolutions() {
    let registry = wem_profiles::installed_registry(&data_dir()).expect("registry");
    assert_eq!(registry.len(), 1);

    let by_geometry = registry.resolve_geometry(6, 44100).expect("geometry");
    assert_eq!(by_geometry.name(), "wwise2013-6ch-44100");

    // Uninstalled geometry rejected.
    assert!(registry.resolve_geometry(2, 48000).is_err());
    // Unknown key rejected.
    let unknown = ProfileKey::with_identity(
        2,
        48000,
        Some("2013.2".into()),
        Some("stereo".into()),
        Some("sha256:0".into()),
    )
    .expect("full identity");
    assert!(registry.resolve_key(&unknown).is_err());
    // resolve_setup: right sha, wrong geometry -> rejected.
    let sha = wem_profiles::WWISE2013_6CH_44100_SETUP_IDENTITY
        .strip_prefix("sha256:")
        .unwrap();
    assert!(registry.resolve_setup(2, 44100, sha).is_err());
    let matched = registry
        .resolve_setup(6, 44100, sha)
        .expect("template setup resolves");
    assert_eq!(matched.name(), "wwise2013-6ch-44100");

    // load_wem_profile / resolve_wem_profile through the environment.
    let name = "wwise2013-6ch-44100";
    let loaded = wem_profiles::load_wem_profile(name).expect("named profile loads");
    assert_eq!(loaded.name(), name);
    assert!(wem_profiles::load_wem_profile("nope").is_err());
    let resolved = wem_profiles::resolve_wem_profile(6, 44100).expect("geometry profile");
    assert_eq!(resolved.block_sizes(), [256, 2048]);
}

#[test]
fn encoder_profile_model_checks() {
    let b = bundle();
    let profile = b.to_encoder_profile().expect("encoder profile");
    assert_eq!(profile.block_sizes(), [256, 2048]);
    assert_eq!(profile.channels(), 6);
    assert_eq!(profile.sample_rate(), 44100);
    let fmt = profile.fmt();
    assert_eq!(fmt.get("nChannels").and_then(|v| v.as_i64()), Some(6));
    assert_eq!(profile.setup_packet().expect("packet").len(), 201);
    let view = profile.runtime_manifest().expect("manifest view");
    assert_eq!(view.resources.len(), 10);
    assert!(view.files.contains_key("vorbis/setup.bin"));
}
