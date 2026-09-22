//! Pure Wwise Vorbis setup packet syntax codec
//! (Python: `wwise_wem/vorbis/setup.py`).
//!
//! `parse_setup` decodes one setup packet into typed structs; the `pack_*`
//! functions serialize them back to the identical byte stream.

use std::fmt;

use crate::bitio::{BitError, BitReader, OggPack};

/// Bits needed to store values in `[0, n]` inclusive when n >= 0
/// (Vorbis-style; Python `setup.ilog`): `ilog(0)=0, ilog(1)=1, ilog(2)=2`.
#[inline]
pub fn ilog(n: u64) -> u32 {
    let mut r = 0u32;
    let mut v = n;
    while v > 0 {
        r += 1;
        v >>= 1;
    }
    r
}

/// Floor-1 setup (Python `parse_floor1` dict).
#[derive(Debug, Clone, PartialEq)]
pub struct Floor1Setup {
    pub partitions: u64,
    pub partition_classes: Vec<u64>,
    pub max_class: i64,
    pub class_dims: Vec<u64>,
    pub class_subs: Vec<u64>,
    pub class_masterbooks: Vec<Option<u64>>,
    pub subclass_books: Vec<Vec<i64>>,
    pub multiplier: u64,
    pub rangebits: u64,
    pub x_list: Vec<u64>,
    pub bit_start: u64,
    pub bit_end: u64,
}

/// Residue setup (Python `parse_residue` dict).
#[derive(Debug, Clone, PartialEq)]
pub struct ResidueSetup {
    pub residue_type: u64,
    pub begin: u64,
    pub end: u64,
    pub partition_size: u64,
    pub classifications: u64,
    pub classbook: u64,
    pub cascades: Vec<u64>,
    pub books: Vec<[i64; 8]>,
    pub bit_start: u64,
    pub bit_end: u64,
}

/// One channel coupling step.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct CouplingStep {
    pub mag: u64,
    pub ang: u64,
}

/// Mapping-0 setup (Python `parse_mapping0` dict).
#[derive(Debug, Clone, PartialEq)]
pub struct Mapping0Setup {
    pub submaps: u64,
    pub coupling: Vec<CouplingStep>,
    pub reserved: u64,
    pub chmux: Vec<u64>,
    pub floors: Vec<u64>,
    pub residues: Vec<u64>,
    pub bit_start: u64,
    pub bit_end: u64,
}

/// Mode setup (Python `parse_mode` dict).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct ModeSetup {
    pub blockflag: u64,
    pub mapping: u64,
    pub bit_start: u64,
    pub bit_end: u64,
}

/// Bit positions of the major setup sections.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct BitPositions {
    pub after_books: u64,
    pub after_floor_count: u64,
    pub after_floors: u64,
    pub after_residues: u64,
    pub after_maps: u64,
    pub after_modes: u64,
    pub end: u64,
}

/// Complete parsed setup (Python `parse_setup` dict).
#[derive(Debug, Clone, PartialEq)]
pub struct SetupInfo {
    pub setup_size: usize,
    pub bits_total: u64,
    pub channels: i64,
    pub nbooks: u64,
    pub book_ids: Vec<u64>,
    pub unique_book_ids: Vec<u64>,
    pub nfloors: u64,
    pub floors: Vec<Floor1Setup>,
    pub nresidues: u64,
    pub residues: Vec<ResidueSetup>,
    pub nmaps: u64,
    pub maps: Vec<Mapping0Setup>,
    pub nmodes: u64,
    pub modes: Vec<ModeSetup>,
    pub bit_positions: BitPositions,
    pub trailing_pad_bits: u64,
    pub trailing_pad_value: u64,
    pub parse_complete: bool,
    /// Informational mapping note (Python constant string).
    pub book_id_assignment: &'static str,
}

fn parse_floor1(br: &mut BitReader) -> Result<Floor1Setup, BitError> {
    let start = br.tell_bits();
    let partitions = br.read(5)?;
    let mut partition_classes = Vec::with_capacity(partitions as usize);
    for _ in 0..partitions {
        partition_classes.push(br.read(4)?);
    }
    let max_class = partition_classes
        .iter()
        .max()
        .copied()
        .map(|v| v as i64)
        .unwrap_or(-1);

    let mut class_dims = Vec::new();
    let mut class_subs = Vec::new();
    let mut class_masterbooks = Vec::new();
    let mut subclass_books = Vec::new();

    for _c in 0..max_class + 1 {
        let dim = br.read(3)? + 1;
        let subs = br.read(2)?;
        class_dims.push(dim);
        class_subs.push(subs);
        let master = if subs != 0 { Some(br.read(8)?) } else { None };
        class_masterbooks.push(master);
        let mut sbooks = Vec::with_capacity(1 << subs);
        for _ in 0..(1u64 << subs) {
            sbooks.push(br.read(8)? as i64 - 1);
        }
        subclass_books.push(sbooks);
    }

    let multiplier = br.read(2)? + 1;
    let rangebits = br.read(4)?;
    let n_x: u64 = partition_classes
        .iter()
        .map(|&p| class_dims[p as usize])
        .sum();
    let mut x_list = Vec::with_capacity(n_x as usize);
    for _ in 0..n_x {
        x_list.push(br.read(rangebits as u32)?);
    }

    Ok(Floor1Setup {
        partitions,
        partition_classes,
        max_class,
        class_dims,
        class_subs,
        class_masterbooks,
        subclass_books,
        multiplier,
        rangebits,
        x_list,
        bit_start: start,
        bit_end: br.tell_bits(),
    })
}

fn parse_residue(br: &mut BitReader) -> Result<ResidueSetup, BitError> {
    let start = br.tell_bits();
    let rtype = br.read(2)?;
    let begin = br.read(24)?;
    let end = br.read(24)?;
    let partition_size = br.read(24)? + 1;
    let classifications = br.read(6)? + 1;
    let classbook = br.read(8)?;

    let mut cascades = Vec::with_capacity(classifications as usize);
    for _ in 0..classifications {
        // Writer: if bitlen(cascade) > 3 -> low3 + flag1 + high5; else -> 4-bit cascade.
        let low = br.read(3)?;
        let flag = br.read(1)?;
        let high = if flag != 0 { br.read(5)? } else { 0 };
        cascades.push(high * 8 + low);
    }

    let mut books = Vec::with_capacity(cascades.len());
    for &cascade in &cascades {
        let mut row: [i64; 8] = [-1; 8];
        for (k, cell) in row.iter_mut().enumerate() {
            if cascade & (1 << k) != 0 {
                *cell = br.read(8)? as i64;
            }
        }
        books.push(row);
    }

    Ok(ResidueSetup {
        residue_type: rtype,
        begin,
        end,
        partition_size,
        classifications,
        classbook,
        cascades,
        books,
        bit_start: start,
        bit_end: br.tell_bits(),
    })
}

fn parse_mapping0(br: &mut BitReader, channels: i64) -> Result<Mapping0Setup, BitError> {
    let start = br.tell_bits();
    let submaps = if br.read(1)? != 0 { br.read(4)? + 1 } else { 1 };

    let mut coupling = Vec::new();
    if br.read(1)? != 0 {
        let steps = br.read(8)? + 1;
        let chbits = ilog(channels.saturating_sub(1) as u64);
        for _ in 0..steps {
            coupling.push(CouplingStep {
                mag: br.read(chbits)?,
                ang: br.read(chbits)?,
            });
        }
    }
    let reserved = br.read(2)?;

    let chmux = if submaps > 1 {
        let mut mux = Vec::with_capacity(channels as usize);
        for _ in 0..channels {
            mux.push(br.read(4)?);
        }
        mux
    } else {
        vec![0; channels as usize]
    };

    let mut floors = Vec::with_capacity(submaps as usize);
    let mut residues = Vec::with_capacity(submaps as usize);
    for _ in 0..submaps {
        let _unused = br.read(8)?; // always 0 in pack
        floors.push(br.read(8)?);
        residues.push(br.read(8)?);
    }

    Ok(Mapping0Setup {
        submaps,
        coupling,
        reserved,
        chmux,
        floors,
        residues,
        bit_start: start,
        bit_end: br.tell_bits(),
    })
}

fn parse_mode(br: &mut BitReader) -> Result<ModeSetup, BitError> {
    let start = br.tell_bits();
    let blockflag = br.read(1)?;
    let mapping = br.read(8)?;
    Ok(ModeSetup {
        blockflag,
        mapping,
        bit_start: start,
        bit_end: br.tell_bits(),
    })
}

/// Setup packet packing errors (Python: the `IndexError`/`TypeError` family a
/// malformed setup dict raises, plus the `bitio` write refusals).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SetupError {
    /// A field the writer must read is absent: a list shorter than the count
    /// that indexes it, or a `None` where the bitstream requires a value.
    FieldMissing { field: &'static str },
    /// The underlying bit writer refused the write (Python `ValueError`).
    Bit(BitError),
}

impl fmt::Display for SetupError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            SetupError::FieldMissing { field } => write!(f, "setup field {field} is missing"),
            SetupError::Bit(err) => write!(f, "{err}"),
        }
    }
}

impl std::error::Error for SetupError {}

impl From<BitError> for SetupError {
    fn from(err: BitError) -> Self {
        SetupError::Bit(err)
    }
}

/// Pack one floor-1 setup record (Python `pack_floor1`).
pub fn pack_floor1(op: &mut OggPack, fl: &Floor1Setup) -> Result<(), SetupError> {
    op.write(fl.partitions, 5)?;
    for &c in &fl.partition_classes {
        op.write(c, 4)?;
    }
    for i in 0..fl.max_class + 1 {
        let i = i as usize;
        // The oracle masks every `value - 1` write, so a zero or negative
        // field wraps instead of underflowing.
        let dim = *fl.class_dims.get(i).ok_or(SetupError::FieldMissing {
            field: "class_dims",
        })?;
        op.write(dim.wrapping_sub(1), 3)?;
        let subs = *fl.class_subs.get(i).ok_or(SetupError::FieldMissing {
            field: "class_subs",
        })?;
        op.write(subs, 2)?;
        if subs != 0 {
            let master =
                fl.class_masterbooks
                    .get(i)
                    .copied()
                    .flatten()
                    .ok_or(SetupError::FieldMissing {
                        field: "class_masterbooks",
                    })?;
            op.write(master, 8)?;
        }
        let books = fl.subclass_books.get(i).ok_or(SetupError::FieldMissing {
            field: "subclass_books",
        })?;
        for &b in books {
            op.write((b as u64).wrapping_add(1), 8)?;
        }
    }
    op.write(fl.multiplier.wrapping_sub(1), 2)?;
    op.write(fl.rangebits, 4)?;
    for &x in &fl.x_list {
        op.write(x, fl.rangebits as u32)?;
    }
    Ok(())
}

/// Pack one residue setup record (Python `pack_residue`).
pub fn pack_residue(op: &mut OggPack, rs: &ResidueSetup) -> Result<(), SetupError> {
    op.write(rs.residue_type, 2)?;
    op.write(rs.begin, 24)?;
    op.write(rs.end, 24)?;
    op.write(rs.partition_size.wrapping_sub(1), 24)?;
    op.write(rs.classifications.wrapping_sub(1), 6)?;
    op.write(rs.classbook, 8)?;
    for &cascade in &rs.cascades {
        let mut bitlen = 0u32;
        let mut v = cascade;
        while v > 0 {
            bitlen += 1;
            v >>= 1;
        }
        if bitlen > 3 {
            op.write(cascade & 7, 3)?;
            op.write(1, 1)?;
            op.write(cascade >> 3, 5)?;
        } else {
            op.write(cascade, 4)?;
        }
    }
    for (row, &cascade) in rs.books.iter().zip(rs.cascades.iter()) {
        for (k, &value) in row.iter().enumerate().take(8) {
            if cascade & (1 << k) != 0 {
                op.write(value as u64, 8)?;
            }
        }
    }
    Ok(())
}

/// Pack one mapping-0 setup record (Python `pack_mapping0`).
pub fn pack_mapping0(
    op: &mut OggPack,
    mp: &Mapping0Setup,
    channels: i64,
) -> Result<(), SetupError> {
    if mp.submaps <= 1 {
        op.write(0, 1)?;
    } else {
        op.write(1, 1)?;
        op.write(mp.submaps - 1, 4)?;
    }
    if mp.coupling.is_empty() {
        op.write(0, 1)?;
    } else {
        op.write(1, 1)?;
        op.write(mp.coupling.len() as u64 - 1, 8)?;
        let chbits = ilog(channels.saturating_sub(1) as u64);
        for step in &mp.coupling {
            op.write(step.mag, chbits)?;
            op.write(step.ang, chbits)?;
        }
    }
    op.write(mp.reserved, 2)?;
    if mp.submaps > 1 {
        for &m in &mp.chmux {
            op.write(m, 4)?;
        }
    }
    let submaps =
        usize::try_from(mp.submaps).map_err(|_| SetupError::FieldMissing { field: "floors" })?;
    if mp.floors.len() < submaps {
        return Err(SetupError::FieldMissing { field: "floors" });
    }
    if mp.residues.len() < submaps {
        return Err(SetupError::FieldMissing { field: "residues" });
    }
    for i in 0..submaps {
        op.write(0, 8)?;
        op.write(mp.floors[i], 8)?;
        op.write(mp.residues[i], 8)?;
    }
    Ok(())
}

/// Pack one mode setup record (Python `pack_mode`).
pub fn pack_mode(op: &mut OggPack, md: &ModeSetup) -> Result<(), SetupError> {
    op.write(md.blockflag, 1)?;
    op.write(md.mapping, 8)?;
    Ok(())
}

/// Repack a parse_setup() result to Wwise setup packet bytes.
pub fn pack_setup(info: &SetupInfo) -> Result<Vec<u8>, SetupError> {
    let mut op = OggPack::new(std::cmp::max(256, info.setup_size + 16));
    let channels = info.channels;
    op.write(info.nbooks.wrapping_sub(1), 8)?;
    for &bid in &info.book_ids {
        op.write(bid, 10)?;
    }
    op.write(info.nfloors.wrapping_sub(1), 6)?;
    for fl in &info.floors {
        pack_floor1(&mut op, fl)?;
    }
    op.write(info.nresidues.wrapping_sub(1), 6)?;
    for rs in &info.residues {
        pack_residue(&mut op, rs)?;
    }
    op.write(info.nmaps.wrapping_sub(1), 6)?;
    for mp in &info.maps {
        pack_mapping0(&mut op, mp, channels)?;
    }
    op.write(info.nmodes.wrapping_sub(1), 6)?;
    for md in &info.modes {
        pack_mode(&mut op, md)?;
    }
    // trailing zero bits already in buffer; round up to byte
    Ok(op.into_buffer())
}

/// Parse a Wwise Vorbis setup packet (Python `parse_setup`).
pub fn parse_setup(data: &[u8], channels: i64) -> Result<SetupInfo, BitError> {
    let mut br = BitReader::new(data);
    let bits_total = (data.len() as u64) * 8;

    let nbooks = br.read(8)? + 1;
    let mut book_ids = Vec::with_capacity(nbooks as usize);
    for _ in 0..nbooks {
        book_ids.push(br.read(10)?);
    }
    let pos_after_books = br.tell_bits();

    let nfloors = br.read(6)? + 1;
    let pos_after_floor_count = br.tell_bits();
    let mut floors = Vec::with_capacity(nfloors as usize);
    for _ in 0..nfloors {
        floors.push(parse_floor1(&mut br)?);
    }
    let pos_after_floors = br.tell_bits();

    let nresidues = br.read(6)? + 1;
    let mut residues = Vec::with_capacity(nresidues as usize);
    for _ in 0..nresidues {
        residues.push(parse_residue(&mut br)?);
    }
    let pos_after_residues = br.tell_bits();

    let nmaps = br.read(6)? + 1;
    let mut maps = Vec::with_capacity(nmaps as usize);
    for _ in 0..nmaps {
        maps.push(parse_mapping0(&mut br, channels)?);
    }
    let pos_after_maps = br.tell_bits();

    let nmodes = br.read(6)? + 1;
    let mut modes = Vec::with_capacity(nmodes as usize);
    for _ in 0..nmodes {
        modes.push(parse_mode(&mut br)?);
    }
    let pos_after_modes = br.tell_bits();

    let pad_bits = br.bits_left();
    let pad_value = if pad_bits > 0 {
        br.read(pad_bits as u32)?
    } else {
        0
    };

    let mut unique_book_ids = book_ids.clone();
    unique_book_ids.sort_unstable();
    unique_book_ids.dedup();

    Ok(SetupInfo {
        setup_size: data.len(),
        bits_total,
        channels,
        nbooks,
        book_ids,
        unique_book_ids,
        nfloors,
        floors,
        nresidues,
        residues,
        nmaps,
        maps,
        nmodes,
        modes,
        bit_positions: BitPositions {
            after_books: pos_after_books,
            after_floor_count: pos_after_floor_count,
            after_floors: pos_after_floors,
            after_residues: pos_after_residues,
            after_maps: pos_after_maps,
            after_modes: pos_after_modes,
            end: br.tell_bits(),
        },
        trailing_pad_bits: pad_bits,
        trailing_pad_value: pad_value,
        parse_complete: br.tell_bits() == bits_total && pad_value == 0,
        book_id_assignment: "t97:0-96, t219:97-315, t282:316-597",
    })
}
