//! Reference-value tests for the pure Vorbis codec core, mirrored from the
//! Python reference (tests/unit in the wem repo).

use wem_vorbis::bitio::{BitReader, OggPack};
use wem_vorbis::codebook::{
    book_maptype1_quantvals, float32_unpack, ilog_bits, make_codewords, Codebook, StaticCodebook,
};

#[test]
fn ilog_bits_matches_python() {
    assert_eq!(ilog_bits(0), 0);
    assert_eq!(ilog_bits(1), 1);
    assert_eq!(ilog_bits(2), 2);
    assert_eq!(ilog_bits(3), 2);
    assert_eq!(ilog_bits(7), 3);
    assert_eq!(ilog_bits(8), 4);
}

#[test]
fn float32_unpack_matches_python() {
    // Oracle from Python: struct/ldexp reference values.
    assert_eq!(float32_unpack(0), 0.0);
    // Oracle values computed by the Python reference.
    assert_eq!(float32_unpack(0x6180_0001), 0.00390625);
    assert_eq!(float32_unpack(0x61A0_0001), 0.0078125);
    // Sign handling: bit 31 negates the mantissa (Python: mant = -mant).
    assert_eq!(float32_unpack(0x8180_0001), -2.5160737381238802e-234);
    // Zero mantissa stays zero even with the sign bit set.
    assert_eq!(float32_unpack(0x8040_0000), 0.0);
}

#[test]
fn make_codewords_oracle_book38() {
    // t97 book 38: dim=1, entries=8 (Python oracle codewords).
    // t97 book 38 lengthlist (installed profile data).
    let lengths = [1, 6, 3, 7, 2, 4, 5, 7];
    assert_eq!(
        make_codewords(&lengths).unwrap(),
        [0, 1, 5, 33, 3, 9, 17, 97]
    );
}

#[test]
fn make_codewords_rejects_overpopulation() {
    // More codewords than the level allows -> overpopulated tree.
    let lengths = [1, 1, 1, 1, 1];
    assert!(make_codewords(&lengths).is_err());
}

#[test]
fn decode_tree_roundtrip() {
    let lengths = [1, 6, 3, 7, 2, 4, 5, 7];
    let cb = Codebook::from_static(
        StaticCodebook {
            dim: 1,
            entries: 8,
            lengthlist: lengths.to_vec(),
            maptype: 0,
            q_min: 0,
            q_delta: 0,
            q_quant: 0,
            q_sequencep: 0,
            quantlist: None,
        },
        None,
        None,
        None,
    )
    .unwrap();
    let codelist = &cb.codelist;
    let mut pack = OggPack::new(64);
    for entry in 0..8i64 {
        cb.encode(&mut pack, entry).unwrap();
    }
    let bytes = pack.get_buffer();
    let mut reader = BitReader::new(&bytes);
    for entry in 0..8i64 {
        assert_eq!(cb.decode(&mut reader).unwrap(), entry);
        assert_eq!(codelist[entry as usize], cb.codelist[entry as usize]);
    }
}

#[test]
fn maptype1_quantvals_matches_python() {
    // 81 entries, dim 4 -> largest r with r^4 <= 81 is 3.
    assert_eq!(book_maptype1_quantvals(81, 4).unwrap(), 3);
    assert_eq!(book_maptype1_quantvals(0, 4).unwrap(), 0);
    assert!(book_maptype1_quantvals(81, 0).is_err());
}

#[test]
fn maptype1_requires_quantlist() {
    // maptype 1 without a quantlist is rejected (Python ValueError).
    let sc = StaticCodebook {
        dim: 2,
        entries: 2,
        lengthlist: vec![1, 1],
        maptype: 1,
        q_min: 0,
        q_delta: 0,
        q_quant: 0,
        q_sequencep: 0,
        quantlist: None,
    };
    assert!(Codebook::from_static(sc, None, None, None).is_err());
}

#[test]
fn bitio_ooggpack_truncation_matches_c_semantics() {
    // 32-bit write at endbit 0 must zero the spill byte exactly like Python.
    let mut op = OggPack::new(16);
    op.write(0xDEADBEEF, 32).unwrap();
    // The defensive spill byte at ptr+4 is zeroed but not part of the used range.
    assert_eq!(op.get_buffer(), vec![0xEF, 0xBE, 0xAD, 0xDE]);

    // Cross-byte accumulation.
    op.reset();
    op.write(0b11111, 5).unwrap();
    op.write(0b010, 3).unwrap();
    op.write(0b1010, 4).unwrap();
    let bytes = op.get_buffer();
    let mut br = BitReader::new(&bytes);
    assert_eq!(br.read(5).unwrap(), 0b11111);
    assert_eq!(br.read(3).unwrap(), 0b010);
    assert_eq!(br.read(4).unwrap(), 0b1010);
}

#[test]
fn make_codewords_oracle_t219_row117() {
    let ll: [i64; 81] = [
        1, 4, 4, 5, 8, 7, 5, 7, 8, 5, 8, 8, 8, 10, 11, 8, 10, 11, 5, 8, 8, 8, 11, 10, 8, 11, 11, 4,
        8, 8, 8, 11, 11, 8, 11, 11, 8, 11, 11, 11, 13, 14, 11, 15, 14, 8, 11, 11, 10, 13, 12, 11,
        14, 14, 4, 8, 8, 8, 11, 11, 8, 11, 11, 7, 11, 11, 11, 15, 14, 10, 12, 14, 8, 11, 11, 11,
        14, 14, 11, 14, 13,
    ];
    let rust = wem_vorbis::codebook::make_codewords(&ll).unwrap();
    let py: [i64; 81] = [
        0, 1, 9, 5, 21, 85, 13, 53, 149, 29, 117, 245, 3, 131, 643, 67, 387, 1667, 19, 195, 35,
        163, 899, 99, 227, 1923, 611, 11, 7, 135, 71, 1635, 355, 199, 1379, 867, 39, 1891, 167,
        1191, 679, 4775, 1703, 12967, 2727, 103, 423, 1447, 935, 6823, 231, 1255, 10919, 2279, 15,
        23, 151, 87, 743, 1767, 215, 487, 1511, 55, 999, 2023, 119, 29351, 10471, 631, 1143, 6375,
        247, 375, 1399, 887, 14567, 3191, 1911, 11383, 7287,
    ];
    for (i, (a, b)) in rust.iter().zip(py.iter()).enumerate() {
        if a != b {
            panic!("diverge at {i}: rust {a} py {b}");
        }
    }
}
