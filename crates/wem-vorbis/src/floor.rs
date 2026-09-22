//! Pure Vorbis floor1 algorithms in both directions (Python:
//! `wwise_wem/vorbis/floor.py`; libvorbis `floor1.c`).
//!
//! Packet Y values are *residuals* (wrapped predictions), not absolute post
//! heights. Call [`floor1_wrap`] before packing; the decode direction is
//! [`decode_floor1_body`] (bits → residuals) followed by [`floor1_unwrap`]
//! (residuals → absolute posts); [`floor1_curve_from_posts`] then renders the
//! curve.

use crate::bitio::{BitError, BitReader};
use crate::codebook::{Codebook, CodebookError};
use crate::setup::{ilog, Floor1Setup};

/// Floor1 errors (Python: `ValueError` family).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Floor1Error {
    /// posts/postlist length mismatch.
    LengthMismatch { posts: usize, postlist: usize },
    /// floor1_wrap input shorter than two endpoints.
    TooFewPosts { posts: usize },
    /// invalid multiplier.
    InvalidMultiplier { multiplier: u64 },
}

impl std::fmt::Display for Floor1Error {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Floor1Error::LengthMismatch { posts, postlist } => {
                write!(f, "posts len {posts} != postlist len {postlist}")
            }
            Floor1Error::TooFewPosts { posts } => {
                write!(f, "floor1 wrap needs two or more posts, got {posts}")
            }
            Floor1Error::InvalidMultiplier { multiplier } => {
                write!(f, "invalid floor1 multiplier: {multiplier}")
            }
        }
    }
}

impl std::error::Error for Floor1Error {}

/// Floor1 decode-direction errors.
///
/// Mirrors the `EOFError`/`ValueError` family that the reference decoder's
/// `decode_floor1_body` raises (and that its packet decoder wraps into an
/// incomplete-packet report). Every failure mode carries the observed values;
/// wrapped causes stay reachable through [`std::error::Error::source`].
#[derive(Debug, Clone, PartialEq)]
pub enum Floor1DecodeError {
    /// The packet ended before the floor body did.
    Bit(BitError),
    /// A classification or subclass book could not decode a codeword.
    Codebook(CodebookError),
    /// The floor's multiplier has no range table entry (valid: 1..=4).
    InvalidMultiplier { multiplier: u64 },
    /// A book id the floor body references is absent from `books`.
    BookIndexOutOfRange { book_id: i64, books: usize },
    /// A class named by `partition_classes` has no class record.
    ClassTableIncomplete { class: usize },
    /// A class declares `subs != 0` but carries no master book.
    MasterBookMissing { class: usize },
    /// A subclass book index falls outside the class's `2^subs` book table.
    SubclassBookOutOfRange {
        class: usize,
        index: usize,
        subclass_books: usize,
    },
    /// The decoded post count disagrees with `2 + x_list.len()`.
    PostCountMismatch { decoded: usize, expected: usize },
}

impl std::fmt::Display for Floor1DecodeError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Floor1DecodeError::Bit(err) => write!(f, "floor1 body bits: {err}"),
            Floor1DecodeError::Codebook(err) => write!(f, "floor1 body codebook: {err}"),
            Floor1DecodeError::InvalidMultiplier { multiplier } => {
                write!(f, "floor1 multiplier {multiplier} has no range table")
            }
            Floor1DecodeError::BookIndexOutOfRange { book_id, books } => {
                write!(f, "floor1 book id {book_id} out of range 0..{books}")
            }
            Floor1DecodeError::ClassTableIncomplete { class } => {
                write!(f, "floor1 class {class} has no class record")
            }
            Floor1DecodeError::MasterBookMissing { class } => {
                write!(f, "floor1 class {class} has subclasses but no master book")
            }
            Floor1DecodeError::SubclassBookOutOfRange {
                class,
                index,
                subclass_books,
            } => write!(
                f,
                "floor1 class {class} subclass index {index} out of range 0..{subclass_books}"
            ),
            Floor1DecodeError::PostCountMismatch { decoded, expected } => {
                write!(f, "floor1 Y count {decoded} != {expected}")
            }
        }
    }
}

impl std::error::Error for Floor1DecodeError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Floor1DecodeError::Bit(err) => Some(err),
            Floor1DecodeError::Codebook(err) => Some(err),
            _ => None,
        }
    }
}

impl From<BitError> for Floor1DecodeError {
    fn from(err: BitError) -> Self {
        Floor1DecodeError::Bit(err)
    }
}

impl From<CodebookError> for Floor1DecodeError {
    fn from(err: CodebookError) -> Self {
        Floor1DecodeError::Codebook(err)
    }
}

/// multiplier (1..=4) → quant range (libvorbis `look->quant_q`).
/// Index 0 is unused; valid multipliers are 1..=4.
pub const FLOOR1_RANGES: [u64; 5] = [0, 256, 128, 86, 64];

/// libvorbis `FLOOR1_fromdB_LOOKUP[256]`: quant 0..=255 → linear amplitude.
/// Index is `post_Y * multiplier`, clamped to `[0, 255]`.
/// Bit patterns are the exact f64 constants of the Python reference.
pub const FLOOR1_FROM_DB_LOOKUP: [f64; 256] = [
    f64::from_bits(0x3E7C9687B6622AFB),
    f64::from_bits(0x3E7E72211EF8F8A9),
    f64::from_bits(0x3E8036516543D7E7),
    f64::from_bits(0x3E81440778A2F267),
    f64::from_bits(0x3E8263446F17CCE6),
    f64::from_bits(0x3E83952C0CD58ECA),
    f64::from_bits(0x3E84DAF4D264C67D),
    f64::from_bits(0x3E8635E964ED21D7),
    f64::from_bits(0x3E87A76A0D03C211),
    f64::from_bits(0x3E8930EDF1EBBD1C),
    f64::from_bits(0x3E8AD4046B5B3A8D),
    f64::from_bits(0x3E8C9256F0E0D125),
    f64::from_bits(0x3E8E6DAA98B1DAC9),
    f64::from_bits(0x3E9033F10387E90A),
    f64::from_bits(0x3E91417F81CDA0E3),
    f64::from_bits(0x3E9260926CA81601),
    f64::from_bits(0x3E93924D2E3801AF),
    f64::from_bits(0x3E94D7E6308156D2),
    f64::from_bits(0x3E9632A7EBA28C35),
    f64::from_bits(0x3E97A3F26FE53E3E),
    f64::from_bits(0x3E992D3C8A7A14C6),
    f64::from_bits(0x3E9AD0159317375F),
    f64::from_bits(0x3E9C8E26B27B1C76),
    f64::from_bits(0x3E9E6934BB4D4B5C),
    f64::from_bits(0x3EA03190F63D4165),
    f64::from_bits(0x3EA13EF7F04D0B3C),
    f64::from_bits(0x3EA25DE0C9EBF417),
    f64::from_bits(0x3EA38F6EBA905754),
    f64::from_bits(0x3EA4D4D7FF34F0C9),
    f64::from_bits(0x3EA62F66E8902715),
    f64::from_bits(0x3EA7A07B435DC40C),
    f64::from_bits(0x3EA9298BC0A8AD1E),
    f64::from_bits(0x3EAACC275E149BC2),
    f64::from_bits(0x3EAC89F71CF7F639),
    f64::from_bits(0x3EAE64BF7B88FC9C),
    f64::from_bits(0x3EB02F31430507DB),
    f64::from_bits(0x3EB13C70B33DBCCE),
    f64::from_bits(0x3EB25B2F81424048),
    f64::from_bits(0x3EB38C90A69C41F5),
    f64::from_bits(0x3EB4D1CA333D469D),
    f64::from_bits(0x3EB62C265614CB96),
    f64::from_bits(0x3EB79D0492AFA13E),
    f64::from_bits(0x3EB925DB8394117F),
    f64::from_bits(0x3EBAC839BB6FF310),
    f64::from_bits(0x3EBC85C814319C06),
    f64::from_bits(0x3EBE604AE4A73C4F),
    f64::from_bits(0x3EC02CD1E16D8219),
    f64::from_bits(0x3EC139E9E9960B72),
    f64::from_bits(0x3EC2587EA92F961A),
    f64::from_bits(0x3EC389B30E8183FA),
    f64::from_bits(0x3EC4CEBCF131D50A),
    f64::from_bits(0x3EC628E647E481CF),
    f64::from_bits(0x3EC7998E7A00983E),
    f64::from_bits(0x3EC9222BBF8839D4),
    f64::from_bits(0x3ECAC44CAB293D49),
    f64::from_bits(0x3ECC8199C260B179),
    f64::from_bits(0x3ECE5BD6F9789DE3),
    f64::from_bits(0x3ED02A72E52AB835),
    f64::from_bits(0x3ED13763745FA14E),
    f64::from_bits(0x3ED255CE33A11459),
    f64::from_bits(0x3ED386D5D8EAEE6D),
    f64::from_bits(0x3ED4CBB018B3FC7E),
    f64::from_bits(0x3ED625A6AA4B41A8),
    f64::from_bits(0x3ED79618D1E898DE),
    f64::from_bits(0x3ED91E7C88392E32),
    f64::from_bits(0x3EDAC0602EA8C425),
    f64::from_bits(0x3EDC7D6BF643225B),
    f64::from_bits(0x3EDE5763B0231D50),
    f64::from_bits(0x3EE028143D593589),
    f64::from_bits(0x3EE134DD61AD5F98),
    f64::from_bits(0x3EE2531E2096BB06),
    f64::from_bits(0x3EE383F913EB6280),
    f64::from_bits(0x3EE4C8A3B0CD2D93),
    f64::from_bits(0x3EE6226792655CF1),
    f64::from_bits(0x3EE792A3B68D6588),
    f64::from_bits(0x3EE91ACDEBB9CFCE),
    f64::from_bits(0x3EEABC7437DBA671),
    f64::from_bits(0x3EEC793EE11B02E2),
    f64::from_bits(0x3EEE52F11DC30C62),
    f64::from_bits(0x3EF025B5E9F8FA17),
    f64::from_bits(0x3EF13257B17F464F),
    f64::from_bits(0x3EF2506E70108A1F),
    f64::from_bits(0x3EF3811CB16FFF01),
    f64::from_bits(0x3EF4C597B273F7AF),
    f64::from_bits(0x3EF61F28E40D1141),
    f64::from_bits(0x3EF78F2F165764BA),
    f64::from_bits(0x3EF9171FC6DAEBA5),
    f64::from_bits(0x3EFAB888E9F1172E),
    f64::from_bits(0x3EFC75124A9CCE3F),
    f64::from_bits(0x3EFE4E7F0D919E97),
    f64::from_bits(0x3F002357F2137677),
    f64::from_bits(0x3F012FD2507B5FCB),
    f64::from_bits(0x3F024DBF2C9CAA8C),
    f64::from_bits(0x3F037E40C3105D6F),
    f64::from_bits(0x3F04C28C364964EE),
    f64::from_bits(0x3F061BEAB9A5C4DA),
    f64::from_bits(0x3F078BBAF1469675),
    f64::from_bits(0x3F0913723784A046),
    f64::from_bits(0x3F0AB49E318F20B6),
    f64::from_bits(0x3F0C70E65B3ECBE8),
    f64::from_bits(0x3F0E4A0DBEE3C959),
    f64::from_bits(0x3F1020FA5689D8BE),
    f64::from_bits(0x3F112D4D5905124E),
    f64::from_bits(0x3F124B104584B0E0),
    f64::from_bits(0x3F137B6539D86E85),
    f64::from_bits(0x3F14BF8128125194),
    f64::from_bits(0x3F1618AD0A63AAFB),
    f64::from_bits(0x3F178847548CADD8),
    f64::from_bits(0x3F190FC52C1F5430),
    f64::from_bits(0x3F1AB0B3FD1E2986),
    f64::from_bits(0x3F1C6CBB1BCCC89C),
    f64::from_bits(0x3F1E459D0E8A59A5),
    f64::from_bits(0x3F201E9D175C20EB),
    f64::from_bits(0x3F212AC8B51EDDF7),
    f64::from_bits(0x3F224861C39469DA),
    f64::from_bits(0x3F23788A15C83241),
    f64::from_bits(0x3F24BC76909A8A62),
    f64::from_bits(0x3F26156FDAACAA05),
    f64::from_bits(0x3F2784D44029AAE5),
    f64::from_bits(0x3F290C18A4AB0763),
    f64::from_bits(0x3F2AACCA670197E1),
    f64::from_bits(0x3F2C6890531A1178),
    f64::from_bits(0x3F2E412D09B7029D),
    f64::from_bits(0x3F301C402525A8AD),
    f64::from_bits(0x3F312844815F1C37),
    f64::from_bits(0x3F3245B3AF97A23C),
    f64::from_bits(0x3F3375AF6CDD2886),
    f64::from_bits(0x3F34B96C5C1782A7),
    f64::from_bits(0x3F36123318E92876),
    f64::from_bits(0x3F378161A285F41A),
    f64::from_bits(0x3F39086CBB8B2020),
    f64::from_bits(0x3F3AA8E154D60585),
    f64::from_bits(0x3F3C64663A535960),
    f64::from_bits(0x3F3E3CBDA9D0EAB0),
    f64::from_bits(0x3F4019E390648FED),
    f64::from_bits(0x3F4125C0A015FA05),
    f64::from_bits(0x3F424305F9103A1B),
    f64::from_bits(0x3F4372D52C663E39),
    f64::from_bits(0x3F44B662A086BA43),
    f64::from_bits(0x3F460EF6D017E63F),
    f64::from_bits(0x3F477DEF88D33C97),
    f64::from_bits(0x3F4904C14B5D7836),
    f64::from_bits(0x3F4AA4F8DC98F253),
    f64::from_bits(0x3F4C603CC0FA806A),
    f64::from_bits(0x3F4E384EF45771D6),
    f64::from_bits(0x3F501787539976B3),
    f64::from_bits(0x3F51233D16C2D759),
    f64::from_bits(0x3F524058B07C5163),
    f64::from_bits(0x3F536FFB5463735C),
    f64::from_bits(0x3F54B35963679130),
    f64::from_bits(0x3F560BBB0038E360),
    f64::from_bits(0x3F577A7DE2936475),
    f64::from_bits(0x3F5901166A1F8F85),
    f64::from_bits(0x3F5AA11103C9BE43),
    f64::from_bits(0x3F5C5C13E190269F),
    f64::from_bits(0x3F5E33E0E94A9810),
    f64::from_bits(0x3F60152B71840CFC),
    f64::from_bits(0x3F6120BA0661F405),
    f64::from_bits(0x3F623DABBFDE6830),
    f64::from_bits(0x3F636D21E21517F1),
    f64::from_bits(0x3F64B0507B7EB7A7),
    f64::from_bits(0x3F66087FAECB7FD2),
    f64::from_bits(0x3F67770CD3825B80),
    f64::from_bits(0x3F68FD6C12520615),
    f64::from_bits(0x3F6A9D29A12D1990),
    f64::from_bits(0x3F6C57EBA4535BF3),
    f64::from_bits(0x3F6E2F73782C3D74),
    f64::from_bits(0x3F7012CFE4A4F2CE),
    f64::from_bits(0x3F711E373AB94051),
    f64::from_bits(0x3F723AFF2F758E77),
    f64::from_bits(0x3F736A48DDBA3BEE),
    f64::from_bits(0x3F74AD481D063D5F),
    f64::from_bits(0x3F760544D9100B98),
    f64::from_bits(0x3F77739C470279D6),
    f64::from_bits(0x3F78F9C243F4DBE6),
    f64::from_bits(0x3F7A9942E37DB3F8),
    f64::from_bits(0x3F7C53C3F766287E),
    f64::from_bits(0x3F7E2B06ABFB21F3),
    f64::from_bits(0x3F801074B48B4C1F),
    f64::from_bits(0x3F811BB4D994700A),
    f64::from_bits(0x3F8238530D003426),
    f64::from_bits(0x3F8367704073A75B),
    f64::from_bits(0x3F84AA401EC2D292),
    f64::from_bits(0x3F86020A714816C5),
    f64::from_bits(0x3F87702C2F554F8A),
    f64::from_bits(0x3F88F618FF0810F9),
    f64::from_bits(0x3F8A955CAF3EADA1),
    f64::from_bits(0x3F8C4F9CF6456C1B),
    f64::from_bits(0x3F8E269A84B7458D),
    f64::from_bits(0x3F900E19DA57E0F9),
    f64::from_bits(0x3F911932CE55DB4B),
    f64::from_bits(0x3F9235A74E2F854B),
    f64::from_bits(0x3F9364980DB0F635),
    f64::from_bits(0x3F94A738AD5F6301),
    f64::from_bits(0x3F95FED099CFB92A),
    f64::from_bits(0x3F976CBC96C9B08E),
    f64::from_bits(0x3F98F2704A6ADD44),
    f64::from_bits(0x3F9A91771C7D4A6A),
    f64::from_bits(0x3F9C4B767E950EF9),
    f64::from_bits(0x3F9E222F101F182F),
    f64::from_bits(0x3FA00BBF597A4D57),
    f64::from_bits(0x3FA116B12D9B29F9),
    f64::from_bits(0x3FA232FBEF93E5EA),
    f64::from_bits(0x3FA361C04A999274),
    f64::from_bits(0x3FA4A4319A7934ED),
    f64::from_bits(0x3FA5FB97296BA300),
    f64::from_bits(0x3FA7694D904576C6),
    f64::from_bits(0x3FA8EEC80E0FFCE7),
    f64::from_bits(0x3FAA8D92132C4674),
    f64::from_bits(0x3FAC4750AA1A22F4),
    f64::from_bits(0x3FAE1DC4362555FB),
    f64::from_bits(0x3FB0096532CE7837),
    f64::from_bits(0x3FB1142FDE7B3135),
    f64::from_bits(0x3FB23050FC5810F4),
    f64::from_bits(0x3FB35EE8F72D7C18),
    f64::from_bits(0x3FB4A12AFB89D737),
    f64::from_bits(0x3FB5F85E3149E02F),
    f64::from_bits(0x3FB765DEFDB80D5E),
    f64::from_bits(0x3FB8EB20680804BA),
    f64::from_bits(0x3FBA89AD934BA1BF),
    f64::from_bits(0x3FBC432B78D4A80D),
    f64::from_bits(0x3FBE195A192616C0),
    f64::from_bits(0x3FC0070B6208DEA1),
    f64::from_bits(0x3FC111AEE98CF6F4),
    f64::from_bits(0x3FC22DA66BE50075),
    f64::from_bits(0x3FC35C120F213026),
    f64::from_bits(0x3FC49E24D9284FD5),
    f64::from_bits(0x3FC5F525B16A70B7),
    f64::from_bits(0x3FC76270E36CF74F),
    f64::from_bits(0x3FC8E7794B706BCE),
    f64::from_bits(0x3FCA85C9AE096834),
    f64::from_bits(0x3FCC3F06D0FF8C68),
    f64::from_bits(0x3FCE14F07D0030D2),
    f64::from_bits(0x3FD004B1EB75038D),
    f64::from_bits(0x3FD10F2E666FCB95),
    f64::from_bits(0x3FD22AFC37C96FF6),
    f64::from_bits(0x3FD3593B859225B2),
    f64::from_bits(0x3FD49B1F154409F0),
    f64::from_bits(0x3FD5F1EDAE18D792),
    f64::from_bits(0x3FD75F035B294675),
    f64::from_bits(0x3FD8E3D2AD8C6AB2),
    f64::from_bits(0x3FDA81E669D6DE49),
    f64::from_bits(0x3FDC3AE2D4F6E7D3),
    f64::from_bits(0x3FDE1087A22050D8),
    f64::from_bits(0x3FE00258CAC76402),
    f64::from_bits(0x3FE10CAE33DA7806),
    f64::from_bits(0x3FE228526F0DA9E2),
    f64::from_bits(0x3FE356656762E5A8),
    f64::from_bits(0x3FE49819CCDAB99F),
    f64::from_bits(0x3FE5EEB626423402),
    f64::from_bits(0x3FE75B964E608B30),
    f64::from_bits(0x3FE8E02CA5FB51C5),
    f64::from_bits(0x3FEA7E03A67DADAB),
    f64::from_bits(0x3FEC36BF69E2C7B6),
    f64::from_bits(0x3FEE0C1F5D93590E),
    f64::from_bits(0x3FF0000000000000),
];

/// Full post X coordinates: `[0, 1 << rangebits] + x_list`
/// (Python `postlist_from_floor`).
pub fn postlist_from_floor(floor: &Floor1Setup) -> Vec<i64> {
    let mut postlist = Vec::with_capacity(2 + floor.x_list.len());
    postlist.push(0);
    postlist.push(1i64 << floor.rangebits);
    for &x in &floor.x_list {
        postlist.push(x as i64);
    }
    postlist
}

/// loneighbor / hineighbor for posts i=2..n-1 (libvorbis `floor1_look`).
///
/// For each post i in packet order (2..n-1), find the closest lower/higher
/// X among posts already "looked at" (indices 0..i-1). Returns lists of
/// length n-2 indexed by (i-2).
pub fn floor1_neighbor_tables(postlist: &[i64]) -> (Vec<i64>, Vec<i64>) {
    let n = postlist.len();
    if n < 2 {
        return (Vec::new(), Vec::new());
    }
    let hx_limit = postlist[1]; // 1<<rangebits
    let mut loneighbor = Vec::with_capacity(n - 2);
    let mut hineighbor = Vec::with_capacity(n - 2);
    for i in 0..n - 2 {
        let mut lo = 0i64;
        let mut hi = 1i64;
        let mut lx = 0i64;
        let mut hx = hx_limit;
        let currentx = postlist[i + 2];
        for (j, &x) in postlist.iter().enumerate().take(i + 2) {
            if x > lx && x < currentx {
                lo = j as i64;
                lx = x;
            }
            if x < hx && x > currentx {
                hi = j as i64;
                hx = x;
            }
        }
        loneighbor.push(lo);
        hineighbor.push(hi);
    }
    (loneighbor, hineighbor)
}

/// Linear interpolation prediction (libvorbis `render_point`; masks 0x8000).
pub fn render_point(x0: i64, x1: i64, y0: i64, y1: i64, x: i64) -> i64 {
    let y0 = y0 & 0x7FFF;
    let y1 = y1 & 0x7FFF;
    let dy = y1 - y0;
    let adx = x1 - x0;
    if adx == 0 {
        return y0;
    }
    let ady = dy.abs();
    let err = ady * (x - x0);
    // Python floor division on a non-negative numerator.
    let off = err / adx;
    if dy < 0 {
        y0 - off
    } else {
        y0 + off
    }
}

/// Inverse of [`floor1_wrap`]: packet residuals → absolute posts
/// (libvorbis `floor1_inverse1`'s second half, Python `floor1_unwrap`).
///
/// `fit_value[i] == 0` for `i >= 2` means "use the prediction": the result
/// carries the predicted value flagged with `0x8000`. Endpoints
/// `fit_value[0]`/`fit_value[1]` are absolute quant values.
///
/// The arithmetic mirrors Python's unbounded integers, so it is total over
/// the whole `i64` input domain: an adversarially large residual selects a
/// branch but never traps or wraps.
pub fn floor1_unwrap(
    fit_value: &[i64],
    postlist: &[i64],
    range_: i64,
) -> Result<Vec<i64>, Floor1Error> {
    let posts = fit_value.len();
    if posts != postlist.len() {
        return Err(Floor1Error::LengthMismatch {
            posts,
            postlist: postlist.len(),
        });
    }
    let (loneighbor, hineighbor) = floor1_neighbor_tables(postlist);
    let mut out = fit_value.to_vec();
    for i in 2..posts {
        let lo = loneighbor[i - 2] as usize;
        let hi = hineighbor[i - 2] as usize;
        let predicted = render_point(postlist[lo], postlist[hi], out[lo], out[hi], postlist[i]);
        // i128 keeps the reference's unbounded-integer branches exact for
        // every i64 residual instead of overflowing.
        let hiroom = range_ as i128 - predicted as i128;
        let loroom = predicted as i128;
        let room = hiroom.min(loroom) << 1;
        let val = out[i] as i128;
        if val != 0 {
            let val = if val >= room {
                if hiroom > loroom {
                    val - loroom
                } else {
                    -1 - (val - hiroom)
                }
            } else if val & 1 != 0 {
                -((val + 1) >> 1)
            } else {
                val >> 1
            };
            out[i] = ((val + predicted as i128) & 0x7FFF) as i64;
            out[lo] &= 0x7FFF;
            out[hi] &= 0x7FFF;
        } else {
            out[i] = predicted | 0x8000;
        }
    }
    Ok(out)
}

/// Decode one channel's floor1 Y posts from the packet (Python
/// `decode_floor1_body`; libvorbis `floor1_inverse1`'s first half).
///
/// The returned row is *wrapped residuals*, not absolute posts: pass it to
/// [`floor1_unwrap`] with the floor's [`postlist_from_floor`] and range to
/// recover absolute posts (the caller reads the per-channel nonzero flag
/// before calling; `nonzero != 0` is already true here).
pub fn decode_floor1_body(
    br: &mut BitReader<'_>,
    floor: &Floor1Setup,
    books: &[Codebook],
) -> Result<Vec<i64>, Floor1DecodeError> {
    if !(1..=4).contains(&floor.multiplier) {
        return Err(Floor1DecodeError::InvalidMultiplier {
            multiplier: floor.multiplier,
        });
    }
    let rng = FLOOR1_RANGES[floor.multiplier as usize];
    let ybits = ilog(rng - 1);
    let nvals = 2 + floor.x_list.len();
    let mut y = vec![0i64; nvals];
    y[0] = br.read(ybits)? as i64;
    y[1] = br.read(ybits)? as i64;
    let mut ppos = 2usize;
    for &class in &floor.partition_classes {
        let class = class as usize;
        let cdim = *floor
            .class_dims
            .get(class)
            .ok_or(Floor1DecodeError::ClassTableIncomplete { class })?;
        let cbits = *floor
            .class_subs
            .get(class)
            .ok_or(Floor1DecodeError::ClassTableIncomplete { class })?;
        let subclass_books = floor
            .subclass_books
            .get(class)
            .ok_or(Floor1DecodeError::ClassTableIncomplete { class })?;
        let csub = (1u64 << cbits) - 1;
        let mut cval = if cbits != 0 {
            let master_id = floor
                .class_masterbooks
                .get(class)
                .copied()
                .flatten()
                .ok_or(Floor1DecodeError::MasterBookMissing { class })?;
            let master =
                books
                    .get(master_id as usize)
                    .ok_or(Floor1DecodeError::BookIndexOutOfRange {
                        book_id: master_id as i64,
                        books: books.len(),
                    })?;
            master.decode(br)? as u64
        } else {
            0
        };
        for _ in 0..cdim {
            let index = (cval & csub) as usize;
            // `cval & csub` lies inside the class's declared subclass domain;
            // a hand-built setup whose row is shorter is reported, not read
            // past.
            let book_id =
                *subclass_books
                    .get(index)
                    .ok_or(Floor1DecodeError::SubclassBookOutOfRange {
                        class,
                        index,
                        subclass_books: subclass_books.len(),
                    })?;
            cval >>= cbits;
            // The reference decodes the codeword before indexing the post
            // row, so an over-long partition list is reported after the bits
            // that codeword consumed rather than before them.
            let entry = if book_id >= 0 {
                let book =
                    books
                        .get(book_id as usize)
                        .ok_or(Floor1DecodeError::BookIndexOutOfRange {
                            book_id,
                            books: books.len(),
                        })?;
                Some(book.decode(br)?)
            } else {
                None
            };
            if ppos >= nvals {
                // Every further post, coded or not, consumes a slot.
                return Err(Floor1DecodeError::PostCountMismatch {
                    decoded: ppos + 1,
                    expected: nvals,
                });
            }
            if let Some(entry) = entry {
                y[ppos] = entry;
            }
            ppos += 1;
        }
    }
    if ppos != nvals {
        return Err(Floor1DecodeError::PostCountMismatch {
            decoded: ppos,
            expected: nvals,
        });
    }
    Ok(y)
}

/// Encode residual wrap: absolute posts → packet Y (libvorbis
/// `floor1_encode`, Python `floor1_wrap`).
pub fn floor1_wrap(posts: &[i64], postlist: &[i64], range_: i64) -> Result<Vec<i64>, Floor1Error> {
    Ok(floor1_wrap_with_posts(posts, postlist, range_)?.1)
}

/// Return both floor1's normalized absolute posts and packet residuals
/// (Python `floor1_wrap_with_posts`).
///
/// `floor1_encode` modifies its quantized post vector while producing the
/// coded residuals: predicted posts are replaced by their prediction and
/// marked unused, while neighbors needed by a coded post have their unused
/// flag cleared. The modified vector is also the input to the integer floor
/// rasterizer used by the following residue quantizer.
pub fn floor1_wrap_with_posts(
    posts: &[i64],
    postlist: &[i64],
    range_: i64,
) -> Result<(Vec<i64>, Vec<i64>), Floor1Error> {
    let n = posts.len();
    if n != postlist.len() {
        return Err(Floor1Error::LengthMismatch {
            posts: n,
            postlist: postlist.len(),
        });
    }
    if n < 2 {
        return Err(Floor1Error::TooFewPosts { posts: n });
    }
    let (loneighbor, hineighbor) = floor1_neighbor_tables(postlist);
    // Work on a mutable copy; encode clears flags on used neighbors.
    let mut post: Vec<i64> = posts.to_vec();
    let mut out = vec![0i64; n];
    out[0] = post[0] & 0x7FFF;
    out[1] = post[1] & 0x7FFF;
    for i in 2..n {
        let ln = loneighbor[i - 2] as usize;
        let hn = hineighbor[i - 2] as usize;
        let predicted = render_point(postlist[ln], postlist[hn], post[ln], post[hn], postlist[i]);
        let pi = post[i];
        if (pi & 0x8000) != 0 || predicted == pi {
            post[i] = predicted | 0x8000;
            out[i] = 0;
        } else {
            let headroom = if range_ - predicted < predicted {
                range_ - predicted
            } else {
                predicted
            };
            let mut val = (pi & 0x7FFF) - predicted;
            if val < 0 {
                if val < -headroom {
                    val = headroom - val - 1;
                } else {
                    val = -1 - (val * 2);
                }
            } else {
                if val >= headroom {
                    val += headroom;
                } else {
                    val <<= 1;
                }
            }
            out[i] = val;
            post[ln] &= 0x7FFF;
            post[hn] &= 0x7FFF;
        }
    }
    Ok((post, out))
}

/// libvorbis `render_line`: multiply d[x] by FLOOR1_fromdB_LOOKUP[y]
/// (Python `_render_line_fromdB`).
fn render_line_from_db(n: usize, x0: i64, x1: i64, y0: i64, y1: i64, d: &mut [f64]) {
    if x0 >= x1 {
        return;
    }
    let dy = y1 - y0;
    let adx = x1 - x0;
    let mut ady = dy.abs();
    // C integer division truncates toward zero.
    let base_step = if adx != 0 {
        ((dy as f64) / (adx as f64)).trunc() as i64
    } else {
        0
    };
    let sy = if dy < 0 { base_step - 1 } else { base_step + 1 };
    let mut x = x0;
    let mut y = y0;
    let mut err = 0i64;
    ady -= (base_step * adx).abs();
    let limit = if n as i64 > x1 { x1 } else { n as i64 };
    if x < limit {
        let yi = y.clamp(0, 255) as usize;
        d[x as usize] *= FLOOR1_FROM_DB_LOOKUP[yi];
    }
    while x + 1 < limit {
        x += 1;
        err += ady;
        if err >= adx {
            err -= adx;
            y += sy;
        } else {
            y += base_step;
        }
        let yi = y.clamp(0, 255) as usize;
        d[x as usize] *= FLOOR1_FROM_DB_LOOKUP[yi];
    }
}

/// Piecewise-linear floor curve, n spectrum bins (libvorbis
/// `floor1_inverse2`, Python `floor1_curve_from_posts`).
///
/// Absolute posts (0x8000 = skip / predicted-only) × multiplier → quant
/// 0..255, then amp = `FLOOR1_fromdB_LOOKUP[q]`. Output is linear amplitude
/// (starts as 1.0 and is multiplied along active segments; inactive tail
/// holds the final level).
pub fn floor1_curve_from_posts(
    posts: &[i64],
    postlist: &[i64],
    n: usize,
    multiplier: u64,
) -> Result<Vec<f64>, Floor1Error> {
    let nposts = posts.len();
    if nposts != postlist.len() {
        return Err(Floor1Error::LengthMismatch {
            posts: nposts,
            postlist: postlist.len(),
        });
    }
    // Sort order by X (forward_index); stable like Python's sorted().
    let mut order: Vec<usize> = (0..nposts).collect();
    order.sort_by_key(|&i| postlist[i]);
    let mut out = vec![1.0f64; n];
    let mut ly = (posts[0] & 0x7FFF) as u64 * multiplier;
    ly = ly.clamp(0, 255);
    let mut lx = 0i64;
    let mut hx = 0i64;
    for &current in order.iter().take(nposts).skip(1) {
        let hy = posts[current] & 0x7FFF;
        // active if no 0x8000 flag (libvorbis: hy == fit_value[current])
        if hy == posts[current] {
            hx = postlist[current];
            let mut hyq = hy as u64 * multiplier;
            hyq = hyq.clamp(0, 255);
            render_line_from_db(n, lx, hx, ly as i64, hyq as i64, &mut out);
            lx = hx;
            ly = hyq;
        }
    }
    for item in out.iter_mut().take(n).skip(hx as usize) {
        *item *= FLOOR1_FROM_DB_LOOKUP[ly as usize];
    }
    Ok(out)
}

/// Render floor1 in the integer 0..255 domain consumed by residue
/// quantization (Python `floor1_quant_curve_from_posts`).
pub fn floor1_quant_curve_from_posts(
    posts: &[i64],
    postlist: &[i64],
    n: usize,
    multiplier: u64,
) -> Result<Vec<i64>, Floor1Error> {
    if posts.len() != postlist.len() {
        return Err(Floor1Error::LengthMismatch {
            posts: posts.len(),
            postlist: postlist.len(),
        });
    }
    let mut order: Vec<usize> = (0..posts.len()).collect();
    order.sort_by_key(|&index| postlist[index]);
    let mut ly = ((posts[0] & 0x7fff) * multiplier as i64).clamp(0, 255);
    let mut lx = 0i64;
    let mut hx = 0i64;
    let mut out = vec![ly; n];
    for &current in order.iter().skip(1) {
        let mut hy = posts[current] & 0x7fff;
        if hy == posts[current] {
            hx = postlist[current];
            hy = (hy * multiplier as i64).clamp(0, 255);
            render_line_quant(lx, hx, ly, hy, &mut out);
            lx = hx;
            ly = hy;
        }
    }
    for value in out.iter_mut().skip(hx.max(0) as usize) {
        *value = ly;
    }
    Ok(out)
}

fn render_line_quant(x0: i64, x1: i64, y0: i64, y1: i64, out: &mut [i64]) {
    if x0 >= x1 || x0 < 0 || x0 as usize >= out.len() {
        return;
    }
    let dy = y1 - y0;
    let adx = x1 - x0;
    let mut ady = dy.abs();
    let base_step = dy / adx;
    let sy = if dy < 0 { base_step - 1 } else { base_step + 1 };
    let mut x = x0;
    let mut y = y0;
    let mut err = 0i64;
    ady -= (base_step * adx).abs();
    let limit = x1.min(out.len() as i64);
    if x < limit {
        out[x as usize] = y;
    }
    while x + 1 < limit {
        x += 1;
        err += ady;
        if err >= adx {
            err -= adx;
            y += sy;
        } else {
            y += base_step;
        }
        out[x as usize] = y;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn neighbor_tables_oracle() {
        // postlist: [0, 128, 32, 64, 96] (packet order ≠ sorted order).
        let pl = vec![0, 128, 32, 64, 96];
        let (lo, hi) = floor1_neighbor_tables(&pl);
        // i=2 (x=32): lo=0 (x=0), hi=1 (x=128)
        // i=3 (x=64): lo=2 (x=32), hi=1 (x=128)
        // i=4 (x=96): lo=3 (x=64), hi=1 (x=128)
        assert_eq!(lo, [0, 2, 3]);
        assert_eq!(hi, [1, 1, 1]);
    }

    #[test]
    fn render_point_matches_python() {
        assert_eq!(render_point(0, 10, 4, 8, 5), 6);
        assert_eq!(render_point(0, 10, 8, 4, 5), 6);
        assert_eq!(render_point(5, 5, 3, 9, 5), 3); // adx == 0
                                                    // 0x8000 flags are masked.
        assert_eq!(render_point(0, 10, 0x8004, 0x8008, 5), 6);
    }

    #[test]
    fn wrap_roundtrip() {
        // Absolute posts with a flagged interior; wrap→unwrap identity on Y.
        let pl = vec![0, 128, 32, 64, 96];
        let posts = vec![6, 10, 0x8000, 8, 4];
        let (raster, y) = floor1_wrap_with_posts(&posts, &pl, 128).unwrap();
        // Flagged posts become predicted | 0x8000, coded residuals for the rest.
        assert_eq!(y[0], 6);
        assert_eq!(y[1], 10);
        assert_eq!(raster[2] & 0x8000, 0x8000);
        assert_eq!(y[2], 0);
    }

    // ---- decode direction (segment 6) ----

    use crate::bitio::OggPack;
    use crate::codebook::StaticCodebook;
    use crate::packet_encoder::pack_floor1_body;
    use crate::setup::{BitPositions, SetupInfo};

    /// A maptype-0 book whose `entries` codewords all have the same length:
    /// a complete code, so `decode` returns exactly the entry the packer
    /// chose for any value inside `0..entries`.
    fn maptype0_book(entries: i64, length: i64) -> Codebook {
        Codebook::from_static(
            StaticCodebook {
                dim: 1,
                entries,
                lengthlist: vec![length; entries as usize],
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
        .expect("synthetic maptype-0 book must build")
    }

    /// One floor with `partitions` partitions, all class 0, plus the book list
    /// the packer and the decoder both index.
    #[allow(clippy::too_many_arguments)] // one parameter per Floor1Setup field group
    fn floor_with_subclass_book(
        partition_classes: Vec<u64>,
        x_list: Vec<u64>,
        class_dims: Vec<u64>,
        class_subs: Vec<u64>,
        class_masterbooks: Vec<Option<u64>>,
        subclass_books: Vec<Vec<i64>>,
        multiplier: u64,
        rangebits: u64,
    ) -> Floor1Setup {
        Floor1Setup {
            partitions: partition_classes.len() as u64,
            partition_classes,
            max_class: 0,
            class_dims,
            class_subs,
            class_masterbooks,
            subclass_books,
            multiplier,
            rangebits,
            x_list,
            bit_start: 0,
            bit_end: 0,
        }
    }

    fn setup_with_floor(floor: Floor1Setup) -> SetupInfo {
        SetupInfo {
            setup_size: 0,
            bits_total: 0,
            channels: 2,
            nbooks: 1,
            book_ids: vec![0],
            unique_book_ids: vec![0],
            nfloors: 1,
            floors: vec![floor],
            nresidues: 1,
            residues: Vec::new(),
            nmaps: 1,
            maps: Vec::new(),
            nmodes: 1,
            modes: Vec::new(),
            bit_positions: BitPositions {
                after_books: 0,
                after_floor_count: 0,
                after_floors: 0,
                after_residues: 0,
                after_maps: 0,
                after_modes: 0,
                end: 0,
            },
            trailing_pad_bits: 0,
            trailing_pad_value: 0,
            parse_complete: true,
            book_id_assignment: "",
        }
    }

    #[test]
    fn unwrap_round_trips_the_wrap_direction() {
        // Two interior posts, one of them equal to its own prediction (so the
        // wrap codes it as "use predicted"), one flagged unused by the fit.
        let pl = vec![0, 128, 32, 64, 96];
        for posts in [
            vec![6, 10, 20, 8, 4],
            vec![6, 10, 20, 0x8000, 4],
            vec![0, 0, 0, 0, 0],
            vec![255, 255, 255, 255, 255],
        ] {
            let (raster, y) = floor1_wrap_with_posts(&posts, &pl, 256).unwrap();
            let recovered = floor1_unwrap(&y, &pl, 256).unwrap();
            assert_eq!(
                recovered, raster,
                "unwrap must invert wrap for posts {posts:?}"
            );
        }
    }

    #[test]
    fn unwrap_flags_predicted_posts_with_0x8000() {
        // Y[2] == 0 and Y[3] == 0 mean "use the prediction": post 2 comes
        // back flagged with its predicted height (render_point of the
        // endpoints at x=32 is 7), and post 3 — used as a neighbour by the
        // coded post 4 — has the flag cleared again, exactly as the wrap
        // direction clears the neighbours it codes against.
        let pl = vec![0, 128, 32, 64, 96];
        let y = vec![6, 10, 0, 0, 4];
        let out = floor1_unwrap(&y, &pl, 256).unwrap();
        assert_eq!(out[2], 7 | 0x8000);
        assert_eq!(out[3] & 0x8000, 0, "a coded neighbour clears the flag");
        assert_eq!(out[3], 8);
        assert_eq!(out[4], 11);
        assert_eq!(out[0], 6);
        assert_eq!(out[1], 10);
    }

    #[test]
    fn unwrap_is_total_over_extreme_residuals() {
        // Python's unbounded integers never trap; neither may this path. A
        // residual at the i64 boundary must select a branch, not overflow.
        let pl = vec![0, 128, 32, 64, 96];
        let y = vec![i64::MIN, i64::MAX, i64::MIN, i64::MAX, i64::MIN];
        let out = floor1_unwrap(&y, &pl, i64::MIN).unwrap();
        for value in &out {
            assert!(*value >= 0, "masked post must be non-negative: {value}");
        }
        // Endpoints are masked absolute values.
        assert_eq!(out[0], 0);
        assert_eq!(out[1], 0x7FFF);
    }

    #[test]
    fn unwrap_rejects_a_postlist_length_mismatch() {
        let err = floor1_unwrap(&[1, 2, 3], &[0, 128], 256).unwrap_err();
        assert_eq!(
            err,
            Floor1Error::LengthMismatch {
                posts: 3,
                postlist: 2
            }
        );
    }

    #[test]
    fn floor1_body_bitstream_round_trips_through_the_packer() {
        // 4 x values ⇒ 6 posts; two class-0 partitions of dim 2.
        let x_list = vec![32, 64, 96, 16];
        let floor = floor_with_subclass_book(
            vec![0, 0],
            x_list,
            vec![2],
            vec![0],
            vec![None],
            vec![vec![0]],
            1,
            7,
        );
        let setup = setup_with_floor(floor.clone());
        let books = vec![maptype0_book(256, 8)];
        let pl = postlist_from_floor(&floor);
        assert_eq!(pl, vec![0, 128, 32, 64, 96, 16]);
        let posts = vec![7, 200, 31, 64, 96, 250];
        let (_, y) = floor1_wrap_with_posts(&posts, &pl, 256).unwrap();

        let mut op = OggPack::new(64);
        pack_floor1_body(&mut op, &setup, 0, &books, &y).unwrap();
        let bytes = op.into_buffer();

        let mut br = BitReader::new(&bytes);
        let decoded = decode_floor1_body(&mut br, &floor, &books).unwrap();
        assert_eq!(decoded, y);
        // The body is the whole payload; the reader consumed it all.
        assert!(br.bits_left() < 8);
    }

    #[test]
    fn floor1_body_round_trips_through_a_master_and_subclass_books() {
        // Class 0 has subs = 1: a 4-entry master book selects the subclass
        // book of each dim of the partition.
        let x_list = vec![16, 48, 80, 112];
        let floor = floor_with_subclass_book(
            vec![0, 0],
            x_list,
            vec![2],
            vec![1],
            vec![Some(1)],
            vec![vec![0, 0]],
            1,
            7,
        );
        let setup = setup_with_floor(floor.clone());
        let books = vec![maptype0_book(256, 8), maptype0_book(4, 2)];
        let pl = postlist_from_floor(&floor);
        let posts = vec![3, 250, 40, 60, 0x8000, 90];
        let (_, y) = floor1_wrap_with_posts(&posts, &pl, 256).unwrap();

        let mut op = OggPack::new(64);
        pack_floor1_body(&mut op, &setup, 0, &books, &y).unwrap();
        let bytes = op.into_buffer();

        let mut br = BitReader::new(&bytes);
        let decoded = decode_floor1_body(&mut br, &floor, &books).unwrap();
        assert_eq!(decoded, y);
    }

    #[test]
    fn floor1_body_handles_a_no_bits_subclass_entry() {
        // Class 0, subs = 1: subclass index 0 is coded by book 0, index 1
        // writes no bits and decodes as residual 0 (libvorbis's `book < 0`).
        let floor = floor_with_subclass_book(
            vec![0, 0],
            vec![16, 48, 80, 112],
            vec![2],
            vec![1],
            vec![Some(1)],
            vec![vec![0, -1]],
            1,
            7,
        );
        let books = vec![maptype0_book(256, 8), maptype0_book(4, 2)];
        // Hand-built schedule: endpoints, then per partition a master entry
        // selecting cval = 2 (bit 0 set: dim 0 coded, dim 1 no-bits).
        let mut op = OggPack::new(32);
        op.write(7, 8).unwrap();
        op.write(200, 8).unwrap();
        for residual in [55u64, 77] {
            books[1].encode(&mut op, 2).unwrap();
            books[0].encode(&mut op, residual as i64).unwrap();
        }
        let bytes = op.into_buffer();

        let mut br = BitReader::new(&bytes);
        let decoded = decode_floor1_body(&mut br, &floor, &books).unwrap();
        assert_eq!(decoded, vec![7, 200, 55, 0, 77, 0]);
        assert!(br.bits_left() < 8);
    }

    #[test]
    fn malformed_floor1_body_inputs_return_errors() {
        let floor = floor_with_subclass_book(
            vec![0],
            vec![32, 64],
            vec![2],
            vec![0],
            vec![None],
            vec![vec![0]],
            1,
            7,
        );
        let books = vec![maptype0_book(256, 8)];

        // Empty payload: the endpoint read runs out of bits.
        let mut br = BitReader::new(&[]);
        assert_eq!(
            decode_floor1_body(&mut br, &floor, &books).unwrap_err(),
            Floor1DecodeError::Bit(BitError::OutOfBits)
        );

        // Truncated after the endpoints.
        let mut br = BitReader::new(&[0u8]);
        assert!(matches!(
            decode_floor1_body(&mut br, &floor, &books).unwrap_err(),
            Floor1DecodeError::Bit(BitError::OutOfBits)
        ));

        // Multiplier outside 1..=4 (the reference's range table KeyError).
        let mut bad = floor.clone();
        bad.multiplier = 0;
        let mut br = BitReader::new(&[0xFF, 0xFF]);
        assert_eq!(
            decode_floor1_body(&mut br, &bad, &books).unwrap_err(),
            Floor1DecodeError::InvalidMultiplier { multiplier: 0 }
        );

        // A class record the partition list names but does not carry.
        let mut bad = floor.clone();
        bad.class_dims.clear();
        let mut br = BitReader::new(&[0xFF, 0xFF]);
        assert_eq!(
            decode_floor1_body(&mut br, &bad, &books).unwrap_err(),
            Floor1DecodeError::ClassTableIncomplete { class: 0 }
        );

        // subs != 0 requires a master book.
        let mut bad = floor.clone();
        bad.class_subs = vec![1];
        bad.subclass_books = vec![vec![0, 0]];
        let mut br = BitReader::new(&[0xFF, 0xFF]);
        assert_eq!(
            decode_floor1_body(&mut br, &bad, &books).unwrap_err(),
            Floor1DecodeError::MasterBookMissing { class: 0 }
        );

        // A subclass book id the caller did not supply.
        let bad = floor_with_subclass_book(
            vec![0],
            vec![32, 64],
            vec![2],
            vec![0],
            vec![None],
            vec![vec![7]],
            1,
            7,
        );
        let mut br = BitReader::new(&[0xFF, 0xFF]);
        assert_eq!(
            decode_floor1_body(&mut br, &bad, &books).unwrap_err(),
            Floor1DecodeError::BookIndexOutOfRange {
                book_id: 7,
                books: 1
            }
        );

        // The partition list codes more posts than x_list declares.
        let over = floor_with_subclass_book(
            vec![0, 0],
            vec![32, 64],
            vec![2],
            vec![0],
            vec![None],
            vec![vec![0]],
            1,
            7,
        );
        let mut br = BitReader::new(&[0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF]);
        assert_eq!(
            decode_floor1_body(&mut br, &over, &books).unwrap_err(),
            Floor1DecodeError::PostCountMismatch {
                decoded: 5,
                expected: 4
            }
        );

        // The partition list codes fewer posts than x_list declares.
        let under = floor_with_subclass_book(
            vec![0],
            vec![32, 64, 96],
            vec![1],
            vec![0],
            vec![None],
            vec![vec![0]],
            1,
            7,
        );
        let mut br = BitReader::new(&[0xFF, 0xFF, 0xFF]);
        assert_eq!(
            decode_floor1_body(&mut br, &under, &books).unwrap_err(),
            Floor1DecodeError::PostCountMismatch {
                decoded: 3,
                expected: 5
            }
        );
    }

    #[test]
    fn decode_errors_expose_their_cause() {
        use std::error::Error;
        let err = Floor1DecodeError::Bit(BitError::OutOfBits);
        assert!(err.source().is_some());
        let err = Floor1DecodeError::Codebook(CodebookError::EmptyCodebook);
        assert!(err.source().is_some());
        assert!(Floor1DecodeError::MasterBookMissing { class: 0 }
            .source()
            .is_none());
    }
}
