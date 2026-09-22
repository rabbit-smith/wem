//! Pure Vorbis Huffman and maptype-1 VQ codebook core
//! (Python: `wwise_wem/vorbis/codebook.py`).
//!
//! Structures mirror Python field for field: `StaticCodebook`
//! (dim/entries/lengthlist/maptype/q_min/q_delta/q_quant/q_sequencep/
//! quantlist), the runtime `Codebook` (codelist, decode tree, valuallist,
//! quantvals), and the `CodebookRow` descriptor parsed from the profile's
//! decoded book tables (Python's raw dict rows).

use std::collections::HashMap;

use crate::bitio::{BitReader, OggPack};

/// Codebook construction/usage errors (Python: `ValueError` family).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CodebookError {
    /// The book table row is missing required fields.
    RowMissingField { field: &'static str },
    /// lengthlist length disagrees with entries.
    LengthlistMismatch { got: usize, want: i64 },
    /// dim must be >= 1.
    DimTooSmall { dim: i64 },
    /// overpopulated Huffman tree (Python `_make_words`).
    OverpopulatedTree { entry: i64, length: i64 },
    /// A codeword length outside the `0..=32` the marker table and the wire
    /// format express (Python `IndexError` / the 5-bit first-length field).
    LengthOutOfRange { entry: i64, length: i64 },
    /// code prefix collision (Python `_build_decode_tree`).
    CodePrefixCollision { entry: i64 },
    /// code embeds an earlier leaf (Python `_build_decode_tree`).
    CodeEmbedsLeaf { leaf: i64, entry: i64 },
    /// maptype 1 requires a quantlist.
    Maptype1RequiresQuantlist,
    /// quantvals must be positive for maptype1.
    QuantvalsNonPositive,
    /// quantlist length < quantvals.
    QuantlistTooShort { got: usize, want: i64 },
    /// entry out of range.
    EntryOutOfRange { entry: i64, entries: i64 },
    /// entry is unused (length 0).
    EntryUnused { entry: i64 },
    /// empty codebook (no used entries).
    EmptyCodebook,
    /// invalid Huffman code on the wire.
    InvalidHuffmanCode,
    /// decode_vq on a non-maptype-1 book.
    NotMaptype1 { maptype: i64 },
    /// no VQ vector for the entry.
    NoVqVector { entry: i64 },
    /// vq_values/best_vq misuse.
    VqMisuse { reason: &'static str },
    /// target length < dim.
    TargetTooShort { got: usize, dim: i64 },
    /// no usable VQ entries.
    NoUsableVqEntries,
    /// A packer write width outside `0..=32` (Python `ValueError` from
    /// `OggPack.write`; the lengthlist or `q_quant` carries the width).
    PackBitsOutOfRange { bits: u32 },
}

impl std::fmt::Display for CodebookError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            CodebookError::RowMissingField { field } => {
                write!(f, "book row missing field {field}")
            }
            CodebookError::LengthlistMismatch { got, want } => {
                write!(f, "lengthlist len {got} != entries {want}")
            }
            CodebookError::DimTooSmall { dim } => write!(f, "dim {dim} must be >= 1"),
            CodebookError::OverpopulatedTree { entry, length } => {
                write!(
                    f,
                    "overpopulated Huffman tree at entry {entry} length {length}"
                )
            }
            CodebookError::LengthOutOfRange { entry, length } => {
                write!(
                    f,
                    "codeword length {length} at entry {entry} is outside 0..=32"
                )
            }
            CodebookError::CodePrefixCollision { entry } => {
                write!(f, "code prefix collision at entry {entry}")
            }
            CodebookError::CodeEmbedsLeaf { leaf, entry } => {
                write!(f, "code embeds earlier leaf (entry {leaf}) at {entry}")
            }
            CodebookError::Maptype1RequiresQuantlist => {
                write!(f, "maptype 1 requires quantlist")
            }
            CodebookError::QuantvalsNonPositive => {
                write!(f, "quantvals must be positive for maptype1")
            }
            CodebookError::QuantlistTooShort { got, want } => {
                write!(f, "quantlist length {got} < quantvals {want}")
            }
            CodebookError::EntryOutOfRange { entry, entries } => {
                write!(f, "entry {entry} out of range 0..{entries}")
            }
            CodebookError::EntryUnused { entry } => {
                write!(f, "entry {entry} is unused (length 0)")
            }
            CodebookError::EmptyCodebook => write!(f, "empty codebook (no used entries)"),
            CodebookError::InvalidHuffmanCode => {
                write!(f, "invalid Huffman code (no matching entry)")
            }
            CodebookError::NotMaptype1 { maptype } => {
                write!(f, "decode_vq requires maptype 1, got {maptype}")
            }
            CodebookError::NoVqVector { entry } => {
                write!(f, "entry {entry} has no VQ vector (unused)")
            }
            CodebookError::VqMisuse { reason } => write!(f, "vq misuse: {reason}"),
            CodebookError::TargetTooShort { got, dim } => {
                write!(f, "target len {got} < dim {dim}")
            }
            CodebookError::NoUsableVqEntries => write!(f, "no usable VQ entries"),
            CodebookError::PackBitsOutOfRange { bits } => {
                write!(f, "pack bits must be 0..32, got {bits}")
            }
        }
    }
}

impl std::error::Error for CodebookError {}

/// Bit count needed to represent values in `[0, 2^r)` (Python `codebook.ilog`):
/// `ilog(0) = 0`, `ilog(1) = 1`, `ilog(2) = 1`, `ilog(3) = 2`.
#[inline]
pub fn ilog_bits(v: u64) -> u32 {
    64 - v.leading_zeros()
}

/// Static codebook descriptor (Python `StaticCodebook`).
#[derive(Debug, Clone, PartialEq)]
pub struct StaticCodebook {
    pub dim: i64,
    pub entries: i64,
    pub lengthlist: Vec<i64>,
    pub maptype: i64,
    /// Stored as parsed (Python keeps ints; pack/unquantize mask to 32 bits).
    pub q_min: i64,
    pub q_delta: i64,
    pub q_quant: i64,
    pub q_sequencep: i64,
    pub quantlist: Option<Vec<i64>>,
}

impl StaticCodebook {
    /// True when the lengthlist is non-decreasing and starts non-zero.
    ///
    /// A lengthlist shorter than `entries` (Python `IndexError`) reports
    /// `false` rather than reading past it.
    pub fn is_ordered(&self) -> bool {
        let n = self.entries;
        let lengths = &self.lengthlist;
        if n <= 1 {
            return true;
        }
        let Some(&first) = lengths.first() else {
            return false;
        };
        if first == 0 {
            return false;
        }
        if n < 0 || n as usize > lengths.len() {
            return false;
        }
        for i in 1..n as usize {
            if lengths[i] < lengths[i - 1] {
                return false;
            }
        }
        true
    }

    /// Pack the Wwise static-codebook representation (Python `pack`).
    ///
    /// Every write propagates: a write width outside `0..=32` (a lengthlist
    /// or `q_quant` the wire format cannot carry) is reported instead of
    /// truncating the packet, and a lengthlist that does not cover `entries`
    /// is reported instead of reading past it.
    pub fn pack(&self) -> Result<Vec<u8>, CodebookError> {
        let mut op = OggPack::new(256);
        let mut write = |value: u64, bits: u32| -> Result<(), CodebookError> {
            op.write(value, bits)
                .map_err(|_| CodebookError::PackBitsOutOfRange { bits })
        };
        let lengths = &self.lengthlist;
        let n = self.entries;
        if n < 0 || n as usize > lengths.len() {
            return Err(CodebookError::LengthlistMismatch {
                got: lengths.len(),
                want: n,
            });
        }
        let n = n as usize;
        // Codeword lengths live in the `0..=32` domain the wire format and the
        // word builder's marker table express (the first length is written as
        // `length - 1` into five bits); a longer value cannot be serialized,
        // and in the ordered branch it would run one length step per unit of
        // the delta.
        for (entry, &length) in lengths.iter().enumerate() {
            if length > 32 {
                return Err(CodebookError::LengthOutOfRange {
                    entry: entry as i64,
                    length,
                });
            }
        }
        write(self.dim as u64, 4)?;
        write(self.entries as u64, 14)?;
        let max_len = lengths.iter().copied().max().unwrap_or(0) as u64;
        let nob = ilog_bits(max_len);

        if self.is_ordered() {
            write(1, 1)?;
            // The oracle masks each `value - 1` write, so a zero or negative
            // first length wraps instead of underflowing.
            let first = *lengths.first().ok_or(CodebookError::EmptyCodebook)?;
            write((first as u64).wrapping_sub(1), 5)?;
            let mut this = 0i64;
            let mut i = 1i64;
            while i < n as i64 {
                if lengths[i as usize] > lengths[(i - 1) as usize] {
                    let mut num = lengths[i as usize] - lengths[(i - 1) as usize];
                    while num > 0 {
                        write((i - this) as u64, ilog_bits((n - this as usize) as u64))?;
                        this = i;
                        num -= 1;
                    }
                }
                i += 1;
            }
            write((i - this) as u64, ilog_bits((n - this as usize) as u64))?;
        } else {
            write(0, 1)?;
            write(nob as u64, 3)?;
            let mut first_unused = 0usize;
            while first_unused < n && lengths[first_unused] != 0 {
                first_unused += 1;
            }
            if first_unused == n {
                write(0, 1)?;
                for &l in lengths {
                    write((l as u64).wrapping_sub(1), nob)?;
                }
            } else {
                write(1, 1)?;
                for &l in lengths {
                    if l != 0 {
                        write(1, 1)?;
                        write((l as u64).wrapping_sub(1), nob)?;
                    } else {
                        write(0, 1)?;
                    }
                }
            }
        }

        // Wwise stores maptype as one bit: zero or non-zero.
        write(if self.maptype != 0 { 1 } else { 0 }, 1)?;
        if self.maptype == 0 {
            return Ok(op.into_buffer());
        }

        write((self.q_min as u64) & 0xFFFF_FFFF, 32)?;
        write((self.q_delta as u64) & 0xFFFF_FFFF, 32)?;
        write((self.q_quant as u64).wrapping_sub(1) & 0xF, 4)?;
        write(self.q_sequencep as u64 & 1, 1)?;
        if let Some(quantlist) = &self.quantlist {
            for &v in quantlist {
                let av = v.unsigned_abs();
                let bits = self.q_quant as u32;
                let mask = if bits < 32 {
                    (1u64 << bits) - 1
                } else {
                    0xFFFF_FFFF
                };
                write(av & mask, bits)?;
            }
        }
        Ok(op.into_buffer())
    }
}

/// Vorbis non-IEEE float32_unpack (spec / libvorbis sharedbook).
pub fn float32_unpack(val: u32) -> f64 {
    let mant = val & 0x1FFFFF;
    let sign = val & 0x8000_0000 != 0;
    let exp = (val & 0x7FE0_0000) >> 21;
    // Python: `mant = -mant` when signed, then `math.ldexp(float(mant), exp - 20 - 768)`.
    let mant_value = if sign {
        -(mant as i64) as f64
    } else {
        mant as f64
    };
    mant_value * 2f64.powi(exp as i32 - 20 - 768)
}

/// Largest r such that r**dim <= entries (libvorbis `_book_maptype1_quantvals`).
pub fn book_maptype1_quantvals(entries: i64, dim: i64) -> Result<i64, CodebookError> {
    if dim < 1 {
        return Err(CodebookError::DimTooSmall { dim });
    }
    if entries < 1 {
        return Ok(0);
    }
    let mut vals = (entries as f64).powf(1.0 / dim as f64).floor() as i64;
    loop {
        let mut acc = 1i128;
        let mut acc1 = 1i128;
        for _ in 0..dim {
            acc *= vals as i128;
            acc1 *= (vals + 1) as i128;
        }
        if acc <= entries as i128 && acc1 > entries as i128 {
            return Ok(vals);
        }
        if acc > entries as i128 {
            vals -= 1;
            if vals < 0 {
                return Ok(0);
            }
        } else {
            vals += 1;
        }
    }
}

/// Assign Huffman codewords from lengths (libvorbis `_make_words`,
/// sparsecount=0). Returns one codeword per entry (0 if unused). Words are
/// bit-reversed for LSB-first pack/unpack.
pub fn make_codewords(lengthlist: &[i64]) -> Result<Vec<i64>, CodebookError> {
    let mut marker = [0i64; 33];
    let mut r = vec![0i64; lengthlist.len()];
    let mut count = 0usize;
    for (i, &length) in lengthlist.iter().enumerate() {
        if length > 0 {
            if length > 32 {
                return Err(CodebookError::LengthOutOfRange {
                    entry: i as i64,
                    length,
                });
            }
            let mut entry = marker[length as usize];
            if length < 32 && (entry >> length) != 0 {
                return Err(CodebookError::OverpopulatedTree {
                    entry: i as i64,
                    length,
                });
            }
            r[count] = entry;
            count += 1;

            for j in (1..=length).rev() {
                if marker[j as usize] & 1 != 0 {
                    if j == 1 {
                        marker[1] += 1;
                    } else {
                        marker[j as usize] = marker[j as usize - 1] << 1;
                    }
                    break;
                }
                marker[j as usize] += 1;
            }

            for j in (length as usize + 1)..33 {
                if (marker[j] >> 1) == entry {
                    entry = marker[j];
                    marker[j] = marker[j - 1] << 1;
                } else {
                    break;
                }
            }
        } else {
            // unused: still advance dense slot so second pass indexes align
            count += 1;
        }
    }

    // Bit-reverse into dense slots (all entries when sparsecount=0)
    let mut out = vec![0i64; lengthlist.len()];
    count = 0;
    for &value in lengthlist {
        let length = value as usize;
        let mut temp = 0i64;
        for j in 0..length {
            temp <<= 1;
            temp |= (r[count] >> j) & 1;
        }
        out[count] = temp;
        count += 1;
    }
    Ok(out)
}

/// Decode tree node (Python's nested dict with int leaves).
#[derive(Debug, Clone)]
pub(crate) enum Tree {
    /// Internal node: children keyed by bit.
    Node(HashMap<u8, Tree>),
    /// Leaf: entry index.
    Leaf(i64),
}

/// Binary tree for the LSB-first bit walk (Python `_build_decode_tree`).
fn build_decode_tree(codelist: &[i64], lengthlist: &[i64]) -> Result<Tree, CodebookError> {
    let mut root = Tree::Node(HashMap::new());
    for (entry, (code, length)) in codelist.iter().zip(lengthlist.iter()).enumerate() {
        let length = *length;
        if length <= 0 {
            continue;
        }
        descend(&mut root, 0, length, *code, entry as i64)?;
    }
    Ok(root)
}

/// Recursively insert one codeword, mirroring the Python walk exactly.
///
/// `node` is the internal map we currently stand on; we process bit `bit_idx`
/// (0-based, LSB-first). The leaf lands at bit `length - 1`.
fn descend(
    node: &mut Tree,
    bit_idx: i64,
    length: i64,
    code: i64,
    entry: i64,
) -> Result<(), CodebookError> {
    match node {
        Tree::Leaf(prev) => Err(CodebookError::CodeEmbedsLeaf { leaf: *prev, entry }),
        Tree::Node(map) => {
            let bit = ((code >> bit_idx) & 1) as u8;
            if bit_idx + 1 == length {
                // Last bit: internal node where a leaf is required is a collision.
                if let Some(Tree::Node(_)) = map.get(&bit) {
                    return Err(CodebookError::CodePrefixCollision { entry });
                }
                map.insert(bit, Tree::Leaf(entry));
                Ok(())
            } else {
                match map.get_mut(&bit) {
                    None => {
                        map.insert(bit, Tree::Node(HashMap::new()));
                        match map.get_mut(&bit) {
                            Some(child) => descend(child, bit_idx + 1, length, code, entry),
                            None => unreachable!("just inserted a child node"),
                        }
                    }
                    Some(child) => match child {
                        Tree::Leaf(prev) => {
                            Err(CodebookError::CodeEmbedsLeaf { leaf: *prev, entry })
                        }
                        Tree::Node(_) => descend(child, bit_idx + 1, length, code, entry),
                    },
                }
            }
        }
    }
}

/// libvorbis `_book_unquantize` for maptype 1 (full dense valuallist).
/// Unused entries (length 0) get `None`. Sequential adds last to each step.
/// `value = |quantlist[index]| * delta + mindel [+ last]`
fn book_unquantize_maptype1(
    entries: i64,
    dim: i64,
    quantlist: &[i64],
    q_min: u32,
    q_delta: u32,
    q_sequencep: i64,
    lengthlist: &[i64],
) -> Result<Vec<Option<Vec<f64>>>, CodebookError> {
    let quantvals = book_maptype1_quantvals(entries, dim)?;
    if quantvals <= 0 {
        return Err(CodebookError::QuantvalsNonPositive);
    }
    if quantlist.len() < quantvals as usize {
        return Err(CodebookError::QuantlistTooShort {
            got: quantlist.len(),
            want: quantvals,
        });
    }
    let mindel = float32_unpack(q_min);
    let delta = float32_unpack(q_delta);
    let mut out = vec![None; entries as usize];
    for (j, slot) in out.iter_mut().enumerate() {
        let length = *lengthlist.get(j).ok_or(CodebookError::LengthlistMismatch {
            got: lengthlist.len(),
            want: entries,
        })?;
        if length <= 0 {
            continue;
        }
        let mut last = 0.0f64;
        let mut vals = Vec::with_capacity(dim as usize);
        let mut indexdiv = 1i64;
        for _k in 0..dim {
            let index = (j as i64 / indexdiv) % quantvals;
            let q = quantlist[index as usize];
            let val = (q.abs() as f64) * delta + mindel + last;
            if q_sequencep != 0 {
                last = val;
            }
            vals.push(val);
            indexdiv *= quantvals;
        }
        *slot = Some(vals);
    }
    Ok(out)
}

/// Runtime Huffman + optional VQ lattice codebook (Python `Codebook`).
#[derive(Debug, Clone)]
pub struct Codebook {
    pub static_codebook: StaticCodebook,
    pub book_id: Option<i64>,
    pub table: Option<String>,
    pub index: Option<i64>,
    /// Huffman codeword per entry (bit-reversed, LSB-first).
    pub codelist: Vec<i64>,
    tree: Tree,
    /// maptype1: per-entry VQ vector, or None when unused.
    pub valuallist: Option<Vec<Option<Vec<f64>>>>,
    pub quantvals: i64,
    /// Assembly-time precompute of the used entries (length > 0), ascending;
    /// the `nearest_used_entry` scan reads it without rebuilding it on every
    /// hot-path call.
    used_entries_cache: Vec<i64>,
    /// Per-entry membership flag (length > 0) for O(1) domain checks in the
    /// floor1/residue packers; same domain as the Python length-list test.
    used_flags: Vec<bool>,
    /// Flat maptype1 VQ cache: entry ids followed by their concatenated
    /// vectors, in the same order as the previous per-entry allocations.
    vq_flat_entries: Vec<i64>,
    vq_flat: Vec<f64>,
}

impl Codebook {
    /// Build a runtime Codebook (Huffman tree + optional VQ) from static
    /// fields (Python `codebook_from_static`).
    pub fn from_static(
        sc: StaticCodebook,
        book_id: Option<i64>,
        table: Option<String>,
        index: Option<i64>,
    ) -> Result<Self, CodebookError> {
        if sc.dim < 1 {
            return Err(CodebookError::DimTooSmall { dim: sc.dim });
        }
        let codelist = make_codewords(&sc.lengthlist)?;
        let tree = build_decode_tree(&codelist, &sc.lengthlist)?;
        let mut valuallist = None;
        let mut quantvals = 0i64;
        if sc.maptype == 1 {
            let quantlist = sc
                .quantlist
                .clone()
                .filter(|q| !q.is_empty())
                .ok_or(CodebookError::Maptype1RequiresQuantlist)?;
            quantvals = book_maptype1_quantvals(sc.entries, sc.dim)?;
            valuallist = Some(book_unquantize_maptype1(
                sc.entries,
                sc.dim,
                &quantlist,
                (sc.q_min as u64) as u32,
                (sc.q_delta as u64) as u32,
                sc.q_sequencep,
                &sc.lengthlist,
            )?);
        }
        let valuallist = valuallist.clone();
        let vq_cache = build_vq_cache(&valuallist, &sc.lengthlist)?;
        let vq_dim = sc.dim as usize;
        let mut vq_flat_entries = Vec::with_capacity(vq_cache.len());
        let mut vq_flat = Vec::with_capacity(vq_cache.len() * vq_dim);
        for (e, vec) in &vq_cache {
            vq_flat_entries.push(*e);
            vq_flat.extend_from_slice(vec);
        }
        drop(vq_cache);
        let used_entries_cache = sc
            .lengthlist
            .iter()
            .enumerate()
            .filter_map(|(i, &l)| if l > 0 { Some(i as i64) } else { None })
            .collect();
        let used_flags = sc.lengthlist.iter().map(|&l| l > 0).collect();
        Ok(Self {
            static_codebook: sc,
            book_id,
            table,
            index,
            codelist,
            tree,
            valuallist,
            quantvals,
            used_entries_cache,
            used_flags,
            vq_flat_entries,
            vq_flat,
        })
    }

    pub fn dim(&self) -> i64 {
        self.static_codebook.dim
    }

    pub fn entries(&self) -> i64 {
        self.static_codebook.entries
    }

    pub fn maptype(&self) -> i64 {
        self.static_codebook.maptype
    }

    pub fn lengthlist(&self) -> &[i64] {
        &self.static_codebook.lengthlist
    }

    /// True when entry `e` has a non-zero length (hot-path domain test).
    pub fn entry_is_used(&self, e: i64) -> bool {
        e >= 0 && (e as usize) < self.used_flags.len() && self.used_flags[e as usize]
    }

    /// True when some used entry is within distance 2 of `y`
    /// (the Python `(e - y).abs() <= 2` scan, precomputed membership).
    pub fn has_used_within_two(&self, y: i64) -> bool {
        for offset in (y - 2)..=y + 2 {
            if offset >= 0
                && (offset as usize) < self.used_flags.len()
                && self.used_flags[offset as usize]
            {
                return true;
            }
        }
        false
    }

    /// Nearest used entry (first on ties), or `None` when no entry is used.
    /// Mirrors the Python `_nearest_used_entry` scan over ascending used
    /// entries without rebuilding the list per call.
    pub fn nearest_used_entry(&self, value: i64) -> Option<i64> {
        let used = &self.used_entries_cache;
        if used.is_empty() {
            return None;
        }
        if self.entry_is_used(value) {
            return Some(value);
        }
        let mut best = used[0];
        for &e in used {
            if (e - value).abs() < (best - value).abs() {
                best = e;
            }
        }
        Some(best)
    }

    /// Borrowed VQ vector for a used maptype1 entry (no allocation).
    fn vq_slice(&self, entry: i64) -> Option<&[f64]> {
        self.valuallist
            .as_ref()?
            .get(usize::try_from(entry).ok()?)?
            .as_deref()
    }

    /// Write the Huffman code for entry index (maptype 0 or 1 index).
    pub fn encode(&self, op: &mut OggPack, entry: i64) -> Result<(), CodebookError> {
        // The codeword and length arrays are `lengthlist.len()` long, so an
        // entry is only in range when it is inside both `entries` and them
        // (a hand-built book whose `entries` exceeds the arrays is reported
        // rather than read past).
        let entries = self.entries().min(self.lengthlist().len() as i64);
        if entry < 0 || entry >= entries {
            return Err(CodebookError::EntryOutOfRange { entry, entries });
        }
        let length = self.lengthlist()[entry as usize];
        if length <= 0 {
            return Err(CodebookError::EntryUnused { entry });
        }
        op.write(self.codelist[entry as usize] as u64, length as u32)
            .map_err(|_| CodebookError::EntryOutOfRange {
                entry,
                entries: self.entries(),
            })
    }

    /// Decode one Huffman entry index from the bit stream.
    pub fn decode(&self, br: &mut BitReader) -> Result<i64, CodebookError> {
        let mut node = &self.tree;
        match node {
            Tree::Node(map) if map.is_empty() => return Err(CodebookError::EmptyCodebook),
            _ => {}
        }
        loop {
            let bit = br.read(1).map_err(|_| CodebookError::InvalidHuffmanCode)? as u8;
            node = match node {
                Tree::Leaf(_) => unreachable!(),
                Tree::Node(map) => match map.get(&bit) {
                    None => return Err(CodebookError::InvalidHuffmanCode),
                    Some(next) => next,
                },
            };
            if let Tree::Leaf(entry) = node {
                return Ok(*entry);
            }
        }
    }

    /// Decode an entry and return its VQ vector (maptype 1).
    pub fn decode_vq(&self, br: &mut BitReader) -> Result<Vec<f64>, CodebookError> {
        if self.maptype() != 1 {
            return Err(CodebookError::NotMaptype1 {
                maptype: self.maptype(),
            });
        }
        let entry = self.decode(br)?;
        let vec = self.vq_values(entry)?;
        Ok(vec)
    }

    /// Borrowed VQ vector accessor used by the residue packer (maptype 1).
    pub(crate) fn borrow_vq(&self, entry: i64) -> Result<&[f64], CodebookError> {
        if self.maptype() != 1 || self.valuallist.is_none() {
            return Err(CodebookError::VqMisuse {
                reason: "vq_values requires maptype 1 with valuallist",
            });
        }
        self.vq_slice(entry)
            .ok_or(CodebookError::NoVqVector { entry })
    }

    /// Return the unquantized VQ vector for an entry (maptype 1).
    pub fn vq_values(&self, entry: i64) -> Result<Vec<f64>, CodebookError> {
        if self.maptype() != 1 || self.valuallist.is_none() {
            return Err(CodebookError::VqMisuse {
                reason: "vq_values requires maptype 1 with valuallist",
            });
        }
        let vec = self
            .vq_slice(entry)
            .ok_or(CodebookError::NoVqVector { entry })?;
        Ok(vec.to_vec())
    }

    /// Nearest used maptype1 entry (squared Euclidean distance; first entry
    /// on ties, matching the Python scalar loop and `np.argmin`).
    pub fn best_vq(&self, target: &[f64]) -> Result<i64, CodebookError> {
        if self.maptype() != 1 || self.valuallist.is_none() {
            return Err(CodebookError::VqMisuse {
                reason: "best_vq requires maptype 1 with valuallist",
            });
        }
        let dim = self.dim() as usize;
        if target.len() < dim {
            return Err(CodebookError::TargetTooShort {
                got: target.len(),
                dim: self.dim(),
            });
        }
        let cache_dim = self.dim() as usize;
        let mut best_e = -1i64;
        let mut best_err = f64::INFINITY;
        for (k, &e) in self.vq_flat_entries.iter().enumerate() {
            let base = k * cache_dim;
            let mut err = 0.0f64;
            for (t, v) in target
                .iter()
                .zip(self.vq_flat[base..base + cache_dim].iter())
            {
                let d = t - v;
                err += d * d;
            }
            if err < best_err {
                best_err = err;
                best_e = e;
                if err == 0.0 {
                    break;
                }
            }
        }
        if best_e < 0 {
            return Err(CodebookError::NoUsableVqEntries);
        }
        Ok(best_e)
    }
}

/// Build the (entry, vector) cache for used maptype1 entries.
///
/// A used entry past the end of `valuallist` (a lengthlist longer than
/// `entries`) is reported rather than indexed.
fn build_vq_cache(
    valuallist: &Option<Vec<Option<Vec<f64>>>>,
    lengthlist: &[i64],
) -> Result<Vec<(i64, Vec<f64>)>, CodebookError> {
    let mut cache = Vec::new();
    if let Some(valuallist) = valuallist {
        for (e, &l) in lengthlist.iter().enumerate() {
            if l <= 0 {
                continue;
            }
            let entry = valuallist.get(e).ok_or(CodebookError::LengthlistMismatch {
                got: lengthlist.len(),
                want: valuallist.len() as i64,
            })?;
            if let Some(vec) = entry {
                cache.push((e as i64, vec.clone()));
            }
        }
    }
    Ok(cache)
}

/// One decoded book descriptor from the profile tables (Python raw row dict).
///
/// JSON rows carry `i`, `dim`, `entries`, `lengthlist`, `maptype`, `q_min`,
/// `q_delta`, `q_quant`, `q_sequencep` and (t219 only) `quantlist`,
/// `quantvals`. `lengthlist`/`quantlist`/`quantvals` may be absent.
#[derive(Debug, Clone, PartialEq)]
pub struct CodebookRow {
    /// Row index within its table (the JSON `i` field).
    pub i: Option<i64>,
    pub dim: i64,
    pub entries: i64,
    pub lengthlist: Option<Vec<i64>>,
    /// Python: `int(row.get("maptype") or 0)` — falsy entries become 0.
    pub maptype: i64,
    /// Python: `int(row.get("q_min") or 0)` — falsy entries become 0.
    pub q_min: i64,
    /// Python: `int(row.get("q_delta") or 0)` — falsy entries become 0.
    pub q_delta: i64,
    /// Python: `int(row.get("q_quant") or 0)` — falsy entries become 0.
    pub q_quant: i64,
    /// Python: `int(row.get("q_sequencep") or 0)` — falsy entries become 0.
    pub q_sequencep: i64,
    pub quantlist: Option<Vec<i64>>,
    /// Informational only (the runtime derives quantvals itself).
    pub quantvals: Option<i64>,
}

impl CodebookRow {
    /// Build a `StaticCodebook` from this row (Python `static_from_entry_dict`).
    pub fn to_static(&self) -> Result<StaticCodebook, CodebookError> {
        let lengthlist = self.lengthlist.clone().unwrap_or_default();
        if lengthlist.len() as i64 != self.entries {
            return Err(CodebookError::LengthlistMismatch {
                got: lengthlist.len(),
                want: self.entries,
            });
        }
        Ok(StaticCodebook {
            dim: self.dim,
            entries: self.entries,
            lengthlist,
            maptype: self.maptype,
            q_min: self.q_min,
            q_delta: self.q_delta,
            q_quant: self.q_quant,
            q_sequencep: self.q_sequencep,
            quantlist: self.quantlist.clone(),
        })
    }
}
