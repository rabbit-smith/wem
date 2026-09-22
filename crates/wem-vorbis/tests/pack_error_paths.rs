//! Packing error paths for the pure Vorbis codec core: a setup record or
//! static codebook a caller can build by hand comes back as `SetupError` /
//! `CodebookError`, never as a panic, and an accepted record still packs to
//! the oracle bytes.

use wem_vorbis::bitio::{BitError, BitReader, OggPack};
use wem_vorbis::codebook::{make_codewords, Codebook, CodebookError, StaticCodebook};
use wem_vorbis::setup::{
    pack_floor1, pack_mapping0, pack_setup, parse_setup, BitPositions, Floor1Setup, Mapping0Setup,
    ModeSetup, ResidueSetup, SetupError, SetupInfo,
};

fn floor_with_one_class() -> Floor1Setup {
    Floor1Setup {
        partitions: 1,
        partition_classes: vec![0],
        max_class: 0,
        class_dims: vec![1],
        class_subs: vec![0],
        class_masterbooks: vec![None],
        subclass_books: vec![vec![-1]],
        multiplier: 2,
        rangebits: 8,
        x_list: vec![0],
        bit_start: 0,
        bit_end: 0,
    }
}

fn residue_with_one_class() -> ResidueSetup {
    ResidueSetup {
        residue_type: 1,
        begin: 0,
        end: 0,
        partition_size: 1,
        classifications: 1,
        classbook: 0,
        cascades: vec![0],
        books: vec![[-1; 8]],
        bit_start: 0,
        bit_end: 0,
    }
}

fn mapping_with_one_submap() -> Mapping0Setup {
    Mapping0Setup {
        submaps: 1,
        coupling: Vec::new(),
        reserved: 0,
        chmux: vec![0],
        floors: vec![0],
        residues: vec![0],
        bit_start: 0,
        bit_end: 0,
    }
}

fn mode_setup() -> ModeSetup {
    ModeSetup {
        blockflag: 0,
        mapping: 0,
        bit_start: 0,
        bit_end: 0,
    }
}

/// The smallest setup the oracle accepts: one book, one floor, one residue,
/// one map, one mode (mirrors tests/parity/test_setup_core_boundary.py).
fn minimal_setup() -> SetupInfo {
    SetupInfo {
        setup_size: 0,
        bits_total: 0,
        channels: 1,
        nbooks: 1,
        book_ids: vec![0],
        unique_book_ids: vec![0],
        nfloors: 1,
        floors: vec![floor_with_one_class()],
        nresidues: 1,
        residues: vec![residue_with_one_class()],
        nmaps: 1,
        maps: vec![mapping_with_one_submap()],
        nmodes: 1,
        modes: vec![mode_setup()],
        bit_positions: BitPositions {
            after_books: 18,
            after_floor_count: 24,
            after_floors: 0,
            after_residues: 0,
            after_maps: 0,
            after_modes: 0,
            end: 0,
        },
        trailing_pad_bits: 0,
        trailing_pad_value: 0,
        parse_complete: true,
        book_id_assignment: "t97:0-96, t219:97-315, t282:316-597",
    }
}

/// A write width the packer cannot express is reported, not truncated.
#[test]
fn pack_floor1_reports_write_width_beyond_32_bits() {
    let mut floor = floor_with_one_class();
    floor.rangebits = 40;
    assert_eq!(
        pack_floor1(&mut OggPack::new(64), &floor),
        Err(SetupError::Bit(BitError::BitsOutOfRange { bits: 40 }))
    );
}

/// A subclass declared without its masterbook cannot be serialized.
#[test]
fn pack_floor1_reports_missing_masterbook() {
    let mut floor = floor_with_one_class();
    floor.class_subs = vec![1];
    floor.class_masterbooks = vec![None];
    assert_eq!(
        pack_floor1(&mut OggPack::new(64), &floor),
        Err(SetupError::FieldMissing {
            field: "class_masterbooks"
        })
    );
}

/// A submap index without a matching floor/residue entry is reported.
#[test]
fn pack_mapping0_reports_missing_floor_map() {
    let mut mapping = mapping_with_one_submap();
    mapping.submaps = 2;
    mapping.floors = vec![0];
    mapping.residues = vec![0, 0];
    assert_eq!(
        pack_mapping0(&mut OggPack::new(64), &mapping, 2),
        Err(SetupError::FieldMissing { field: "floors" })
    );

    mapping.floors = vec![0, 0];
    mapping.residues = vec![0];
    assert_eq!(
        pack_mapping0(&mut OggPack::new(64), &mapping, 2),
        Err(SetupError::FieldMissing { field: "residues" })
    );
}

/// `pack_setup` propagates the record error instead of returning a truncated
/// packet.
#[test]
fn pack_setup_propagates_a_write_failure() {
    let mut info = minimal_setup();
    info.floors[0].rangebits = 40;
    assert_eq!(
        pack_setup(&info),
        Err(SetupError::Bit(BitError::BitsOutOfRange { bits: 40 }))
    );
}

/// The accepted setup still round-trips to identical bytes.
#[test]
fn pack_setup_roundtrip_is_byte_stable() {
    let packet = pack_setup(&minimal_setup()).expect("minimal setup packs");
    let parsed = parse_setup(&packet, 1).expect("packed setup parses");
    assert!(parsed.parse_complete);
    assert_eq!(pack_setup(&parsed).expect("parsed setup repacks"), packet);
}

fn maptype1_book(entries: i64, lengthlist: Vec<i64>) -> Codebook {
    Codebook::from_static(
        StaticCodebook {
            dim: 2,
            entries,
            lengthlist,
            maptype: 1,
            q_min: 0,
            q_delta: 0x61A0_0001,
            q_quant: 1,
            q_sequencep: 0,
            quantlist: Some(vec![0, 1]),
        },
        None,
        None,
        None,
    )
    .expect("maptype1 book builds")
}

/// A lengthlist that does not cover `entries` is reported, not read past.
#[test]
fn static_codebook_pack_reports_lengthlist_mismatch() {
    let sc = StaticCodebook {
        dim: 1,
        entries: 3,
        lengthlist: vec![1, 1],
        maptype: 0,
        q_min: 0,
        q_delta: 0,
        q_quant: 0,
        q_sequencep: 0,
        quantlist: None,
    };
    assert_eq!(
        sc.pack(),
        Err(CodebookError::LengthlistMismatch { got: 2, want: 3 })
    );
}

/// A single-entry book whose first length is zero still packs: the oracle
/// masks the `lengths[0] - 1` write, so the field becomes 31, not an
/// underflow.
#[test]
fn static_codebook_pack_masks_a_zero_first_length() {
    let sc = StaticCodebook {
        dim: 1,
        entries: 1,
        lengthlist: vec![0],
        maptype: 0,
        q_min: 0,
        q_delta: 0,
        q_quant: 0,
        q_sequencep: 0,
        quantlist: None,
    };
    let bytes = sc.pack().expect("zero first length packs");
    let mut br = BitReader::new(&bytes);
    assert_eq!(br.read(4).unwrap(), 1); // dim
    assert_eq!(br.read(14).unwrap(), 1); // entries
    assert_eq!(br.read(1).unwrap(), 1); // ordered
    assert_eq!(br.read(5).unwrap(), 31); // (0 - 1) & 0x1F, the oracle's mask
}

/// A lengthlist width the writer cannot express is reported, in the unordered
/// branch too (the bound is checked before either branch writes).
#[test]
fn static_codebook_pack_reports_write_width_beyond_32_bits() {
    // A decreasing list takes the unordered branch; its width comes straight
    // from the largest length, so 2^33 needs 34 bits.
    let sc = StaticCodebook {
        dim: 1,
        entries: 2,
        lengthlist: vec![1i64 << 33, 5],
        maptype: 0,
        q_min: 0,
        q_delta: 0,
        q_quant: 0,
        q_sequencep: 0,
        quantlist: None,
    };
    assert_eq!(
        sc.pack(),
        Err(CodebookError::LengthOutOfRange {
            entry: 0,
            length: 1i64 << 33
        })
    );
}

/// `q_quant` is the width of every quantlist write: a value the writer cannot
/// express is reported, not truncated.
#[test]
fn static_codebook_pack_reports_quantlist_width_beyond_32_bits() {
    let sc = StaticCodebook {
        dim: 2,
        entries: 2,
        lengthlist: vec![1, 2],
        maptype: 1,
        q_min: 0,
        q_delta: 0x61A0_0001,
        q_quant: 33,
        q_sequencep: 0,
        quantlist: Some(vec![0, 1]),
    };
    assert_eq!(
        sc.pack(),
        Err(CodebookError::PackBitsOutOfRange { bits: 33 })
    );
}

/// `vq_values` reports an entry with no vector (unused, or outside the book)
/// instead of indexing the list.
#[test]
fn vq_values_reports_missing_vector() {
    let book = maptype1_book(4, vec![1, 2, 3, 0]);
    assert_eq!(book.vq_values(0).expect("entry 0 has a vector").len(), 2);
    assert_eq!(
        book.vq_values(3),
        Err(CodebookError::NoVqVector { entry: 3 })
    );
    assert_eq!(
        book.vq_values(4),
        Err(CodebookError::NoVqVector { entry: 4 })
    );
    assert_eq!(
        book.vq_values(-1),
        Err(CodebookError::NoVqVector { entry: -1 })
    );
}

// ---------------------------------------------------------------------------
// Codebook construction: an index past the end of an array is reported
// ---------------------------------------------------------------------------

fn maptype1_static(entries: i64, lengthlist: Vec<i64>) -> StaticCodebook {
    StaticCodebook {
        dim: 2,
        entries,
        lengthlist,
        maptype: 1,
        q_min: 0,
        q_delta: 0x61A0_0001,
        q_quant: 1,
        q_sequencep: 0,
        quantlist: Some(vec![0, 1]),
    }
}

/// A codeword length beyond the builder's 33-slot marker table is reported,
/// from the public helper and through the public constructor.
#[test]
fn make_codewords_reports_length_beyond_marker_table() {
    assert_eq!(
        make_codewords(&[40]),
        Err(CodebookError::LengthOutOfRange {
            entry: 0,
            length: 40
        })
    );
    let sc = StaticCodebook {
        dim: 1,
        entries: 1,
        lengthlist: vec![40],
        maptype: 0,
        q_min: 0,
        q_delta: 0,
        q_quant: 0,
        q_sequencep: 0,
        quantlist: None,
    };
    assert_eq!(
        Codebook::from_static(sc, None, None, None).err(),
        Some(CodebookError::LengthOutOfRange {
            entry: 0,
            length: 40
        })
    );
}

/// A lengthlist longer than `entries` is reported where the VQ cache would
/// have indexed past the book's vectors.
#[test]
fn from_static_reports_lengthlist_longer_than_entries() {
    assert_eq!(
        Codebook::from_static(maptype1_static(1, vec![1, 1]), None, None, None).err(),
        Some(CodebookError::LengthlistMismatch { got: 2, want: 1 })
    );
    // A trailing *unused* entry never reaches the cache and stays accepted.
    assert!(Codebook::from_static(maptype1_static(1, vec![1, 0]), None, None, None).is_ok());
}

/// `entries` longer than the lengthlist is reported where the unquantizer
/// would have indexed past the lengthlist.
#[test]
fn from_static_reports_entries_beyond_lengthlist() {
    assert_eq!(
        Codebook::from_static(maptype1_static(2, vec![1]), None, None, None).err(),
        Some(CodebookError::LengthlistMismatch { got: 1, want: 2 })
    );
}

/// The ordered branch emits one field per length step; a length the writer's
/// first-length field cannot express is rejected instead of looping on it.
#[test]
fn static_codebook_pack_bounds_the_ordered_length_steps() {
    let sc = StaticCodebook {
        dim: 1,
        entries: 2,
        lengthlist: vec![5, 1i64 << 33],
        maptype: 0,
        q_min: 0,
        q_delta: 0,
        q_quant: 0,
        q_sequencep: 0,
        quantlist: None,
    };
    assert_eq!(
        sc.pack(),
        Err(CodebookError::LengthOutOfRange {
            entry: 1,
            length: 1i64 << 33
        })
    );
}

/// An entry inside `entries` but past the codeword/length arrays (the same
/// hand-built mismatch the maptype-0 path accepts) is reported, not indexed.
#[test]
fn encode_reports_entry_past_the_codeword_arrays() {
    let sc = StaticCodebook {
        dim: 1,
        entries: 2,
        lengthlist: vec![1],
        maptype: 0,
        q_min: 0,
        q_delta: 0,
        q_quant: 0,
        q_sequencep: 0,
        quantlist: None,
    };
    let book = Codebook::from_static(sc, None, None, None).expect("constructor accepts the book");
    let mut op = OggPack::new(16);
    assert_eq!(
        book.encode(&mut op, 1),
        Err(CodebookError::EntryOutOfRange {
            entry: 1,
            entries: 1
        })
    );
    // Entry 0 is inside the arrays and still encodes.
    book.encode(&mut op, 0).expect("entry 0 encodes");
    // Past `entries` altogether behaves exactly as before.
    assert_eq!(
        book.encode(&mut op, 2),
        Err(CodebookError::EntryOutOfRange {
            entry: 2,
            entries: 1
        })
    );
}
