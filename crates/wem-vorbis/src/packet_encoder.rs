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
    floor1_curve_from_posts, floor1_quant_curve_from_posts, floor1_wrap_with_posts,
    postlist_from_floor, Floor1Error, FLOOR1_FROM_DB_LOOKUP, FLOOR1_RANGES,
};
use crate::floor_fit::{floor1_quantize_posts, FloorFitError};
use crate::residue::{
    mdct_to_residue, pack_residue_silent, pack_residue_type2_quantized, pack_residue_vq,
    quantize_residue_value, F32Sample, ResidueError,
};
use crate::setup::CouplingStep;
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
    /// analysis channel count differs from packet mapping.
    AnalysisChannelsMismatch { want: usize, got: usize },
    /// Stereo coupling peak rows do not match the MDCT geometry.
    CouplingPeakGeometry,
    /// Channel mux row is shorter than the declared channel count.
    ChannelMuxTooShort { got: usize, want: usize },
    /// Floor map is shorter than the declared submap count.
    FloorMapTooShort { got: usize, want: usize },
    /// Mapping declares no residue submap.
    MissingResidueSubmap,
    /// Floor multiplier has no range table entry.
    FloorMultiplierOutOfRange { multiplier: u64 },
    /// Floor class arrays do not contain the referenced class.
    FloorClassOutOfRange { class: usize },
    /// Mapping coupling references a missing channel.
    CouplingChannelOutOfRange { channel: usize, channels: usize },
    /// Coupled residue rows have different lengths.
    CouplingRowLengthMismatch,
    /// A coupling step references the same channel twice.
    CouplingChannelsEqual { channel: usize },
    /// Analysis produced a non-finite sample at the packet boundary.
    NonFiniteAnalysisSample { channel: usize, bin: usize },
    /// Integer-domain stereo coupling exceeded its representable range.
    ResidueCouplingOverflow,
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
            PacketError::MdctTooShort {
                channels,
                n_spectrum,
            } => {
                write!(f, "mdct row {channels} shorter than {n_spectrum}")
            }
            PacketError::MdctRowCount { got, want } => {
                write!(f, "mdct rows {got} != channels {want}")
            }
            PacketError::Codebook(e) => write!(f, "codebook: {e}"),
            PacketError::Floor1(e) => write!(f, "floor1: {e}"),
            PacketError::FloorFit(e) => write!(f, "floor fit: {e}"),
            PacketError::Residue(e) => write!(f, "residue: {e}"),
            PacketError::AnalysisChannelsMismatch { want, got } => {
                write!(
                    f,
                    "analysis channel count differs from packet mapping ({got} != {want})"
                )
            }
            PacketError::CouplingPeakGeometry => {
                write!(f, "stereo coupling peak rows differ from MDCT geometry")
            }
            PacketError::ChannelMuxTooShort { got, want } => {
                write!(f, "channel mux row has {got} entries, expected {want}")
            }
            PacketError::FloorMapTooShort { got, want } => {
                write!(f, "floor map has {got} entries, expected {want}")
            }
            PacketError::MissingResidueSubmap => write!(f, "mapping has no residue submap"),
            PacketError::FloorMultiplierOutOfRange { multiplier } => {
                write!(f, "floor multiplier {multiplier} has no range table")
            }
            PacketError::FloorClassOutOfRange { class } => {
                write!(f, "floor class {class} is structurally incomplete")
            }
            PacketError::CouplingChannelOutOfRange { channel, channels } => {
                write!(f, "coupling channel {channel} out of range 0..{channels}")
            }
            PacketError::CouplingRowLengthMismatch => {
                write!(f, "coupled residue rows have different lengths")
            }
            PacketError::CouplingChannelsEqual { channel } => {
                write!(f, "coupling step references channel {channel} twice")
            }
            PacketError::NonFiniteAnalysisSample { channel, bin } => {
                write!(
                    f,
                    "analysis sample at channel {channel}, bin {bin} is not finite"
                )
            }
            PacketError::ResidueCouplingOverflow => {
                write!(f, "integer stereo coupling exceeded the residue range")
            }
        }
    }
}

impl std::error::Error for PacketError {}

/// Map residual value to a used codebook entry (dim-1 maptype0:
/// entry≈value) (Python `_nearest_used_entry`).
fn nearest_used_entry(book: &Codebook, value: i64) -> Result<i64, PacketError> {
    book.nearest_used_entry(value)
        .ok_or(PacketError::SubclassBookIndexOutOfRange { book_id: value })
}

/// Choose masterbook cval so each dim's subclass book can represent Y
/// (Python `_pick_subclass_cval`).
fn pick_subclass_cval(
    floor: &crate::setup::Floor1Setup,
    cl: u64,
    y_slice: &[i64],
    books: &[Codebook],
) -> Result<u64, PacketError> {
    let class = cl as usize;
    let cbits = *floor
        .class_subs
        .get(class)
        .ok_or(PacketError::FloorClassOutOfRange { class })?;
    let cdim = *floor
        .class_dims
        .get(class)
        .ok_or(PacketError::FloorClassOutOfRange { class })?;
    let csub = 1u64
        .checked_shl(cbits as u32)
        .ok_or(PacketError::FloorClassOutOfRange { class })?
        - 1;
    let sbooks = floor
        .subclass_books
        .get(class)
        .ok_or(PacketError::FloorClassOutOfRange { class })?;
    if cbits == 0 {
        return Ok(0);
    }
    let mut best = 0u64;
    let mut best_score = -1i64;
    let exponent = cbits.saturating_mul(cdim);
    let limit = if exponent >= 12 {
        4096
    } else {
        1u64 << exponent
    };
    for cval in 0..limit {
        let mut t = cval;
        let mut score = 0i64;
        let mut ok = true;
        for j in 0..cdim {
            let book_id = *sbooks
                .get((t & csub) as usize)
                .ok_or(PacketError::FloorClassOutOfRange { class })?;
            t >>= cbits;
            let y = *y_slice.get(j as usize).ok_or(PacketError::BadYLength {
                got: y_slice.len(),
                want: cdim as usize,
            })?;
            if book_id < 0 {
                if y != 0 {
                    ok = false;
                    break;
                }
                score += 1;
            } else {
                let b = books
                    .get(book_id as usize)
                    .ok_or(PacketError::SubclassBookIndexOutOfRange { book_id })?;
                if b.entry_is_used(y) {
                    score += 2;
                } else if b.has_used_within_two(y) {
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
pub fn pack_audio_header(
    op: &mut OggPack,
    setup: &SetupInfo,
    mode: u32,
) -> Result<(), PacketError> {
    let nmodes = setup.nmodes;
    if mode as u64 >= nmodes || setup.modes.get(mode as usize).is_none() {
        return Err(PacketError::ModeOutOfRange {
            mode,
            modes: nmodes,
        });
    }
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
/// absolute post heights. Endpoints `Y[0]`/`Y[1]` are absolute quant values;
/// interior `Y[i]` are prediction residuals (0 ⇒ "use predicted").
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
        .ok_or(PacketError::FloorIndexOutOfRange { index: floor_index })?;
    let rng = *FLOOR1_RANGES.get(floor.multiplier as usize).ok_or(
        PacketError::FloorMultiplierOutOfRange {
            multiplier: floor.multiplier,
        },
    )?;
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
    op.write(clamped(Y[0]), ybits)
        .map_err(|_| PacketError::BadYLength {
            got: Y.len(),
            want: nvals,
        })?;
    op.write(clamped(Y[1]), ybits)
        .map_err(|_| PacketError::BadYLength {
            got: Y.len(),
            want: nvals,
        })?;
    let mut ppos = 2usize;
    for &p in &floor.partition_classes {
        let cl = p as usize;
        let cdim = *floor
            .class_dims
            .get(cl)
            .ok_or(PacketError::FloorClassOutOfRange { class: cl })?;
        let cbits = *floor
            .class_subs
            .get(cl)
            .ok_or(PacketError::FloorClassOutOfRange { class: cl })?;
        let csub = 1u64
            .checked_shl(cbits as u32)
            .ok_or(PacketError::FloorClassOutOfRange { class: cl })?
            - 1;
        let y_slice = Y
            .get(ppos..ppos + cdim as usize)
            .ok_or(PacketError::BadYLength {
                got: Y.len(),
                want: ppos + cdim as usize,
            })?;
        let mut cval;
        if cbits != 0 {
            cval = pick_subclass_cval(floor, cl as u64, y_slice, books)?;
            let mb = floor
                .class_masterbooks
                .get(cl)
                .ok_or(PacketError::FloorClassOutOfRange { class: cl })?
                .ok_or(PacketError::MasterBookMissing { class: cl as u64 })?
                as usize;
            let master = books
                .get(mb)
                .ok_or(PacketError::MasterBookIndexOutOfRange { book_id: mb as u64 })?;
            let entry = nearest_used_entry(master, cval as i64)?;
            master
                .encode(op, entry)
                .map_err(|_| PacketError::MasterBookIndexOutOfRange { book_id: cval })?;
        } else {
            cval = 0;
        }
        for j in 0..cdim {
            let book_id = *floor
                .subclass_books
                .get(cl)
                .and_then(|row| row.get((cval & csub) as usize))
                .ok_or(PacketError::FloorClassOutOfRange { class: cl })?;
            cval >>= cbits;
            if book_id >= 0 {
                let book = books
                    .get(book_id as usize)
                    .ok_or(PacketError::SubclassBookIndexOutOfRange { book_id })?;
                let entry = nearest_used_entry(book, y_slice[j as usize])?;
                book.encode(op, entry)
                    .map_err(|_| PacketError::SubclassBookIndexOutOfRange { book_id })?;
            }
            // book_id < 0 ⇒ residual forced 0 (no bits)
            ppos += 1;
        }
    }
    Ok(())
}

/// In-place forward mapping0 stereo coupling (exact 4-branch pre-image)
/// (Python `apply_mapping_coupling`).
///
/// The published decoder applies the libvorbis mapping0.c four-branch
/// coupling between the residue inverse and floor1 inverse2 (dB residue
/// domain, scripts/decode_wem.py::decouple_db_domain).  For a
/// coupling step (mag, ang) and a pre-coupling pair (M, A), the stored
/// pair is the exact pre-image of that piecewise decode map, so decoding
/// the stored pair recovers (M, A) exactly:
///
/// - M > 0 and A < M:  store (M, M - A)
/// - M > 0 and A >= M: store (A, M - A)
/// - M <= 0 and A > M: store (M, A - M)
/// - M <= 0 and A <= M: store (A, A - M)
fn apply_mapping_coupling(
    residuals: &mut [Vec<f64>],
    coupling: &[CouplingStep],
) -> Result<(), PacketError> {
    for step in coupling {
        let mag = step.mag as usize;
        let ang = step.ang as usize;
        if mag >= residuals.len() {
            return Err(PacketError::CouplingChannelOutOfRange {
                channel: mag,
                channels: residuals.len(),
            });
        }
        if ang >= residuals.len() {
            return Err(PacketError::CouplingChannelOutOfRange {
                channel: ang,
                channels: residuals.len(),
            });
        }
        if mag == ang {
            return Err(PacketError::CouplingChannelsEqual { channel: mag });
        }
        let (mag_row, ang_row) = if mag < ang {
            let (before_ang, from_ang) = residuals.split_at_mut(ang);
            (&mut before_ang[mag], &mut from_ang[0])
        } else {
            let (before_mag, from_mag) = residuals.split_at_mut(mag);
            (&mut from_mag[0], &mut before_mag[ang])
        };
        if mag_row.len() != ang_row.len() {
            return Err(PacketError::CouplingRowLengthMismatch);
        }
        for (magnitude, angle) in mag_row.iter_mut().zip(ang_row.iter_mut()) {
            let m_value = *magnitude;
            let a_value = *angle;
            if m_value > 0.0 {
                if a_value < m_value {
                    *magnitude = m_value;
                    *angle = m_value - a_value;
                } else {
                    *magnitude = a_value;
                    *angle = m_value - a_value;
                }
            } else if a_value > m_value {
                *magnitude = m_value;
                *angle = a_value - m_value;
            } else {
                *magnitude = a_value;
                *angle = a_value - m_value;
            }
        }
    }
    Ok(())
}

/// Propagate floor use across mapping0 coupling pairs before residue coding.
fn propagate_mapping_nonzero(
    ch_used: &mut [bool],
    coupling: &[CouplingStep],
) -> Result<(), PacketError> {
    for step in coupling {
        let mag = step.mag as usize;
        let ang = step.ang as usize;
        if mag >= ch_used.len() {
            return Err(PacketError::CouplingChannelOutOfRange {
                channel: mag,
                channels: ch_used.len(),
            });
        }
        if ang >= ch_used.len() {
            return Err(PacketError::CouplingChannelOutOfRange {
                channel: ang,
                channels: ch_used.len(),
            });
        }
        if ch_used[mag] || ch_used[ang] {
            ch_used[mag] = true;
            ch_used[ang] = true;
        }
    }
    Ok(())
}

fn lossless_couple_f32(first: f32, second: f32) -> (f32, f32) {
    let (mut magnitude, mut angle) = if first.abs() > second.abs() {
        (
            first,
            if first > 0.0 {
                first - second
            } else {
                second - first
            },
        )
    } else {
        (
            second,
            if second > 0.0 {
                first - second
            } else {
                second - first
            },
        )
    };
    if angle >= magnitude.abs() * 2.0 {
        angle = -angle;
        magnitude = -magnitude;
    }
    (magnitude, angle)
}

fn lossless_couple_i64(first: i64, second: i64) -> Result<(i64, i64), PacketError> {
    let (mut magnitude, angle) = if first.unsigned_abs() > second.unsigned_abs() {
        (
            first,
            if first > 0 {
                first.checked_sub(second)
            } else {
                second.checked_sub(first)
            },
        )
    } else {
        (
            second,
            if second > 0 {
                first.checked_sub(second)
            } else {
                second.checked_sub(first)
            },
        )
    };
    let mut angle = angle.ok_or(PacketError::ResidueCouplingOverflow)?;
    let threshold = magnitude
        .checked_abs()
        .and_then(|value| value.checked_mul(2));
    if threshold.is_some_and(|limit| angle >= limit) {
        angle = angle
            .checked_neg()
            .ok_or(PacketError::ResidueCouplingOverflow)?;
        magnitude = magnitude
            .checked_neg()
            .ok_or(PacketError::ResidueCouplingOverflow)?;
    }
    Ok((magnitude, angle))
}

fn point_hypot(first: f32, second: f32, reversal: f32) -> f32 {
    let first_abs = (first * 0.94_f32).abs();
    let second_abs = (second * 0.94_f32).abs();
    if first > 0.0 {
        if second > 0.0 {
            first_abs + second_abs
        } else if first > -second {
            (first_abs as f64 - second_abs as f64 * reversal as f64) as f32
        } else {
            -(second_abs as f64 - first_abs as f64 * reversal as f64) as f32
        }
    } else if second < 0.0 {
        -(first_abs + second_abs)
    } else if -first > second {
        -(first_abs as f64 - second_abs as f64 * reversal as f64) as f32
    } else {
        (second_abs as f64 - first_abs as f64 * reversal as f64) as f32
    }
}

/// aoTuV beta 6.03 joint quantization used by Wwise's stereo profile.
fn aotuv_stereo_residue<S: F32Sample>(
    mdct: &[Vec<S>],
    floor_indices: &[Vec<i64>],
    coupling_peak: &[Vec<S>],
    ch_used: &[bool],
) -> Result<Vec<Vec<i64>>, PacketError> {
    if mdct.len() != 2 || floor_indices.len() != 2 || coupling_peak.len() != 2 || ch_used.len() != 2
    {
        return Err(PacketError::CouplingPeakGeometry);
    }
    let n = mdct[0].len();
    if mdct.iter().any(|row| row.len() < n)
        || floor_indices.iter().any(|row| row.len() < n)
        || coupling_peak.iter().any(|row| row.len() < n)
    {
        return Err(PacketError::CouplingPeakGeometry);
    }
    let partition = if n == 128 { 8 } else { 32 };
    let point_limit = n / 3;
    let lowpass = n * 3 / 4;
    let tonefix_end = n * 35 / 64;
    let mut output = vec![vec![0i64; n]; 2];
    let mut previous_residue_def = -1.0_f32;
    let mut raw = [[0.0_f32; 32]; 2];
    let mut quant = [[0.0_f32; 32]; 2];
    let mut floor_energy = [[0.0_f32; 32]; 2];
    let mut residue = [[0.0_f32; 32]; 2];
    let mut flags = [[0i8; 32]; 2];

    for begin in (0..lowpass).step_by(partition) {
        let count = partition.min(n - begin);
        for channel in 0..2 {
            raw[channel][..count].fill(0.0);
            quant[channel][..count].fill(0.0);
            floor_energy[channel][..count].fill(0.0);
            residue[channel][..count].fill(0.0);
            flags[channel][..count].fill(0);
        }

        for channel in 0..2 {
            if !ch_used[channel] {
                floor_energy[channel].fill(1e-10_f32);
                continue;
            }
            let crossing = point_limit as isize - begin as isize;
            let (mut point, mut rephase_point, interpolate, point_step, rephase_step) =
                if crossing > 0 {
                    let interpolate = crossing <= count as isize;
                    (
                        0.0_f32,
                        0.0_f32,
                        interpolate,
                        if interpolate {
                            2.5_f32 / count as f32
                        } else {
                            0.0
                        },
                        if interpolate {
                            0.5_f32 / count as f32
                        } else {
                            0.0
                        },
                    )
                } else {
                    (2.5_f32, 0.5_f32, false, 0.0, 0.0)
                };
            for local in 0..count {
                let index = begin + local;
                if interpolate {
                    point += point_step;
                    rephase_point += rephase_step;
                }
                let floor_value = FLOOR1_FROM_DB_LOOKUP
                    [floor_indices[channel][index].clamp(0, 255) as usize]
                    as f32;
                let mdct_value = mdct[channel][index].as_f32();
                let peak_value = coupling_peak[channel][index].as_f32();
                let normalized = mdct_value / floor_value;
                let energy = mdct_value * mdct_value;
                if !mdct_value.is_finite()
                    || !peak_value.is_finite()
                    || !normalized.is_finite()
                    || !energy.is_finite()
                {
                    return Err(PacketError::NonFiniteAnalysisSample {
                        channel,
                        bin: index,
                    });
                }
                residue[channel][local] = normalized;
                let threshold = (point - peak_value).max(0.0);
                let magnitude = normalized.abs();
                flags[channel][local] = if magnitude < threshold {
                    if magnitude < rephase_point {
                        0
                    } else {
                        -1
                    }
                } else {
                    1
                };
                quant[channel][local] = energy;
                raw[channel][local] = if mdct_value < 0.0 { -energy } else { energy };
                floor_energy[channel][local] = floor_value * floor_value;
                output[channel][index] = quantize_residue_value(normalized as f64);
            }
        }

        if begin < tonefix_end {
            let mut reversed_phase = 0usize;
            let mut same_phase = 0usize;
            let mut residue_def = 0.0_f32;
            for local in 0..count {
                if residue[0][local] < -0.5
                    || residue[0][local] >= 0.5
                    || residue[1][local] < -0.5
                    || residue[1][local] >= 0.5
                {
                    let opposite = (raw[0][local] > 0.0 && raw[1][local] < 0.0)
                        || (raw[1][local] > 0.0 && raw[0][local] < 0.0);
                    reversed_phase += usize::from(opposite);
                    same_phase += usize::from(!opposite);
                    residue_def += (residue[0][local].abs() - residue[1][local].abs()).abs();
                }
            }
            let active = reversed_phase + same_phase;
            if active != 0 {
                let current_def = residue_def / active as f32;
                residue_def = if previous_residue_def > 0.0 {
                    current_def * 0.5 + previous_residue_def * 0.5
                } else {
                    current_def
                };
                previous_residue_def = current_def;
                if residue_def > 1.0 {
                    let (left, right) = flags.split_at_mut(1);
                    for (left_flag, right_flag) in
                        left[0][..count].iter_mut().zip(&right[0][..count])
                    {
                        if *left_flag == -1 || *right_flag == -1 {
                            *left_flag = 1;
                        }
                    }
                }
                if reversed_phase as f32 / active as f32 >= 0.34_f32 {
                    for local in 0..count {
                        let opposite = (raw[0][local] > 0.0 && raw[1][local] < 0.0)
                            || (raw[1][local] > 0.0 && raw[0][local] < 0.0);
                        if opposite && (flags[0][local] == -1 || flags[1][local] == -1) {
                            flags[0][local] = 1;
                        }
                    }
                }
            } else {
                previous_residue_def = -1.0;
            }
        }

        let mut point_coupled = false;
        for local in 0..count {
            let index = begin + local;
            if flags[0][local] == 1 || flags[1][local] == 1 {
                let coupled_float = lossless_couple_f32(residue[0][local], residue[1][local]);
                if !coupled_float.0.is_finite() || !coupled_float.1.is_finite() {
                    return Err(PacketError::ResidueCouplingOverflow);
                }
                (residue[0][local], residue[1][local]) = coupled_float;
                (output[0][index], output[1][index]) =
                    lossless_couple_i64(output[0][index], output[1][index])?;
                flags[0][local] = 1;
                flags[1][local] = 1;
            } else {
                let reversal = if index < point_limit {
                    0.18_f32
                } else {
                    0.12_f32
                };
                raw[0][local] = point_hypot(raw[0][local], raw[1][local], reversal);
                quant[0][local] = raw[0][local].abs();
                raw[1][local] = 0.0;
                quant[1][local] = 0.0;
                flags[1][local] = 1;
                output[1][index] = 0;
                point_coupled = true;
            }
            let combined_floor = floor_energy[0][local] + floor_energy[1][local];
            floor_energy[0][local] = combined_floor;
            floor_energy[1][local] = combined_floor;
        }

        if point_coupled {
            for local in 0..count {
                if flags[0][local] != 1 {
                    let value =
                        ((quant[0][local] as f64) / (floor_energy[0][local] as f64)).sqrt() as f32;
                    output[0][begin + local] = if raw[0][local] < 0.0 {
                        -quantize_residue_value(value as f64)
                    } else {
                        quantize_residue_value(value as f64)
                    };
                }
            }
        }
    }
    Ok(output)
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
        op.write(0, 1).map_err(|_| PacketError::ModeOutOfRange {
            mode,
            modes: setup.nmodes,
        })?;
    }
    Ok(op.into_buffer())
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
        .ok_or(PacketError::ModeOutOfRange {
            mode,
            modes: setup.nmodes,
        })?;
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
            *mapping
                .chmux
                .get(ch as usize)
                .ok_or(PacketError::ChannelMuxTooShort {
                    got: mapping.chmux.len(),
                    want: channels as usize,
                })?
        } else {
            0
        };
        let floor_index =
            *mapping
                .floors
                .get(sub as usize)
                .ok_or(PacketError::FloorMapTooShort {
                    got: mapping.floors.len(),
                    want: mapping.submaps as usize,
                })?;
        let y = curves.get(ch as usize).and_then(|row| row.as_ref());
        match y {
            None => {
                op.write(0, 1).map_err(|_| PacketError::ModeOutOfRange {
                    mode,
                    modes: setup.nmodes,
                })?;
            }
            Some(y) => {
                op.write(1, 1).map_err(|_| PacketError::ModeOutOfRange {
                    mode,
                    modes: setup.nmodes,
                })?;
                if absolute_posts {
                    let floor = setup
                        .floors
                        .get(floor_index as usize)
                        .ok_or(PacketError::FloorIndexOutOfRange { index: floor_index })?;
                    let pl = postlist_from_floor(floor);
                    let rng = *FLOOR1_RANGES.get(floor.multiplier as usize).ok_or(
                        PacketError::FloorMultiplierOutOfRange {
                            multiplier: floor.multiplier,
                        },
                    )?;
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
        let res_index = *mapping
            .residues
            .first()
            .ok_or(PacketError::MissingResidueSubmap)?;
        let residue = setup
            .residues
            .get(res_index as usize)
            .ok_or(PacketError::ResidueIndexOutOfRange { index: res_index })?;
        pack_residue_silent(&mut op, residue, books, ch_count, n_spectrum)
            .map_err(PacketError::Residue)?;
    }
    Ok(op.into_buffer())
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
#[allow(clippy::too_many_arguments)]
fn pack_block_packet_impl<S: F32Sample>(
    setup: &SetupInfo,
    books: &[Codebook],
    channels: u32,
    mode: u32,
    absolute_posts: &[Option<Vec<i64>>],
    mdct: &[Vec<S>],
    coupling_peak: Option<&[Vec<S>]>,
    residue_vq: bool,
    posts_are_10bit: bool,
    collect_residue: bool,
) -> Result<BlockPacketResult, PacketError> {
    let mut op = OggPack::new(1024);
    pack_audio_header(&mut op, setup, mode)?;
    let md = setup
        .modes
        .get(mode as usize)
        .ok_or(PacketError::ModeOutOfRange {
            mode,
            modes: setup.nmodes,
        })?;
    let mapping = setup
        .maps
        .get(md.mapping as usize)
        .ok_or(PacketError::MappingOutOfRange {
            mapping: md.mapping,
            maps: setup.nmaps,
        })?;
    if mdct.len() != channels as usize {
        return Err(PacketError::MdctRowCount {
            got: mdct.len(),
            want: channels as usize,
        });
    }
    let n_spectrum = mdct.first().map_or(0, Vec::len);
    for (ch, row) in mdct.iter().enumerate() {
        if row.len() < n_spectrum {
            return Err(PacketError::MdctTooShort {
                channels: ch,
                n_spectrum,
            });
        }
    }
    let stereo_peak = if channels == 2 && mapping.coupling.len() == 1 {
        coupling_peak
    } else {
        None
    };

    let mut ch_used: Vec<bool> = Vec::with_capacity(channels as usize);
    let mut residuals: Vec<Vec<f64>> = if stereo_peak.is_some() {
        Vec::new()
    } else {
        Vec::with_capacity(channels as usize)
    };
    let mut floor_indices: Vec<Vec<i64>> = if stereo_peak.is_some() {
        Vec::with_capacity(channels as usize)
    } else {
        Vec::new()
    };
    for ch in 0..channels {
        let sub = if mapping.submaps > 1 {
            *mapping
                .chmux
                .get(ch as usize)
                .ok_or(PacketError::ChannelMuxTooShort {
                    got: mapping.chmux.len(),
                    want: channels as usize,
                })?
        } else {
            0
        };
        let floor_index =
            *mapping
                .floors
                .get(sub as usize)
                .ok_or(PacketError::FloorMapTooShort {
                    got: mapping.floors.len(),
                    want: mapping.submaps as usize,
                })?;
        let floor = setup
            .floors
            .get(floor_index as usize)
            .ok_or(PacketError::FloorIndexOutOfRange { index: floor_index })?;
        let posts = absolute_posts.get(ch as usize).and_then(|p| p.as_ref());
        match posts {
            None => {
                op.write(0, 1).map_err(|_| PacketError::ModeOutOfRange {
                    mode,
                    modes: setup.nmodes,
                })?;
                ch_used.push(false);
                if stereo_peak.is_some() {
                    floor_indices.push(vec![0; n_spectrum]);
                } else {
                    residuals.push(vec![0.0; n_spectrum]);
                }
            }
            Some(posts) => {
                op.write(1, 1).map_err(|_| PacketError::ModeOutOfRange {
                    mode,
                    modes: setup.nmodes,
                })?;
                let pl = postlist_from_floor(floor);
                let rng = *FLOOR1_RANGES.get(floor.multiplier as usize).ok_or(
                    PacketError::FloorMultiplierOutOfRange {
                        multiplier: floor.multiplier,
                    },
                )?;
                let packet_posts = if posts_are_10bit {
                    floor1_quantize_posts(posts, floor.multiplier).map_err(PacketError::Floor1)?
                } else {
                    posts.to_vec()
                };
                let (raster_posts, y) = floor1_wrap_with_posts(&packet_posts, &pl, rng as i64)
                    .map_err(PacketError::Floor1)?;
                pack_floor1_body(&mut op, setup, floor_index, books, &y)?;
                ch_used.push(true);
                if stereo_peak.is_some() {
                    floor_indices.push(
                        floor1_quant_curve_from_posts(
                            &raster_posts,
                            &pl,
                            n_spectrum,
                            floor.multiplier,
                        )
                        .map_err(PacketError::Floor1)?,
                    );
                } else {
                    // Use the unwrapped fit posts to build the multiplicative
                    // floor needed by the uncoupled residue path.
                    let curve =
                        floor1_curve_from_posts(&raster_posts, &pl, n_spectrum, floor.multiplier)
                            .map_err(PacketError::Floor1)?;
                    residuals.push(mdct_to_residue(&mdct[ch as usize], &curve, 1e-8));
                }
            }
        }
    }

    let mut quantized_residue = if collect_residue {
        vec![vec![0i64; n_spectrum]; channels as usize]
    } else {
        Vec::new()
    };
    if ch_used.iter().any(|&u| u) {
        let res_index = *mapping
            .residues
            .first()
            .ok_or(PacketError::MissingResidueSubmap)?;
        let res = setup
            .residues
            .get(res_index as usize)
            .ok_or(PacketError::ResidueIndexOutOfRange { index: res_index })?;
        // Coupled streams (mapping0 coupling_steps > 0) store the (mag, ang)
        // pair in the residue domain, not raw per-channel coefficients:
        // apply the exact inverse of the decoder's 4-branch coupling
        // (no-op when the mapping has no coupling steps, e.g. the 5.1
        // profile).
        let mut mapped_quantized = None;
        if let Some(peak) = stereo_peak {
            mapped_quantized = Some(aotuv_stereo_residue(mdct, &floor_indices, peak, &ch_used)?);
        } else {
            apply_mapping_coupling(&mut residuals, &mapping.coupling)?;
        }
        propagate_mapping_nonzero(&mut ch_used, &mapping.coupling)?;
        if res.residue_type == 2 {
            // Type 2 codes the flat (bin*channels+channel) domain: the
            // quantized handoff and the bit schedule both follow that
            // layout.
            let end_flat = (res.end as usize).min(n_spectrum * channels as usize);
            let mut coded_residue = match mapped_quantized.take() {
                Some(rows) => rows,
                None => residuals
                    .iter()
                    .map(|row| {
                        row.iter()
                            .map(|value| quantize_residue_value(*value))
                            .collect()
                    })
                    .collect(),
            };
            for (channel, row) in coded_residue.iter_mut().enumerate() {
                let used = ch_used[channel];
                for (index, value) in row.iter_mut().enumerate() {
                    let flat = index * channels as usize + channel;
                    if !used || flat < res.begin as usize || flat >= end_flat {
                        *value = 0;
                    }
                }
            }
            pack_residue_type2_quantized(
                &mut op,
                res,
                books,
                &coded_residue,
                &ch_used,
                n_spectrum,
                channels as usize,
                md.blockflag != 0,
            )
            .map_err(PacketError::Residue)?;
            if collect_residue {
                quantized_residue = coded_residue;
            }
        } else {
            // Materialize the integer residue handoff once. The residue
            // packer accepts numeric rows and its own integer
            // normalization is idempotent, so these exact rows feed both
            // classification and VQ.
            let begin = res.begin as usize;
            let end = (res.end as usize).min(n_spectrum);
            quantized_residue = match mapped_quantized {
                Some(rows) => rows,
                None => residuals
                    .iter()
                    .map(|row| {
                        row.iter()
                            .map(|value| quantize_residue_value(*value))
                            .collect()
                    })
                    .collect(),
            };
            for (row, &used) in quantized_residue.iter_mut().zip(ch_used.iter()) {
                for (index, value) in row.iter_mut().enumerate() {
                    if !used || index < begin || index >= end {
                        *value = 0;
                    }
                }
            }
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
    }
    Ok(BlockPacketResult {
        packet: op.into_buffer(),
        quantized_residue,
    })
}

/// Full short/long audio packet with diagnostic residue details.
#[allow(clippy::too_many_arguments)] // stable diagnostic surface mirrors Python
pub fn pack_block_packet_details<S: F32Sample>(
    setup: &SetupInfo,
    books: &[Codebook],
    channels: u32,
    mode: u32,
    absolute_posts: &[Option<Vec<i64>>],
    mdct: &[Vec<S>],
    coupling_peak: Option<&[Vec<S>]>,
    residue_vq: bool,
    posts_are_10bit: bool,
) -> Result<BlockPacketResult, PacketError> {
    pack_block_packet_impl(
        setup,
        books,
        channels,
        mode,
        absolute_posts,
        mdct,
        coupling_peak,
        residue_vq,
        posts_are_10bit,
        true,
    )
}

/// Encode one audio block and return only its packet bytes
/// (Python `pack_block_packet`).
#[allow(clippy::too_many_arguments)] // signature mirrors Python pack_block_packet
pub fn pack_block_packet<S: F32Sample>(
    setup: &SetupInfo,
    books: &[Codebook],
    channels: u32,
    mode: u32,
    absolute_posts: &[Option<Vec<i64>>],
    mdct: &[Vec<S>],
    coupling_peak: Option<&[Vec<S>]>,
    residue_vq: bool,
    posts_are_10bit: bool,
) -> Result<Vec<u8>, PacketError> {
    Ok(pack_block_packet_impl(
        setup,
        books,
        channels,
        mode,
        absolute_posts,
        mdct,
        coupling_peak,
        residue_vq,
        posts_are_10bit,
        false,
    )?
    .packet)
}

#[cfg(test)]
mod coupling_round_trip {
    //! Round-trip proof that [`super::apply_mapping_coupling`] is the exact
    //! pre-image of the published decoder's four-branch mapping0 coupling.
    //!
    //! The published decoder (scripts/decode_wem.py::decouple_db_domain,
    //! mirroring libvorbis 1.3.7's mapping0.c) recovers a pre-coupling pair
    //! (M, A) from the stored pair (mag', ang') via four branches. The encoder
    //! stores the exact pre-image of that map; decoding the stored pair must
    //! therefore recover (M, A) exactly. This test is the formal round-trip
    //! invariant (leftover from the 2ch/48k release).

    use super::{
        aotuv_stereo_residue, apply_mapping_coupling, lossless_couple_i64,
        propagate_mapping_nonzero, PacketError,
    };
    use crate::setup::CouplingStep;

    /// Mirror of the decoder's four-branch coupling inverse, per
    /// libvorbis 1.3.7's mapping0.c:765-787 (the published decoder's
    /// decouple_db_domain in scripts/decode_wem.py transcribes it).
    /// For the stored pair (mag', ang') it returns the recovered
    /// pre-coupling (mag, ang).
    fn decode_branches(mag_stored: f64, ang_stored: f64) -> (f64, f64) {
        if mag_stored > 0.0 {
            if ang_stored > 0.0 {
                // M' > 0, A' > 0: mag = M', ang = M' - A'
                (mag_stored, mag_stored - ang_stored)
            } else {
                // M' > 0, A' <= 0: ang = M', mag = M' + A'
                (mag_stored + ang_stored, mag_stored)
            }
        } else {
            if ang_stored > 0.0 {
                // M' <= 0, A' > 0: mag = M', ang = M' + A'
                (mag_stored, mag_stored + ang_stored)
            } else {
                // M' <= 0, A' <= 0: ang = M', mag = M' - A'
                (mag_stored - ang_stored, mag_stored)
            }
        }
    }

    /// One coupling step over two channels, one bin: forward map then the
    /// decoder's four-branch inverse must be the identity.
    fn assert_round_trip(mag: f64, ang: f64) {
        let mut residuals: Vec<Vec<f64>> = vec![vec![mag], vec![ang]];
        let coupling = vec![CouplingStep { mag: 0, ang: 1 }];
        apply_mapping_coupling(&mut residuals, &coupling).unwrap();
        let (mag_stored, ang_stored) = (residuals[0][0], residuals[1][0]);
        let (mag_rec, ang_rec) = decode_branches(mag_stored, ang_stored);
        assert_eq!(
            mag_rec, mag,
            "mag mismatch for pre-coupling pair ({mag}, {ang})"
        );
        assert_eq!(
            ang_rec, ang,
            "ang mismatch for pre-coupling pair ({mag}, {ang})"
        );
    }

    #[test]
    fn mapping_coupling_propagates_one_sided_floor_use() {
        let coupling = vec![CouplingStep { mag: 0, ang: 1 }];
        for mut used in [vec![true, false], vec![false, true]] {
            propagate_mapping_nonzero(&mut used, &coupling).unwrap();
            assert_eq!(used, vec![true, true]);
        }
    }

    /// Named round-trip invariant: forward coupling followed by the decoder's
    /// four-branch inverse is the exact identity. Covers the full branch
    /// matrix plus the release-mandated boundary cases (M = 0, negative M,
    /// equal absolute values) with codec-plausible magnitudes where the
    /// floating-point cancellation is exact.
    #[test]
    fn apply_mapping_coupling_round_trips_through_decode_branches() {
        // (M, A) pairs spanning all four store branches and the boundaries.
        let cases: &[(f64, f64)] = &[
            // M > 0, A < M
            (5.0, 2.0),
            (1024.0, 512.0),
            (3.0, 1.5),
            // M > 0, A == M (equal)
            (5.0, 5.0),
            // M > 0, A > M
            (2.0, 5.0),
            // M > 0, A = -M (equal absolute values, opposite signs)
            (5.0, -5.0),
            // M < 0, A > M
            (-3.0, 5.0),
            (-5.0, 5.0),
            (-2.5, 1.0),
            // M < 0, A == M (equal, both negative)
            (-5.0, -5.0),
            // M < 0, A < M
            (-5.0, -3.0),
            // M = 0 boundary
            (0.0, 4.0),
            (0.0, -4.0),
            (0.0, 0.0),
            // small fractions (exact binary)
            (0.5, 0.25),
        ];
        for (m, a) in cases.iter() {
            assert_round_trip(*m, *a);
        }
    }

    /// Per-coefficient invariant: a full coefficient row (many bins) and a
    /// two-step coupling chain must each round-trip exactly, coefficient by
    /// coefficient, against the decoder's four-branch inverse applied in
    /// reverse step order.
    #[test]
    fn apply_mapping_coupling_round_trips_per_coefficient_and_chained_steps() {
        // Three channels, eight bins; each (mag, ang) pair is chosen so the
        // stored values exercise every branch along the row.
        let pairs: &[(f64, f64)] = &[
            (7.0, 3.0),
            (4.0, -1.5),
            (-2.0, 9.0),
            (-6.0, -6.0),
            (0.0, 2.0),
            (1.0, 1.0),
            (-0.5, 0.0),
            (1024.0, 512.0),
        ];
        let mut residuals: Vec<Vec<f64>> = vec![
            pairs.iter().map(|(m, _)| *m).collect(),
            pairs.iter().map(|(_, a)| *a).collect(),
            vec![1.25; 8],
        ];
        let original = residuals.clone();

        // Step 1 couples channels (0, 1); step 2 couples (0, 2) — the
        // decoder reverses this order.
        let coupling = vec![
            CouplingStep { mag: 0, ang: 1 },
            CouplingStep { mag: 0, ang: 2 },
        ];
        apply_mapping_coupling(&mut residuals, &coupling).unwrap();

        for step in coupling.iter().rev() {
            let (mag, ang) = (step.mag as usize, step.ang as usize);
            let (mag_row, ang_row) = if mag < ang {
                let (before_ang, from_ang) = residuals.split_at_mut(ang);
                (&mut before_ang[mag], &mut from_ang[0])
            } else {
                let (before_mag, from_mag) = residuals.split_at_mut(mag);
                (&mut from_mag[0], &mut before_mag[ang])
            };
            for (mag_value, ang_value) in mag_row.iter_mut().zip(ang_row.iter_mut()) {
                let (mag_stored, ang_stored) = (*mag_value, *ang_value);
                let (mag_rec, ang_rec) = decode_branches(mag_stored, ang_stored);
                *mag_value = mag_rec;
                *ang_value = ang_rec;
            }
        }

        for ch in 0..3usize {
            for j in 0..8usize {
                assert_eq!(
                    residuals[ch][j], original[ch][j],
                    "coefficient (ch {ch}, bin {j}) not recovered",
                );
            }
        }
    }

    #[test]
    fn coupling_rejects_unrepresentable_integer_result() {
        assert_eq!(
            lossless_couple_i64(i64::MAX, i64::MIN),
            Err(PacketError::ResidueCouplingOverflow)
        );
    }

    #[test]
    fn stereo_quantizer_rejects_non_finite_analysis() {
        let mut left = vec![0.0_f32; 128];
        left[0] = f32::INFINITY;
        let mdct = vec![left, vec![0.0; 128]];
        let floor_indices = vec![vec![0; 128], vec![0; 128]];
        let peaks = vec![vec![0.0_f32; 128], vec![0.0; 128]];
        let err = aotuv_stereo_residue(&mdct, &floor_indices, &peaks, &[true, true])
            .expect_err("non-finite MDCT must be rejected");
        assert_eq!(
            err,
            PacketError::NonFiniteAnalysisSample { channel: 0, bin: 0 }
        );
    }
}
