//! Compiled-carrier equivalence suite.
//!
//! The compiled profile tables under [`crate::generated`] replaced the
//! recorded resource tree as the kernel's profile source. This suite holds
//! the two side by side and demands they agree everywhere: identity, setup
//! packet bytes, container geometry, and every typed table the codec reads,
//! down to the last f32 word. It is the proof that the migration moved the
//! data without moving a single value — the byte-exactness suites over the
//! whole file are the end-to-end half of the same claim.
//!
//! The development tree is read with `verify_all` on, so a table can only
//! agree with the carrier if the recorded document it came from also still
//! matches its own SHA-256 chain.

use std::path::{Path, PathBuf};

use crate::bundle::load_profile_bundle;
use crate::carrier::{compiled_profiles, CompiledProfile};
use crate::data::DataDir;
use crate::source::ProfileSource;

fn repo_root() -> PathBuf {
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

/// One (compiled carrier, development tree) pair for the same profile.
fn pairs() -> Vec<(CompiledProfile, crate::bundle::ProfileBundle)> {
    compiled_profiles()
        .expect("compiled profiles")
        .into_iter()
        .map(|compiled| {
            let bundle = load_profile_bundle(&data_dir(), Some(compiled.name()), true)
                .expect("the recorded tree loads and verifies");
            (compiled, bundle)
        })
        .collect()
}

fn assert_f32_eq(label: &str, left: &[f32], right: &[f32]) {
    assert_eq!(left.len(), right.len(), "{label}: length differs");
    for (index, (a, b)) in left.iter().zip(right).enumerate() {
        assert_eq!(a.to_bits(), b.to_bits(), "{label}[{index}]: {} != {}", a, b);
    }
}

fn assert_f64_eq(label: &str, left: &[f64], right: &[f64]) {
    assert_eq!(left.len(), right.len(), "{label}: length differs");
    for (index, (a, b)) in left.iter().zip(right).enumerate() {
        assert_eq!(a.to_bits(), b.to_bits(), "{label}[{index}]: {} != {}", a, b);
    }
}

#[test]
fn every_compiled_profile_matches_the_recorded_tree() {
    let pairs = pairs();
    assert_eq!(pairs.len(), 2, "two installed profiles");
    for (compiled, bundle) in pairs {
        let label = compiled.name().to_string();

        assert_eq!(compiled.key(), bundle.key(), "{label}: key");
        assert_eq!(
            compiled.tables().name,
            bundle.name(),
            "{label}: diagnostic name"
        );
        assert_eq!(
            compiled.block_sizes(),
            bundle.block_sizes(),
            "{label}: block sizes"
        );
        assert_eq!(
            compiled.container_metadata(),
            bundle.container_metadata(),
            "{label}: container metadata"
        );
        assert_eq!(
            compiled.setup_packet().expect("compiled setup"),
            bundle.setup_packet().expect("recorded setup"),
            "{label}: setup packet bytes"
        );
        assert_eq!(
            compiled.tables().setup_sha256,
            bundle.setup().expect("recorded setup ref").sha256(),
            "{label}: setup identity"
        );

        // MDCT trig banks: bit-for-bit, plus the derived look surfaces.
        let compiled_looks = compiled.mdct_looks().expect("compiled mdct");
        let recorded_looks = bundle.mdct_looks().expect("recorded mdct");
        assert_eq!(
            compiled_looks.keys().collect::<Vec<_>>(),
            recorded_looks.keys().collect::<Vec<_>>(),
            "{label}: mdct bank set"
        );
        for (n, compiled_look) in &compiled_looks {
            let recorded = &recorded_looks[n];
            assert_f32_eq(
                &format!("{label}: mdct {n} trig"),
                &compiled_look.trig,
                &recorded.trig,
            );
            assert_eq!(
                compiled_look.bitrev, recorded.bitrev,
                "{label}: mdct {n} bitrev"
            );
            assert_eq!(
                compiled_look.scale.to_bits(),
                recorded.scale.to_bits(),
                "{label}: mdct {n} scale"
            );
            assert_eq!(compiled_look.n, recorded.n);
            assert_eq!(compiled_look.log2n, recorded.log2n);
        }

        // Codebooks: row by row, then the runtime books built from them.
        let compiled_tables = compiled.book_tables().expect("compiled codebooks");
        let recorded_tables = bundle.book_tables().expect("recorded codebooks");
        for name in ["t97", "t219", "t282"] {
            match (compiled_tables.get(name), recorded_tables.get(name)) {
                (None, None) => {}
                (Some(compiled_table), Some(recorded_table)) => {
                    assert_eq!(
                        compiled_table.name(),
                        recorded_table.name(),
                        "{label}: {name} name"
                    );
                    assert_eq!(
                        compiled_table.rows(),
                        recorded_table.rows(),
                        "{label}: {name} rows"
                    );
                }
                (a, b) => panic!(
                    "{label}: {name} installation differs ({:?} vs {:?})",
                    a.is_some(),
                    b.is_some()
                ),
            }
        }

        // Frozen transcendental tables.
        let compiled_frozen = compiled.frozen_tables().expect("compiled frozen");
        let recorded_frozen = bundle.frozen_tables().expect("recorded frozen");
        match (compiled_frozen, recorded_frozen) {
            (Some(compiled_frozen), Some(recorded_frozen)) => {
                assert_eq!(
                    compiled_frozen.coordinate_ln, recorded_frozen.coordinate_ln,
                    "{label}: frozen coordinate_ln"
                );
                let compiled_twiddles: Vec<(i64, u64, u64)> = {
                    let mut entries: Vec<(i64, u64, u64)> = compiled_frozen
                        .fft_twiddles
                        .iter()
                        .map(|(length, (cos, sin))| (*length, cos.to_bits(), sin.to_bits()))
                        .collect();
                    entries.sort_unstable();
                    entries
                };
                let recorded_twiddles: Vec<(i64, u64, u64)> = {
                    let mut entries: Vec<(i64, u64, u64)> = recorded_frozen
                        .fft_twiddles
                        .iter()
                        .map(|(length, (cos, sin))| (*length, cos.to_bits(), sin.to_bits()))
                        .collect();
                    entries.sort_unstable();
                    entries
                };
                assert_eq!(
                    compiled_twiddles, recorded_twiddles,
                    "{label}: fft twiddles"
                );
                let mut compiled_sizes: Vec<i64> =
                    compiled_frozen.window_halves.keys().copied().collect();
                let mut recorded_sizes: Vec<i64> =
                    recorded_frozen.window_halves.keys().copied().collect();
                compiled_sizes.sort_unstable();
                recorded_sizes.sort_unstable();
                assert_eq!(compiled_sizes, recorded_sizes, "{label}: window sizes");
                for size in compiled_sizes {
                    assert_f32_eq(
                        &format!("{label}: window half {size}"),
                        &compiled_frozen.window_halves[&size],
                        &recorded_frozen.window_halves[&size],
                    );
                }
            }
            (None, None) => {}
            (a, b) => panic!(
                "{label}: frozen table presence differs ({:?} vs {:?})",
                a.is_some(),
                b.is_some()
            ),
        }

        // Transient surfaces, including the quality-materialized forms.
        for quality in [None, Some(0.0), Some(1.0), Some(4.0), Some(7.0), Some(10.0)] {
            assert_eq!(
                compiled
                    .transient_tables(quality)
                    .expect("compiled transient"),
                bundle
                    .transient_tables(quality)
                    .expect("recorded transient"),
                "{label}: transient tables at quality {quality:?}"
            );
        }

        // Short psychoacoustic surface and profiles.
        let compiled_seed = compiled.short_seed().expect("compiled short seed");
        let recorded_seed = bundle.short_seed().expect("recorded short seed");
        assert_eq!(compiled_seed, recorded_seed, "{label}: short seed surface");
        assert_eq!(
            compiled.short_profiles().expect("compiled short profiles"),
            bundle.short_profiles().expect("recorded short profiles"),
            "{label}: short psychoacoustic profiles"
        );

        // Long tables and every analysis mode.
        assert_eq!(
            compiled.long_base().expect("compiled long base"),
            bundle.long_base().expect("recorded long base"),
            "{label}: long base tables"
        );
        for mode in [2, 3] {
            assert_eq!(
                compiled.long_variant(mode).expect("compiled variant"),
                bundle.long_variant(mode).expect("recorded variant"),
                "{label}: long variant mode {mode}"
            );
        }

        // Optional surfaces.
        let compiled_curves = compiled.quality_curves().expect("compiled curves");
        let recorded_curves = bundle.quality_curves().expect("recorded curves");
        match (compiled_curves, recorded_curves) {
            (Some(compiled_curves), Some(recorded_curves)) => {
                assert_f64_eq(
                    &format!("{label}: quality breakpoints"),
                    compiled_curves.breakpoints(),
                    recorded_curves.breakpoints(),
                );
                assert_eq!(
                    compiled_curves.curves(),
                    recorded_curves.curves(),
                    "{label}: quality curves"
                );
                assert_eq!(
                    compiled_curves.semantics(),
                    recorded_curves.semantics(),
                    "{label}: quality semantics"
                );
            }
            (None, None) => {}
            (a, b) => panic!(
                "{label}: quality curves presence differs ({:?} vs {:?})",
                a.is_some(),
                b.is_some()
            ),
        }
        assert_eq!(
            compiled
                .input_conditioner()
                .expect("compiled input conditioner")
                .map(|config| config.dc_filter_coefficient.to_bits()),
            bundle
                .input_conditioner()
                .expect("recorded input conditioner")
                .map(|config| config.dc_filter_coefficient.to_bits()),
            "{label}: input conditioner"
        );
    }
}

#[test]
fn assembled_resources_are_identical_from_both_sources() {
    for (compiled, bundle) in pairs() {
        let label = compiled.name().to_string();
        // A quality value needs the profile to carry quality curves; a profile
        // without them must fail the same way from both sources, not fall back.
        let with_curves = compiled
            .quality_curves()
            .expect("compiled curves")
            .is_some();
        for quality in [None, Some(4.0), Some(10.0)] {
            let from_carrier = crate::assemble_encoder_profile_resources(&compiled, None, quality);
            let from_tree = crate::assemble_encoder_profile_resources(&bundle, None, quality);
            if !with_curves && quality.is_some() {
                assert_eq!(
                    from_carrier
                        .expect_err("carrier requires curves")
                        .to_string(),
                    from_tree.expect_err("tree requires curves").to_string(),
                    "{label}: missing-curves rejection at quality {quality:?}"
                );
                continue;
            }
            let from_carrier = from_carrier.expect("carrier assembly");
            let from_tree = from_tree.expect("tree assembly");
            assert_eq!(
                from_carrier.setup_packet, from_tree.setup_packet,
                "{label}: assembled setup packet at quality {quality:?}"
            );
            assert_eq!(
                from_carrier.setup, from_tree.setup,
                "{label}: parsed setup at quality {quality:?}"
            );
            assert_eq!(
                from_carrier.analysis, from_tree.analysis,
                "{label}: analysis resources at quality {quality:?}"
            );
            assert_eq!(
                from_carrier.codebooks.len(),
                from_tree.codebooks.len(),
                "{label}: codebook count at quality {quality:?}"
            );
            for (index, (carrier_book, tree_book)) in from_carrier
                .codebooks
                .iter()
                .zip(&from_tree.codebooks)
                .enumerate()
            {
                assert_eq!(
                    carrier_book.static_codebook, tree_book.static_codebook,
                    "{label}: codebook {index} static table at quality {quality:?}"
                );
                assert_eq!(
                    carrier_book.codelist, tree_book.codelist,
                    "{label}: codebook {index} codewords at quality {quality:?}"
                );
                assert_eq!(
                    carrier_book.quantvals, tree_book.quantvals,
                    "{label}: codebook {index} quantvals at quality {quality:?}"
                );
                assert_eq!(carrier_book.book_id, tree_book.book_id);
                assert_eq!(carrier_book.table, tree_book.table);
                assert_eq!(carrier_book.index, tree_book.index);
            }
        }
    }
}
