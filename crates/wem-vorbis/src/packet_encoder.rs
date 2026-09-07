//! Encode Wwise Vorbis audio packets: header, floor, and residue
//! (Python: `wwise_wem/vorbis/packet_encoder.py`).
//!
//! This Wwise build's audio header contains its mode bits only: unlike raw
//! Vorbis-I long packets, it does not serialize previous/next-window flags.
//! Floor1 body matches Vorbis I.
//!
//! The `pack_analysis_frame` façade (floor fit + packet assembly for one
//! analysis frame) belongs to the orchestration layer (`wem-core`), not this
//! crate; [`pack_block_packet_details`] is the top-level primitive here.

use crate::bitio::OggPack;
use crate::codebook::Codebook;
use crate::floor::{
    floor1_curve_from_posts, floor1_wrap_with_posts, postlist_from_floor, Floor1Error,
    FLOOR1_RANGES,
};
use crate::floor_fit::{floor1_quantize_posts, FloorFitError};
use crate::residue::{
    mdct_to_residue, pack_residue_silent, pack_residue_vq, quantize_residue_value, ResidueError,
};
use crate::setup::{ilog, SetupInfo};

/// Audio packet assembly errors (Python: `ValueError` family).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PacketError {
    /// `mode` index out of range.
    ModeOutOfRange { mode: u32, modes: u64 },
    /// `mapping` index out of range.
    MappingOutOfRange { mapping: u64, maps: u64 },
    /// floor index out of range.
    FloorIndexOutOfRange { index: u64 },
    /// residue index out of range.
    ResidueIndexOutOfRange { index: u64 },
    /// subclass book id not present in `books`.
    SubclassBookIndexOutOfRange { book_id: i64 },
    /// master book id not present in `books`.
    MasterBookIndexOutOfRange { book_id: u64 },
    /// master book id missing where required.
    MasterBookMissing { class: u64 },
    /// Y length disagrees with `2 + len(x_list)`.
    BadYLength { got: usize, want: usize },
    /// mdct rows shorter than the spectrum.
    MdctTooShort { channels: usize, n_spectrum: usize },
    /// mdct row count differs from channel count.
    MdctRowCount { got: usize, want: usize },
    /// codebook use failed.
    Codebook(crate::codebook::CodebookError),
    /// floor1 algorithm failed.
    Floor1(Floor1Error),
    /// floor fit failed.
    FloorFit(FloorFitError),
    /// residue packing failed.
    Residue(ResidueError),
}

impl std::fmt::Display for PacketError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            PacketError::ModeOutOfRange { mode, modes } => {
                write!(f, "mode {mode} out of range 0..{modes}")
            }
            PacketError::MappingOutOfRange { mapping, maps } => {
                write!(f, "mapping {mapping} out of range 0..{maps}")
            }
            PacketError::FloorIndexOutOfRange { index } => {
                write!(f, "floor index {index} out of range")
            }
            PacketError::ResidueIndexOutOfRange { index } => {
                write!(f, "residue index {index} out of range")
            }
            PacketError::SubclassBookIndexOutOfRange { book_id } => {
                write!(f, "subclass book id {book_id} out of range")
            }
            PacketError::MasterBookIndexOutOfRange { book_id } => {
                write!(f, "master book id {book_id} out of range")
            }
            PacketError::MasterBookMissing { class } => {
                write!(f, "class {class} requires a master book")
            }
            PacketError::BadYLength { got, want } => {
                write!(f, "Y len {got} != {want}")
            }
            PacketError::MdctTooShort { channels, n_spectrum } => {
                write!(f, "mdct row {channels} shorter than {n_spectrum}")
            }
            PacketError::MdctRowCount { got, want } => {
                write!(f, "mdct rows {got} != channels {want}")
            }
            PacketError::Codebook(e) => write!(f, "codebook: {e}"),
            PacketError::Floor1(e) => write!(f, "floor1: {e}"),
            PacketError::FloorFit(e) => write!(f, "floor fit: {e}"),
            PacketError::Residue(e) => write!(f, "residue: {e}"),
        }
    }
}

impl std::error::Error for PacketError {}

/// Map residual value to a used codebook entry (dim-1 maptype0:
/// entry≈value) (Python `_nearest_used_entry`).
fn nearest_used_entry(book: &Codebook, value: i64) -> Result<i64, PacketError> {
    let used = book.used_entries();
    if used.is_empty() {
        return Err(PacketError::SubclassBookIndexOutOfRange {
            book_id: value,
        });
    }
    if used.contains(&value)
        || (value >= 0 && value < book.entries() && book.lengthlist()[value as usize] > 0)
    {
        return Ok(value);
    }
    // nearest used (first on ties, like Python's min over ascending entries)
    let mut best = used[0];
    for &e in &used {
        if (e - value).abs() < (best - value).abs() {
            best = e;
        }
    }
    Ok(best)
}

/// Choose masterbook cval so each dim's subclass book can represent Y
/// (Python `_pick_subclass_cval`).
fn pick_subclass_cval(
    floor: &crate::setup::Floor1Setup,
    cl: u64,
    y_slice: &[i64],
    books: &[Codebook],
) -> Result<u64, PacketError> {
    let cbits = floor.class_subs[cl as usize];
    let cdim = floor.class_dims[cl as usize];
    let csub = (1u64 << cbits) - 1;
    let sbooks = &floor.subclass_books[cl as usize];
    if cbits == 0 {
        return Ok(0);
    }
    let mut best = 0u64;
    let mut best_score = -1i64;
    let mut limit = 1u64 << (cbits * cdim);
    // cap search for large dims
    if limit > 4096 {
        limit = 4096;
    }
    for cval in 0..limit {
        let mut t = cval;
        let mut score = 0i64;
        let mut ok = true;
        for j in 0..cdim {
            let book_id = sbooks[(t & csub) as usize];
            t >>= cbits;
            let y = y_slice[j as usize];
            if book_id < 0 {
                if y != 0 {
                    ok = false;
                    break;
                }
                score += 1;
            } else {
                let b = &books[book_id as usize];
                if y >= 0
                    && y < b.entries()
                    && b.lengthlist()[y as usize] > 0
                {
                    score += 2;
                } else if b
                    .used_entries()
                    .iter()
                    .any(|&e| (e - y).abs() <= 2)
                {
                    score += 1;
                } else {
                    score += 0;
                }
            }
        }
        if ok && score > best_score {
            best_score = score;
            best = cval;
            if score == 2 * cdim as i64 {
                break;
            }
        }
    }
    Ok(best)
}

/// Write the Wwise audio header (Python `pack_audio_header`).
///
/// `prev_window` / `next_window` are not packet fields in this format
/// revision, which writes only the mode value.
pub fn pack_audio_header(op: &mut OggPack, setup: &SetupInfo, mode: u32) -> Result<(), PacketError> {
    let nmodes = setup.nmodes;
    let mode_bits = if nmodes > 1 { ilog(nmodes - 1) } else { 0 };
    if mode_bits != 0 {
        op.write(mode as u64, mode_bits)
            .map_err(|_| PacketError::ModeOutOfRange {
                mode,
                modes: nmodes,
            })?;
    }
    Ok(())
}

/// Pack floor1 packet Y posts (nonzero flag already written as 1)
/// (Python `pack_floor1_body`).
///
/// `Y` must be *wrapped residuals* as produced by `floor1_wrap`, not
/// absolute post heights. Endpoints Y[0]/Y[1] are absolute quant values;
/// interior Y[i] are prediction residuals (0 ⇒ "use predicted").
#[allow(non_snake_case)]
pub fn pack_floor1_body(
    op: &mut OggPack,
    setup: &SetupInfo,
    floor_index: u64,
    books: &[Codebook],
    Y: &[i64],
) -> Result<(), PacketError> {
    let floor = setup
        .floors
        .get(floor_index as usize)
        .ok_or(PacketError::FloorIndexOutOfRange {
            index: floor_index,
        })?;
    let rng = FLOOR1_RANGES[floor.multiplier as usize];
    let ybits = ilog(rng - 1);
    let nvals = 2 + floor.x_list.len();
    if Y.len() != nvals {
        return Err(PacketError::BadYLength {
            got: Y.len(),
            want: nvals,
        });
    }
    // endpoints are absolute quant values in [0, range)
    let clamped = |v: i64| -> u64 {
        let c = v.clamp(0, rng as i64 - 1);
        c as u64
    };
    op.write(clamped(Y[0]), ybits).map_err(|_| PacketError::BadYLength {
        got: Y.len(),
        want: nvals,
    })?;
    op.write(clamped(Y[1]), ybits).map_err(|_| PacketError::BadYLength {
        got: Y.len(),
        want: nvals,
    })?;
    let mut ppos = 2usize;
    for &p in &floor.partition_classes {
        let cl = p as usize;
        let cdim = floor.class_dims[cl];
        let cbits = floor.class_subs[cl];
        let csub = (1u64 << cbits) - 1;
        let y_slice: Vec<i64> = (0..cdim).map(|j| Y[ppos + j as usize]).collect();
        let mut cval;
        if cbits != 0 {
            cval = pick_subclass_cval(floor, cl as u64, &y_slice, books)?;
            let mb = floor
                .class_masterbooks[cl]
                .ok_or(PacketError::MasterBookMissing { class: cl as u64 })? as usize;
            let entry = nearest_used_entry(&books[mb], cval as i64)?;
            books[mb]
                .encode(op, entry)
                .map_err(|_| PacketError::MasterBookIndexOutOfRange {
                    book_id: cval,
                })?;
        } else {
            cval = 0;
        }
        for j in 0..cdim {
            let book_id = floor.subclass_books[cl][(cval & csub) as usize];
            cval >>= cbits;
            if book_id >= 0 {
                let entry = nearest_used_entry(&books[book_id as usize], y_slice[j as usize])?;
                books[book_id as usize]
                    .encode(op, entry)
                    .map_err(|_| PacketError::SubclassBookIndexOutOfRange { book_id })?;
            }
            // book_id < 0 ⇒ residual forced 0 (no bits)
            ppos += 1;
        }
    }
    Ok(())
}

/// Silence audio packet: floor nonzero=0 for all channels → no residue
/// (Python `pack_silence_packet`).
pub fn pack_silence_packet(
    setup: &SetupInfo,
    channels: u32,
    mode: u32,
) -> Result<Vec<u8>, PacketError> {
    let mut op = OggPack::new(16);
    pack_audio_header(&mut op, setup, mode)?;
    for _ in 0..channels {
        op.write(0, 1).map_err(|_| PacketError::ModeOutOfRange { mode, modes: setup.nmodes })?;
    }
    Ok(op.get_buffer())
}

/// Pack mode + floor1 bodies; optional silent residue (class0 only)
/// (Python `pack_floor_only_packet`).
///
/// `curves[ch] = None` ⇒ nonzero 0. If `absolute_posts` is false, curves
/// are *wrapped residuals*; if true they are absolute posts and are wrapped
/// before packing.
#[allow(clippy::too_many_arguments)] // signature mirrors Python pack_floor_only_packet
pub fn pack_floor_only_packet(
    setup: &SetupInfo,
    books: &[Codebook],
    channels: u32,
    mode: u32,
    curves: &[Option<Vec<i64>>],
    silent_residue: bool,
    absolute_posts: bool,
    n_spectrum: Option<usize>,
) -> Result<Vec<u8>, PacketError> {
    let mut op = OggPack::new(512);
    pack_audio_header(&mut op, setup, mode)?;
    let md = setup
        .modes
        .get(mode as usize)
        .ok_or(PacketError::ModeOutOfRange { mode, modes: setup.nmodes })?;
    let mapping = setup
        .maps
        .get(md.mapping as usize)
        .ok_or(PacketError::MappingOutOfRange {
            mapping: md.mapping,
            maps: setup.nmaps,
        })?;
    let mut any_nz = false;
    let mut ch_count = 0u32;
    for ch in 0..channels {
        let sub = if mapping.submaps > 1 {
            mapping.chmux[ch as usize]
        } else {
            0
        };
        let floor_index = mapping.floors[sub as usize];
        let y = curves.get(ch as usize).and_then(|row| row.as_ref());
        match y {
            None => {
                op.write(0, 1).map_err(|_| PacketError::ModeOutOfRange { mode, modes: setup.nmodes })?;
            }
            Some(y) => {
                op.write(1, 1).map_err(|_| PacketError::ModeOutOfRange { mode, modes: setup.nmodes })?;
                if absolute_posts {
                    let pl = postlist_from_floor(
                        &setup.floors[floor_index as usize],
                    );
                    let rng = FLOOR1_RANGES[setup.floors[floor_index as usize].multiplier as usize];
                    let wrapped = crate::floor::floor1_wrap(y, &pl, rng as i64)
                        .map_err(PacketError::Floor1)?;
                    pack_floor1_body(&mut op, setup, floor_index, books, &wrapped)?;
                } else {
                    pack_floor1_body(&mut op, setup, floor_index, books, y)?;
                }
                any_nz = true;
                ch_count += 1;
            }
        }
    }
    if any_nz && silent_residue {
        let res_index = mapping.residues[0];
        pack_residue_silent(
            &mut op,
            &setup.residues[res_index as usize],
            books,
            ch_count,
            n_spectrum,
        )
        .map_err(PacketError::Residue)?;
    }
    Ok(op.get_buffer())
}

/// Encoded packet and the exact integer residue rows supplied to VQ
/// (Python `BlockPacketResult`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BlockPacketResult {
    pub packet: Vec<u8>,
    pub quantized_residue: Vec<Vec<i64>>,
}

/// Full short/long audio packet: floor1 + residue VQ (or silent)
/// (Python `pack_block_packet_details`).
///
/// `absolute_posts[ch]`: packet-domain fit output or `None` (floor zero).
/// Set `posts_are_10bit` when these are raw Wwise fit values from
/// `floor1_fit_wwise`; the packet encoder then applies the format's
/// multiplier shift before wrapping. `mdct[ch]`: MDCT spectrum for
/// residual = mdct/floor_amp.
#[allow(clippy::too_many_arguments)] // signature mirrors Python pack_block_packet_details
pub fn pack_block_packet_details(
    setup: &SetupInfo,
    books: &[Codebook],
    channels: u32,
    mode: u32,
    absolute_posts: &[Option<Vec<i64>>],
    mdct: &[Vec<f32>],
    residue_vq: bool,
    posts_are_10bit: bool,
) -> Result<BlockPacketResult, PacketError> {
    let mut op = OggPack::new(8192);
    pack_audio_header(&mut op, setup, mode)?;
    let md = setup
        .modes
        .get(mode as usize)
        .ok_or(PacketError::ModeOutOfRange { mode, modes: setup.nmodes })?;
    let mapping = setup
        .maps
        .get(md.mapping as usize)
        .ok_or(PacketError::MappingOutOfRange {
            mapping: md.mapping,
            maps: setup.nmaps,
        })?;
    let n_spectrum = if !mdct.is_empty() {
        mdct[0].len()
    } else if md.blockflag != 0 {
        1024
    } else {
        128
    };
    if mdct.len() > 1 && mdct.len() != channels as usize {
        return Err(PacketError::MdctRowCount {
            got: mdct.len(),
            want: channels as usize,
        });
    }
    for (ch, row) in mdct.iter().enumerate() {
        if row.len() < n_spectrum {
            return Err(PacketError::MdctTooShort {
                channels: ch,
                n_spectrum,
            });
        }
    }

    let mut ch_used: Vec<bool> = Vec::with_capacity(channels as usize);
    let mut residuals: Vec<Vec<f64>> = Vec::with_capacity(channels as usize);
    for ch in 0..channels {
        let sub = if mapping.submaps > 1 {
            mapping.chmux[ch as usize]
        } else {
            0
        };
        let floor_index = mapping.floors[sub as usize];
        let floor = setup
            .floors
            .get(floor_index as usize)
            .ok_or(PacketError::FloorIndexOutOfRange {
                index: floor_index,
            })?;
        let posts = absolute_posts.get(ch as usize).and_then(|p| p.as_ref());
        match posts {
            None => {
                op.write(0, 1).map_err(|_| PacketError::ModeOutOfRange { mode, modes: setup.nmodes })?;
                ch_used.push(false);
                residuals.push(vec![0.0; n_spectrum]);
            }
            Some(posts) => {
                op.write(1, 1).map_err(|_| PacketError::ModeOutOfRange { mode, modes: setup.nmodes })?;
                let pl = postlist_from_floor(floor);
                let rng = FLOOR1_RANGES[floor.multiplier as usize];
                let packet_posts = if posts_are_10bit {
                    floor1_quantize_posts(posts, floor.multiplier).map_err(PacketError::Floor1)?
                } else {
                    posts.to_vec()
                };
                let (raster_posts, y) =
                    floor1_wrap_with_posts(&packet_posts, &pl, rng as i64)
                        .map_err(PacketError::Floor1)?;
                pack_floor1_body(&mut op, setup, floor_index, books, &y)?;
                ch_used.push(true);
                // amplitude floor curve for residual
                // use unwrapped absolute posts (fit output already absolute)
                let curve = floor1_curve_from_posts(
                    &raster_posts,
                    &pl,
                    n_spectrum,
                    floor.multiplier,
                )
                .map_err(PacketError::Floor1)?;
                residuals.push(mdct_to_residue(&mdct[ch as usize], &curve, 1e-8));
            }
        }
    }

    let mut quantized_residue = vec![vec![0i64; n_spectrum]; channels as usize];
    if ch_used.iter().any(|&u| u) {
        let res_index = mapping.residues[0];
        let res = setup
            .residues
            .get(res_index as usize)
            .ok_or(PacketError::ResidueIndexOutOfRange { index: res_index })?;
        // Materialize the integer residue handoff once. The residue packer
        // accepts numeric rows and its own integer normalization is
        // idempotent, so these exact rows feed both classification and VQ.
        let begin = res.begin as usize;
        let end = (res.end as usize).min(n_spectrum);
        quantized_residue = residuals
            .iter()
            .zip(ch_used.iter())
            .map(|(row, &used)| {
                row.iter()
                    .enumerate()
                    .map(|(index, value)| {
                        if used && begin <= index && index < end {
                            quantize_residue_value(*value)
                        } else {
                            0
                        }
                    })
                    .collect()
            })
            .collect();
        if residue_vq {
            pack_residue_vq(
                &mut op,
                res,
                books,
                &quantized_residue,
                &ch_used,
                Some(n_spectrum),
            )
            .map_err(PacketError::Residue)?;
        } else {
            pack_residue_silent(
                &mut op,
                res,
                books,
                ch_used.iter().filter(|&&u| u).count() as u32,
                Some(n_spectrum),
            )
            .map_err(PacketError::Residue)?;
        }
    }
    Ok(BlockPacketResult {
        packet: op.get_buffer(),
        quantized_residue,
    })
}

/// Encode one audio block and return only its packet bytes
/// (Python `pack_block_packet`).
#[allow(clippy::too_many_arguments)] // signature mirrors Python pack_block_packet
pub fn pack_block_packet(
    setup: &SetupInfo,
    books: &[Codebook],
    channels: u32,
    mode: u32,
    absolute_posts: &[Option<Vec<i64>>],
    mdct: &[Vec<f32>],
    residue_vq: bool,
    posts_are_10bit: bool,
) -> Result<Vec<u8>, PacketError> {
    Ok(pack_block_packet_details(
        setup,
        books,
        channels,
        mode,
        absolute_posts,
        mdct,
        residue_vq,
        posts_are_10bit,
    )?
    .packet)
}
