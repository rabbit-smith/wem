//! Vorbis residue type 0/1/2 packing and decoding for Wwise 2013.2
//! (Python: `wwise_wem/vorbis/residue.py`; the decode direction also mirrors
//! `scripts/decode_wem.py::decode_residue_coeffs`, the only place the
//! coefficient-producing residue inverse exists).
//!
//! The encoder may stop mid-residue; a decoder treats end-of-packet as
//! end-of-residue and reports it as [`ResidueStatus::EndOfPacket`] rather than
//! as a failure.

use crate::bitio::{BitReader, OggPack};
use crate::codebook::{Codebook, CodebookError};
use crate::setup::ResidueSetup;

mod sealed {
    pub trait Sealed {}

    impl Sealed for f32 {}
    impl Sealed for f64 {}
}

/// A supported spectrum sample consumed at the codec's normative f32 boundary.
///
/// This trait is sealed because the packet path only defines deterministic
/// conversion semantics for the two built-in floating-point carriers.
pub trait F32Sample: sealed::Sealed + Copy {
    fn as_f32(self) -> f32;
}

impl F32Sample for f32 {
    #[inline]
    fn as_f32(self) -> f32 {
        self
    }
}

impl F32Sample for f64 {
    #[inline]
    fn as_f32(self) -> f32 {
        self as f32
    }
}

/// Residue packing errors (Python: `ValueError` family).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ResidueError {
    /// Silent residue requires `cascade[0] == 0` (Python check).
    SilentRequiresEmptyCascade,
    /// The bounded classifier only supports the installed 8-class profile;
    /// other setups would need the encoder-only metric table / log10 path
    /// (Python `_classify_partition_heuristic` fallback).
    UnsupportedClassCount { nclass: u64 },
    /// Residue book index out of range.
    BookIndexOutOfRange { book_id: i64, books: usize },
    /// The phrasebook does not code a class combination required by the setup.
    UnencodableClassword { entry: i64, entries: i64 },
    /// Residue partitions must contain at least one scalar.
    InvalidPartitionSize,
    /// Type-2 residue requires at least one channel.
    InvalidChannelCount,
    /// Residue rows and use flags do not cover the declared channels.
    ChannelLayoutMismatch {
        channels: usize,
        residual_rows: usize,
        use_flags: usize,
    },
    /// The setup's per-class tables do not cover every declared class.
    ClassTablesTooShort {
        classifications: usize,
        cascades: usize,
        stage_books: usize,
    },
    /// A stage codebook vector does not tile one partition exactly.
    IncompatibleBookDimension {
        partition_size: usize,
        dimension: usize,
    },
}

impl std::fmt::Display for ResidueError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            ResidueError::SilentRequiresEmptyCascade => {
                write!(f, "silent residue requires cascade class0 empty")
            }
            ResidueError::UnsupportedClassCount { nclass } => {
                write!(f, "residue classification unsupported for nclass {nclass}")
            }
            ResidueError::BookIndexOutOfRange { book_id, books } => {
                write!(f, "residue book id {book_id} out of range 0..{books}")
            }
            ResidueError::UnencodableClassword { entry, entries } => {
                write!(
                    f,
                    "type-2 classword entry {entry} is not coded in 0..{entries}"
                )
            }
            ResidueError::InvalidPartitionSize => {
                write!(f, "residue partition size must be positive")
            }
            ResidueError::InvalidChannelCount => {
                write!(f, "type-2 residue channel count must be positive")
            }
            ResidueError::ChannelLayoutMismatch {
                channels,
                residual_rows,
                use_flags,
            } => write!(
                f,
                "residue channel layout needs {channels} rows and flags, got {residual_rows} rows and {use_flags} flags"
            ),
            ResidueError::ClassTablesTooShort {
                classifications,
                cascades,
                stage_books,
            } => write!(
                f,
                "residue setup declares {classifications} classes but has {cascades} cascades and {stage_books} stage-book rows"
            ),
            ResidueError::IncompatibleBookDimension {
                partition_size,
                dimension,
            } => write!(
                f,
                "residue partition size {partition_size} is not divisible by codebook dimension {dimension}"
            ),
        }
    }
}

impl std::error::Error for ResidueError {}

/// How far a residue decode got (Python `decode_residue_coeffs` status
/// strings).
///
/// `EndOfPacket` is the format's own early stop, not a failure: the Wwise
/// encoder may end a packet inside its residue, and the decoder keeps what it
/// decoded. It is returned rather than thrown, so a caller can never mistake a
/// truncated residue for a complete one.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ResidueStatus {
    /// Nothing to read: no partitions, or no channel carries a floor.
    Empty,
    /// Every coded partition was read.
    Complete,
    /// The packet ended inside the residue.
    EndOfPacket,
}

/// Residue decode-direction errors.
///
/// Structural defects the setup itself declares are reported instead of
/// indexing past a table, and a failed codeword is reported instead of being
/// folded into [`ResidueStatus::EndOfPacket`] — that status is reserved for
/// the bitstream actually ending. Wrapped causes stay reachable through
/// [`std::error::Error::source`].
#[derive(Debug, Clone, PartialEq)]
pub enum ResidueDecodeError {
    /// Only residue types 0, 1 and 2 are defined.
    UnsupportedResidueType { residue_type: u64 },
    /// Partitions must contain at least one scalar.
    InvalidPartitionSize,
    /// The type-2 flat domain needs at least one channel.
    InvalidChannelCount,
    /// The setup's per-class tables do not cover every declared class.
    ClassTablesTooShort {
        classifications: usize,
        cascades: usize,
        stage_books: usize,
    },
    /// A phrasebook entry unpacked to a class the setup does not declare.
    ClassOutOfRange {
        partition_class: i64,
        classifications: usize,
    },
    /// `classifications^(classwords-1)` is not representable.
    MixedRadixOverflow {
        classifications: u64,
        classwords: usize,
    },
    /// A cascade declares more than the eight stages a residue row holds.
    CascadeOutOfRange { cascade: u64 },
    /// A residue book id is absent from `books`.
    BookIndexOutOfRange { book_id: i64, books: usize },
    /// A stage codebook cannot tile the partition (a zero dimension would
    /// step forever).
    IncompatibleBookDimension {
        partition_size: usize,
        dimension: i64,
    },
    /// A type-2 partition-group's classword was not recorded before it was
    /// read (the group walk and the classword walk disagree).
    PartwordMissing { group: usize, groups: usize },
    /// `n_spectrum * channels` overflows the flat domain's index type.
    SpectrumTooLarge { n_spectrum: usize, channels: usize },
    /// A flat-domain write would land outside the vector it targets.
    FlatDomainOutOfRange { index: usize, flat_len: usize },
    /// A stage book could not hand back the vector its codeword named (a book
    /// that is not maptype 1, or an entry with no vector). A codeword the book
    /// simply cannot decode is *not* reported here: it ends the residue, as it
    /// does in the reference and in libvorbis.
    Codebook(CodebookError),
}

impl std::fmt::Display for ResidueDecodeError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            ResidueDecodeError::UnsupportedResidueType { residue_type } => {
                write!(f, "unsupported residue type {residue_type}")
            }
            ResidueDecodeError::InvalidPartitionSize => {
                write!(f, "residue partition size must be positive")
            }
            ResidueDecodeError::InvalidChannelCount => {
                write!(f, "type-2 residue channel count must be positive")
            }
            ResidueDecodeError::ClassTablesTooShort {
                classifications,
                cascades,
                stage_books,
            } => write!(
                f,
                "residue setup declares {classifications} classes but has {cascades} cascades and {stage_books} stage-book rows"
            ),
            ResidueDecodeError::ClassOutOfRange {
                partition_class,
                classifications,
            } => write!(
                f,
                "residue phrasebook entry unpacked to class {partition_class} outside 0..{classifications}"
            ),
            ResidueDecodeError::MixedRadixOverflow {
                classifications,
                classwords,
            } => write!(
                f,
                "residue mixed radix {classifications}^{} is not representable",
                classwords.saturating_sub(1)
            ),
            ResidueDecodeError::CascadeOutOfRange { cascade } => {
                write!(f, "residue cascade {cascade} exceeds eight stages")
            }
            ResidueDecodeError::BookIndexOutOfRange { book_id, books } => {
                write!(f, "residue book id {book_id} out of range 0..{books}")
            }
            ResidueDecodeError::IncompatibleBookDimension {
                partition_size,
                dimension,
            } => write!(
                f,
                "residue partition size {partition_size} cannot be tiled by codebook dimension {dimension}"
            ),
            ResidueDecodeError::PartwordMissing { group, groups } => write!(
                f,
                "type-2 partition group {group} was read before it was recorded ({groups} recorded)"
            ),
            ResidueDecodeError::SpectrumTooLarge {
                n_spectrum,
                channels,
            } => write!(
                f,
                "type-2 flat domain {n_spectrum} x {channels} overflows this target"
            ),
            ResidueDecodeError::FlatDomainOutOfRange { index, flat_len } => {
                write!(f, "residue flat index {index} out of range 0..{flat_len}")
            }
            ResidueDecodeError::Codebook(err) => write!(f, "residue codebook: {err}"),
        }
    }
}

impl std::error::Error for ResidueDecodeError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            ResidueDecodeError::Codebook(err) => Some(err),
            _ => None,
        }
    }
}

impl From<CodebookError> for ResidueDecodeError {
    fn from(err: CodebookError) -> Self {
        ResidueDecodeError::Codebook(err)
    }
}

/// Encoder-only maximum-coefficient thresholds for the 44.1-kHz uncoupled
/// low-residue template (Python `WWISE_RESIDUE_44_LOW_UN_METRICS`).
/// Deliberately absent from the serialized Vorbis setup packet.
const WWISE_RESIDUE_44_LOW_UN_METRICS_MAX: [i64; 7] = [0, 1, 1, 2, 2, 4, 28];
/// Encoder-only mean-absolute-coefficient thresholds; negative disables the
/// average-absolute-value threshold for that class.
const WWISE_RESIDUE_44_LOW_UN_METRICS_AVG: [i64; 7] = [-1, 25, -1, 45, -1, -1, -1];

/// Number of partitions from begin/end (optionally clipped by spectrum n)
/// (Python `partitions_to_read`).
pub fn partitions_to_read(residue: &ResidueSetup, n_spectrum: Option<usize>) -> i64 {
    let mut end = residue.end;
    let part = residue.partition_size;
    if part == 0 {
        return 0;
    }
    if let Some(n) = n_spectrum {
        if end > n as u64 {
            end = n as u64;
        }
    }
    if end <= residue.begin {
        return 0;
    }
    let span = end - residue.begin;
    (span / part) as i64
}

/// Quantize a residue coefficient using nearest-even rounding
/// (Python `quantize_residue_value` → `int(round(float(value)))`).
///
/// Python's `round` is half-even; Rust's `f64::round` is half-away-from-zero,
/// so the tie case is handled explicitly here. Domain note: the installed
/// profile keeps |value| far below 2^63, so the saturating `as i64` matches
/// Python's arbitrary-precision result on every value the pipeline can
/// produce.
pub fn quantize_residue_value(value: f64) -> i64 {
    let floored = value.floor();
    let frac = value - floored;
    let base = floored as i64;
    let mut result = base;
    if frac > 0.5 || (frac == 0.5 && base % 2 != 0) {
        result += 1;
    }
    result
}

/// Select the Wwise residue class for one partition
/// (Python `classify_partition`).
///
/// Classification operates on the already integer-quantized residue, scans
/// class 0 through `nclass - 2`, and selects the first class satisfying both
/// its maximum absolute coefficient and (when enabled) mean-absolute
/// coefficient thresholds. Only the installed 8-class profile metrics are
/// available; other setups raise [`ResidueError::UnsupportedClassCount`].
pub fn classify_partition(samples: &[f64], nclass: u64) -> Result<i64, ResidueError> {
    if nclass != 8 {
        return Err(ResidueError::UnsupportedClassCount { nclass });
    }
    if samples.is_empty() {
        return Ok(0);
    }
    let mut peak = 0i64;
    let mut sum_abs = 0i64;
    for sample in samples {
        let magnitude = quantize_residue_value(*sample)
            .checked_abs()
            .unwrap_or(i64::MAX);
        peak = peak.max(magnitude);
        sum_abs = sum_abs.saturating_add(magnitude);
    }
    let average_x100 = sum_abs.saturating_mul(100) / samples.len() as i64;
    for classification in 0..WWISE_RESIDUE_44_LOW_UN_METRICS_MAX.len() {
        let max_metric = WWISE_RESIDUE_44_LOW_UN_METRICS_MAX[classification];
        let avg_metric = WWISE_RESIDUE_44_LOW_UN_METRICS_AVG[classification];
        if peak <= max_metric && (avg_metric < 0 || average_x100 < avg_metric) {
            return Ok(classification as i64);
        }
    }
    Ok(nclass as i64 - 1)
}

fn book_or_err(books: &[Codebook], book_id: i64) -> Result<&Codebook, ResidueError> {
    books
        .get(book_id as usize)
        .ok_or(ResidueError::BookIndexOutOfRange {
            book_id,
            books: books.len(),
        })
}

fn validate_class_tables(residue: &ResidueSetup) -> Result<(), ResidueError> {
    let classifications = usize::try_from(residue.classifications).unwrap_or(usize::MAX);
    if classifications == 0
        || residue.cascades.len() < classifications
        || residue.books.len() < classifications
    {
        return Err(ResidueError::ClassTablesTooShort {
            classifications,
            cascades: residue.cascades.len(),
            stage_books: residue.books.len(),
        });
    }
    Ok(())
}

fn validate_channel_layout(
    residuals: &[Vec<i64>],
    ch_used: &[bool],
    channels: usize,
) -> Result<(), ResidueError> {
    if residuals.len() < channels || ch_used.len() != channels {
        return Err(ResidueError::ChannelLayoutMismatch {
            channels,
            residual_rows: residuals.len(),
            use_flags: ch_used.len(),
        });
    }
    Ok(())
}

/// Pack classwords classifications as mixed-radix classbook entry
/// (Python `_pack_classbook_entry`).
fn pack_classbook_entry(
    op: &mut OggPack,
    cb: &Codebook,
    classes: &[i64],
    nclass: u64,
) -> Result<(), ResidueError> {
    // libvorbis phrasebook values are most-significant-first:
    // entry = (...((c0 * nclass) + c1) * nclass + c2)... .  Consequently
    // decode maps the highest radix digit to the earliest partition.
    let mut entry = 0i64;
    for &c in classes {
        entry = entry * nclass as i64 + (c % nclass as i64);
    }
    if entry as u64 >= cb.entries() as u64 || cb.lengthlist()[entry as usize] <= 0 {
        // clamp to valid: use all-zero if broken
        entry = 0;
        if cb.lengthlist()[0] <= 0 {
            // nearest used
            for (e, &l) in cb.lengthlist().iter().enumerate() {
                if l > 0 {
                    entry = e as i64;
                    break;
                }
            }
        }
    }
    cb.encode(op, entry)
        .map_err(|_| ResidueError::BookIndexOutOfRange {
            book_id: entry,
            books: 0,
        })
}

/// Pack a silent residue: all partitions class 0 (Python
/// `pack_residue_silent`).
///
/// Requires `cascade[0] == 0` (no stage books) so no VQ follows.
pub fn pack_residue_silent(
    op: &mut OggPack,
    residue: &ResidueSetup,
    books: &[Codebook],
    n_channels: u32,
    n_spectrum: Option<usize>,
) -> Result<(), ResidueError> {
    if residue.partition_size == 0 {
        return Err(ResidueError::InvalidPartitionSize);
    }
    if residue.cascades.first().copied().unwrap_or(0) != 0 {
        return Err(ResidueError::SilentRequiresEmptyCascade);
    }
    let npart = partitions_to_read(residue, n_spectrum);
    let cb = book_or_err(books, residue.classbook as i64)?;
    let classwords = cb.dim() as usize;
    // Python: range(0, npart, classwords) — step count is ceil, not floor.
    for _i in 0..(npart as usize).div_ceil(classwords.max(1)) {
        for _ch in 0..n_channels {
            cb.encode(op, 0)
                .map_err(|_| ResidueError::BookIndexOutOfRange {
                    book_id: 0,
                    books: 0,
                })?;
        }
    }
    Ok(())
}

/// Pack residue type 0/1 with greedy multi-stage VQ
/// (Python `pack_residue_vq`).
///
/// `residuals[ch][bin]`: full spectrum residual (floor-divided MDCT). Only
/// bins `[begin, end)` are coded; `ch_used` restricts channels.
pub fn pack_residue_vq(
    op: &mut OggPack,
    residue: &ResidueSetup,
    books: &[Codebook],
    residuals: &[Vec<i64>],
    ch_used: &[bool],
    n_spectrum: Option<usize>,
) -> Result<(), ResidueError> {
    let begin = residue.begin as usize;
    let end = residue.end as usize;
    let part = residue.partition_size as usize;
    if part == 0 {
        return Err(ResidueError::InvalidPartitionSize);
    }
    validate_class_tables(residue)?;
    validate_channel_layout(residuals, ch_used, ch_used.len())?;
    let npart = partitions_to_read(residue, n_spectrum);
    if npart <= 0 || !ch_used.iter().any(|&u| u) {
        return Ok(());
    }

    let nclass = residue.classifications;
    let cascades = &residue.cascades;
    let stage_books = &residue.books;
    let cb = book_or_err(books, residue.classbook as i64)?;
    let classwords = cb.dim() as usize;
    let nch = ch_used.len();

    // working residual copies (mutable)
    let mut work: Vec<Vec<f64>> = Vec::with_capacity(nch);
    for ch in 0..nch {
        if ch_used[ch] {
            // Wwise's mapping-forward path converts the floor-divided MDCT to
            // integer residue before both classing and VQ.  Keep the mutable
            // work surface numeric for `best_vq` while preserving those
            // exact integral inputs (Python: float(value)).
            let mut row: Vec<f64> = residuals[ch].iter().map(|&value| value as f64).collect();
            // ensure length
            if row.len() < end {
                row.resize(end, 0.0);
            }
            work.push(row);
        } else {
            work.push(Vec::new());
        }
    }

    // classify each partition per used channel
    let mut partword = vec![vec![0i64; npart as usize]; nch];
    for j in 0..nch {
        if !ch_used[j] {
            continue;
        }
        // `i` is both the partition index and the arithmetic offset base, so
        // a range loop is intentional (clippy's iter_mut suggestion would walk
        // rows, not columns of the 2-D partword).
        #[allow(clippy::needless_range_loop)]
        for i in 0..npart as usize {
            let off = begin + i * part;
            let seg_end = (off + part).min(work[j].len());
            let seg = &work[j][off..seg_end];
            let mut c = classify_partition(seg, nclass)?;
            // if cascade empty for that class, fall back
            if cascades[c as usize] == 0 && c != 0 {
                // try nearest class with cascade
                for alt in (1..nclass as usize).rev() {
                    if cascades[alt] != 0 {
                        c = alt as i64;
                        break;
                    }
                }
            }
            partword[j][i] = c;
        }
    }

    // Interleave the phrasebook with stage 0 exactly as _01forward does.
    // This matters for bit identity: phrase entries are not a contiguous
    // prefix followed by all VQ vectors.
    let mut class_group = Vec::with_capacity(classwords);
    let mut vq_target = Vec::new();
    for s in 0..8u32 {
        for i in (0..npart as usize).step_by(classwords.max(1)) {
            if s == 0 {
                for j in 0..nch {
                    if !ch_used[j] {
                        continue;
                    }
                    class_group.clear();
                    class_group.extend((0..classwords).map(|k| {
                        let idx = i + k;
                        if idx < npart as usize {
                            partword[j][idx]
                        } else {
                            0
                        }
                    }));
                    pack_classbook_entry(op, cb, &class_group, nclass)?;
                }
            }

            for k in 0..classwords {
                let partition = i + k;
                if partition >= npart as usize {
                    break;
                }
                for j in 0..nch {
                    if !ch_used[j] {
                        continue;
                    }
                    let pclass = partword[j][partition] as usize;
                    let book_id = stage_books[pclass][s as usize];
                    if book_id < 0 {
                        continue;
                    }
                    let book = book_or_err(books, book_id)?;
                    let dim = book.dim() as usize;
                    if dim == 0 || !part.is_multiple_of(dim) {
                        return Err(ResidueError::IncompatibleBookDimension {
                            partition_size: part,
                            dimension: dim,
                        });
                    }
                    let off = begin + partition * part;
                    // Scratch VQ target: reused across dim steps of this
                    // partition instead of a per-chunk allocation, with the
                    // same zero-padding semantics as before.
                    vq_target.clear();
                    vq_target.reserve(dim);
                    for v in (0..part).step_by(dim.max(1)) {
                        vq_target.clear();
                        vq_target.extend_from_slice(&work[j][off + v..off + v + dim]);
                        let entry = book.best_vq(&vq_target).map_err(|_| {
                            ResidueError::BookIndexOutOfRange {
                                book_id,
                                books: books.len(),
                            }
                        })?;
                        book.encode(op, entry)
                            .map_err(|_| ResidueError::BookIndexOutOfRange {
                                book_id,
                                books: books.len(),
                            })?;
                        let vq_vec = book.borrow_vq(entry).map_err(|_| {
                            ResidueError::BookIndexOutOfRange {
                                book_id,
                                books: books.len(),
                            }
                        })?;
                        for d in 0..dim {
                            work[j][off + v + d] -= vq_vec[d];
                        }
                    }
                }
            }
        }
    }
    Ok(())
}

/// Class metrics for the coupled 32/44.1/48-kHz `mid` residue template in
/// aoTuV beta6.03 `lib/modes/residue_44.h`. The Wwise type-2 path uses the
/// coupled magnitude/angle classifier even though its codebooks follow the
/// separately configured Wwise setup packet.
/// (Python `WWISE_RESIDUE_TYPE2_CLASS_METRICS`.)
const TYPE2_CLASS_MAGNITUDE_METRICS: [i64; 9] = [0, 1, 1, 2, 2, 4, 8, 16, 32];
const TYPE2_CLASS_ANGLE_METRICS: [i64; 9] = [0, 0, 999, 0, 999, 4, 8, 16, 32];

/// Classify one type-2 partition (reference `_2class`)
/// (Python `_classify_partition_type2`).
///
/// The reference classifier takes two peaks over the partition's flat
/// `(bin * channels + channel)` domain: the maximum absolute coefficient of
/// channel 0, and the maximum over every other channel. It scans the class
/// metrics in order and takes the first class whose magnitude and angle thresholds
/// both hold, otherwise the last class. `long_block` is unused by the
/// reference classifier.
pub fn classify_partition_type2(
    samples_flat: &[f64],
    #[allow(unused_variables)] nclass: u64,
    channels: usize,
    #[allow(unused_variables)] long_block: bool,
) -> i64 {
    if samples_flat.is_empty() || channels == 0 {
        return 0;
    }
    let mut magnitude_peak = 0i64;
    let mut angle_peak = 0i64;
    for (index, sample) in samples_flat.iter().enumerate() {
        let magnitude = quantize_residue_value(*sample)
            .checked_abs()
            .unwrap_or(i64::MAX);
        if index % channels == 0 {
            if magnitude > magnitude_peak {
                magnitude_peak = magnitude;
            }
        } else if magnitude > angle_peak {
            angle_peak = magnitude;
        }
    }
    for classification in 0..TYPE2_CLASS_MAGNITUDE_METRICS.len() {
        let max_metric = TYPE2_CLASS_MAGNITUDE_METRICS[classification];
        let angle_metric = TYPE2_CLASS_ANGLE_METRICS[classification];
        if magnitude_peak <= max_metric && angle_peak <= angle_metric {
            return classification as i64;
        }
    }
    nclass as i64 - 1
}

/// Mixed-radix classbook entry for the type-2 phrasebook
/// (Python `_pack_classbook_entry_type2`).
///
/// entry = c0 * nclass^(ppw-1) + ... + c_{ppw-1}; the decoder unpacks the
/// same mixed radix. A valid setup must code the complete classword domain;
/// silently substituting another entry would change the partition classes.
fn pack_classbook_entry_type2(
    op: &mut OggPack,
    cb: &Codebook,
    classes: &[i64],
    nclass: u64,
) -> Result<(), ResidueError> {
    let mut entry = 0i64;
    for &c in classes {
        entry = entry * nclass as i64 + (c % nclass as i64);
    }
    if entry as u64 >= cb.entries() as u64 || cb.lengthlist()[entry as usize] <= 0 {
        return Err(ResidueError::UnencodableClassword {
            entry,
            entries: cb.entries(),
        });
    }
    cb.encode(op, entry)
        .map_err(|_| ResidueError::BookIndexOutOfRange {
            book_id: entry,
            books: 0,
        })
}

/// Integer-domain type-2 packer used after the mapping-forward quantizer.
/// Keeping this boundary explicit avoids converting integer residue to float
/// and quantizing it again before VQ.
#[allow(clippy::too_many_arguments)]
pub(crate) fn pack_residue_type2_quantized(
    op: &mut OggPack,
    residue: &ResidueSetup,
    books: &[Codebook],
    residuals: &[Vec<i64>],
    ch_used: &[bool],
    n_spectrum: usize,
    n_channels: usize,
    long_block: bool,
) -> Result<(), ResidueError> {
    if n_channels == 0 {
        return Err(ResidueError::InvalidChannelCount);
    }
    if residue.classifications != (TYPE2_CLASS_MAGNITUDE_METRICS.len() + 1) as u64 {
        return Err(ResidueError::UnsupportedClassCount {
            nclass: residue.classifications,
        });
    }
    validate_class_tables(residue)?;
    validate_channel_layout(residuals, ch_used, n_channels)?;
    let max_end = n_spectrum * n_channels;
    let begin = residue.begin as usize;
    let end = (residue.end as usize).min(max_end);
    let span = end.saturating_sub(begin);
    let part = residue.partition_size as usize;
    if part == 0 {
        return Err(ResidueError::InvalidPartitionSize);
    }
    let partvals = if span > 0 { span / part } else { 0 };
    if partvals == 0 || !ch_used.iter().any(|&u| u) {
        return Ok(());
    }

    let nclass = residue.classifications;
    let cascades = &residue.cascades;
    let stage_books = &residue.books;
    let cb = book_or_err(books, residue.classbook as i64)?;
    let ppw = if cb.dim() > 0 { cb.dim() as usize } else { 1 };
    let partwords = partvals.div_ceil(ppw);

    // VQ subtracts floating-point codebook entries from the integer-domain
    // starting values, so materialize the mutable float workspace once.
    let mut work: Vec<Vec<f64>> = Vec::with_capacity(n_channels);
    for ch in 0..n_channels {
        if ch_used[ch] {
            work.push(residuals[ch].iter().map(|value| *value as f64).collect());
        } else {
            work.push(Vec::new());
        }
    }

    // classify each partition (class shared across channels), gathering in
    // flat (bin*ch + ch) order so the metric sees the values the VQ stages
    // will write (mirror of the _vv_add_slots layout)
    let mut partword: Vec<Vec<i64>> = vec![vec![0i64; ppw]; partwords];
    let mut flat = Vec::with_capacity(part);
    // `lw`/`k` double as the partition-group index pair of the 2-D
    // partword, so range loops are intentional (clippy's iter_mut
    // suggestion would walk rows, not columns).
    #[allow(clippy::needless_range_loop)]
    for lw in 0..partwords {
        for k in 0..ppw {
            let p = lw * ppw + k;
            if p >= partvals {
                break;
            }
            let off = begin + p * part;
            flat.clear();
            for f in off..off + part {
                let bin_idx = f / n_channels;
                let ch_idx = f % n_channels;
                if ch_used[ch_idx] && bin_idx < work[ch_idx].len() {
                    flat.push(work[ch_idx][bin_idx]);
                }
            }
            partword[lw][k] = classify_partition_type2(&flat, nclass, n_channels, long_block);
        }
    }

    let mut slots = Vec::new();
    let mut target = Vec::new();
    #[allow(clippy::needless_range_loop)]
    for s in 0..8u32 {
        for lw in 0..partwords {
            if s == 0 {
                // one classword per partition-group, shared across channels
                pack_classbook_entry_type2(op, cb, &partword[lw], nclass)?;
            }
            for k in 0..ppw {
                let p = lw * ppw + k;
                if p >= partvals {
                    break;
                }
                let cls = partword[lw][k] as usize;
                if cascades[cls] & (1 << s) == 0 {
                    continue;
                }
                let book_id = stage_books[cls][s as usize];
                if book_id < 0 {
                    continue;
                }
                let book = book_or_err(books, book_id)?;
                let dim = book.dim() as usize;
                let off = begin + p * part;
                // slot iteration mirrors the libvorbis decodevv_add
                // flat-domain order (bin-major, channel round-robin);
                // `chptr` persists across while iterations exactly as
                // Python's _vv_add_slots generator state does.
                let mut i = off / n_channels;
                let m = (off + part) / n_channels;
                let mut chptr = 0usize;
                while i < m {
                    slots.clear();
                    slots.reserve(dim);
                    for _ in 0..dim {
                        if i >= m {
                            break;
                        }
                        slots.push((i, chptr));
                        chptr += 1;
                        if chptr == n_channels {
                            chptr = 0;
                            i += 1;
                        }
                    }
                    target.clear();
                    target.resize(dim, 0.0);
                    for (j, &(bin_idx, ch_idx)) in slots.iter().enumerate() {
                        if ch_idx < work.len() && bin_idx < work[ch_idx].len() {
                            target[j] = work[ch_idx][bin_idx];
                        } else {
                            target[j] = 0.0;
                        }
                    }
                    let entry =
                        book.best_vq(&target)
                            .map_err(|_| ResidueError::BookIndexOutOfRange {
                                book_id,
                                books: books.len(),
                            })?;
                    book.encode(op, entry)
                        .map_err(|_| ResidueError::BookIndexOutOfRange {
                            book_id,
                            books: books.len(),
                        })?;
                    let vq_vec =
                        book.borrow_vq(entry)
                            .map_err(|_| ResidueError::BookIndexOutOfRange {
                                book_id,
                                books: books.len(),
                            })?;
                    for ((bin_idx, ch_idx), &val) in slots.iter().zip(vq_vec.iter()) {
                        if *ch_idx < work.len() && *bin_idx < work[*ch_idx].len() {
                            work[*ch_idx][*bin_idx] -= val;
                        }
                    }
                }
            }
        }
    }
    Ok(())
}

/// One decoded value, or the packet ending before it could be read.
enum Decoded<T> {
    Value(T),
    EndOfPacket,
}

/// Decode one Huffman entry, mapping a codeword the book cannot decode to the
/// end-of-residue marker (Python `_decode_eop`; libvorbis
/// `vorbis_book_decode`).
///
/// Both the reference's `_decode_eop` and libvorbis treat *every* codeword
/// failure as end-of-residue, and this mirrors that deliberately: a bit pattern
/// the book does not assign (an incomplete code) stops the residue, and the
/// decoded prefix is returned with [`ResidueStatus::EndOfPacket`], exactly as
/// the reference decoder — this crate's comparison target — does. The stop is
/// still visible to the caller: the status says the residue ended early and the
/// bit position says where. What is *not* separable from exhaustion by the
/// return value is *why* it ended; that is the reference's semantics.
fn decode_entry_eop(
    book: &Codebook,
    br: &mut BitReader<'_>,
) -> Result<Decoded<i64>, ResidueDecodeError> {
    if br.bits_left() == 0 {
        return Ok(Decoded::EndOfPacket);
    }
    match book.decode(br) {
        Ok(entry) => Ok(Decoded::Value(entry)),
        Err(CodebookError::InvalidHuffmanCode) | Err(CodebookError::EmptyCodebook) => {
            Ok(Decoded::EndOfPacket)
        }
        Err(err) => Err(ResidueDecodeError::Codebook(err)),
    }
}

/// Decode one VQ vector, mapping bit exhaustion to end-of-residue (Python
/// `_decode_vq_eop`).
fn decode_vq_eop<'a>(
    book: &'a Codebook,
    br: &mut BitReader<'_>,
) -> Result<Decoded<&'a [f64]>, ResidueDecodeError> {
    match decode_entry_eop(book, br)? {
        Decoded::EndOfPacket => Ok(Decoded::EndOfPacket),
        Decoded::Value(entry) => Ok(Decoded::Value(book.borrow_vq(entry)?)),
    }
}

fn decode_book_or_err(books: &[Codebook], book_id: i64) -> Result<&Codebook, ResidueDecodeError> {
    books
        .get(book_id as usize)
        .ok_or(ResidueDecodeError::BookIndexOutOfRange {
            book_id,
            books: books.len(),
        })
}

fn decode_class_tables(residue: &ResidueSetup) -> Result<usize, ResidueDecodeError> {
    let classifications = residue.classifications as usize;
    if classifications == 0
        || residue.cascades.len() < classifications
        || residue.books.len() < classifications
    {
        return Err(ResidueDecodeError::ClassTablesTooShort {
            classifications,
            cascades: residue.cascades.len(),
            stage_books: residue.books.len(),
        });
    }
    Ok(classifications)
}

/// Decode residue type 0/1 into per-channel coefficient rows (Python
/// `decode_residue_coeffs`'s per-channel branch; libvorbis `_01inverse`).
///
/// `coeffs[ch][bin]` starts at zero and accumulates the multi-stage VQ
/// vectors, exactly as `pack_residue_vq` consumed them. Only bins
/// `[begin, min(end, n_spectrum))` are coded; a channel whose `ch_used` flag is
/// false contributes nothing and consumes no bits.
///
/// `EndOfPacket` is returned with the partial rows the packet did carry.
pub fn decode_residue_vq(
    br: &mut BitReader<'_>,
    residue: &ResidueSetup,
    books: &[Codebook],
    ch_used: &[bool],
    n_spectrum: usize,
) -> Result<(Vec<Vec<f64>>, ResidueStatus), ResidueDecodeError> {
    let nch = ch_used.len();
    let mut coeffs = vec![vec![0.0f64; n_spectrum]; nch];
    let part = residue.partition_size;
    if part == 0 {
        return Err(ResidueDecodeError::InvalidPartitionSize);
    }
    let end = residue.end.min(n_spectrum as u64);
    let span = end.saturating_sub(residue.begin);
    let npart = if span > 0 { span / part } else { 0 };
    if npart == 0 || !ch_used.iter().any(|&used| used) {
        return Ok((coeffs, ResidueStatus::Empty));
    }
    // npart <= span / 1 <= n_spectrum, so the conversion is exact.
    let npart = npart as usize;
    let classifications = decode_class_tables(residue)?;
    let nclass = residue.classifications;
    let classbook = decode_book_or_err(books, residue.classbook as i64)?;
    let classwords = classbook.dim() as usize;
    if classwords == 0 {
        return Err(ResidueDecodeError::IncompatibleBookDimension {
            partition_size: part as usize,
            dimension: 0,
        });
    }
    let exponent = u32::try_from(classwords.saturating_sub(1)).map_err(|_| {
        ResidueDecodeError::MixedRadixOverflow {
            classifications: nclass,
            classwords,
        }
    })?;
    let top_radix =
        (nclass as i128)
            .checked_pow(exponent)
            .ok_or(ResidueDecodeError::MixedRadixOverflow {
                classifications: nclass,
                classwords,
            })?;

    let mut partword = vec![vec![0i64; npart]; nch];
    let stage_books = &residue.books;
    let mut status = ResidueStatus::Complete;
    // The stage index is a slot of whichever class row a partition selected,
    // so it indexes into a different row per partition and there is no single
    // collection to iterate: a range loop is intentional here.
    #[allow(clippy::needless_range_loop)]
    'stages: for s in 0..8usize {
        for i in (0..npart).step_by(classwords) {
            if s == 0 {
                for (j, &used) in ch_used.iter().enumerate() {
                    if !used {
                        continue;
                    }
                    let entry = match decode_entry_eop(classbook, br)? {
                        Decoded::EndOfPacket => {
                            status = ResidueStatus::EndOfPacket;
                            break 'stages;
                        }
                        Decoded::Value(entry) => entry,
                    };
                    // libvorbis phrasebook values are most-significant-first:
                    // the highest radix digit is the earliest partition.
                    let mut temp = entry as i128;
                    let mut radix = top_radix;
                    for k in 0..classwords {
                        if i + k < npart {
                            let class = temp / radix;
                            if class >= nclass as i128 {
                                return Err(ResidueDecodeError::ClassOutOfRange {
                                    partition_class: class as i64,
                                    classifications,
                                });
                            }
                            partword[j][i + k] = class as i64;
                        }
                        temp %= radix;
                        radix /= nclass as i128;
                    }
                }
            }
            for k in 0..classwords {
                let partition = i + k;
                if partition >= npart {
                    break;
                }
                for (j, &used) in ch_used.iter().enumerate() {
                    if !used {
                        continue;
                    }
                    let pclass = partword[j][partition] as usize;
                    let book_id = stage_books[pclass][s];
                    if book_id < 0 {
                        continue;
                    }
                    let book = decode_book_or_err(books, book_id)?;
                    let dimension = book.dim();
                    if dimension <= 0 {
                        return Err(ResidueDecodeError::IncompatibleBookDimension {
                            partition_size: part as usize,
                            dimension,
                        });
                    }
                    let dimension = dimension as usize;
                    // begin + partition*part < end <= n_spectrum and the walk
                    // stops at that partition's own end, so offsets stay
                    // inside the row (elements past n_spectrum are clipped
                    // below, exactly as the reference clips them).
                    let mut v = residue.begin + partition as u64 * part;
                    let stop = v + part;
                    while v < stop {
                        let vector = match decode_vq_eop(book, br)? {
                            Decoded::EndOfPacket => {
                                status = ResidueStatus::EndOfPacket;
                                break 'stages;
                            }
                            Decoded::Value(vector) => vector,
                        };
                        for (d, &value) in vector.iter().enumerate() {
                            let index = v + d as u64;
                            if index < n_spectrum as u64 {
                                coeffs[j][index as usize] += value;
                            }
                        }
                        v += dimension as u64;
                    }
                }
            }
        }
    }
    Ok((coeffs, status))
}

/// libvorbis `vorbis_book_decodevv_add` into the flat `bin * channels +
/// channel` domain (script `_decodevv_add`).
///
/// Returns `Ok(false)` when the packet ended inside the vector stream; the
/// vectors written before that stay, which is what libvorbis leaves behind.
fn decode_vv_add(
    book: &Codebook,
    flat: &mut [f64],
    offset: u64,
    part: u64,
    channels: usize,
    br: &mut BitReader<'_>,
) -> Result<bool, ResidueDecodeError> {
    let dimension = book.dim();
    if dimension <= 0 {
        // The reference returns success for a book with no dimension: it
        // codes nothing and consumes nothing.
        return Ok(true);
    }
    let dimension = dimension as usize;
    let channels = channels as u64;
    let end_bin = (offset + part) / channels;
    let mut bin = offset / channels;
    let mut chptr = 0usize;
    while bin < end_bin {
        let vector = match decode_vq_eop(book, br)? {
            Decoded::EndOfPacket => return Ok(false),
            Decoded::Value(vector) => vector,
        };
        for &value in vector.iter().take(dimension) {
            if bin >= end_bin {
                break;
            }
            let index = bin * channels + chptr as u64;
            // index < end_bin * channels <= offset + part <= end <= max_flat,
            // so the cast is exact and the lookup cannot be out of range; it
            // is still checked so no input can reach a panic.
            let index = index as usize;
            let flat_len = flat.len();
            let slot = flat
                .get_mut(index)
                .ok_or(ResidueDecodeError::FlatDomainOutOfRange { index, flat_len })?;
            *slot += value;
            chptr += 1;
            if chptr as u64 == channels {
                chptr = 0;
                bin += 1;
            }
        }
    }
    Ok(true)
}

/// Decode residue type 2 on the flat `bin * channels + channel` domain
/// (libvorbis `res2_inverse`; script `decode_residue_type2`).
///
/// `max_flat` is the caller-supplied flat extent (`n_spectrum * channels`);
/// `end` is clamped to it. One classword per partition-group is read once for
/// all channels, and each VQ vector's values are added in bin-major,
/// channel-round-robin order.
pub fn decode_residue_type2(
    br: &mut BitReader<'_>,
    residue: &ResidueSetup,
    books: &[Codebook],
    ch_used: &[bool],
    max_flat: usize,
) -> Result<(Vec<f64>, ResidueStatus), ResidueDecodeError> {
    let channels = ch_used.len();
    if channels == 0 {
        return Err(ResidueDecodeError::InvalidChannelCount);
    }
    let part = residue.partition_size;
    if part == 0 {
        return Err(ResidueDecodeError::InvalidPartitionSize);
    }
    let end = residue.end.min(max_flat as u64);
    let mut flat = vec![0.0f64; max_flat];
    let any_used = ch_used.iter().any(|&used| used);
    let span = end.saturating_sub(residue.begin);
    if span == 0 || !any_used {
        return Ok((
            flat,
            if any_used {
                ResidueStatus::Complete
            } else {
                ResidueStatus::Empty
            },
        ));
    }
    let partvals = span / part;
    if partvals == 0 {
        return Ok((flat, ResidueStatus::Complete));
    }
    // partvals <= span / 1 <= max_flat, so the conversion is exact.
    let partvals = partvals as usize;
    let classifications = decode_class_tables(residue)?;
    let nclass = residue.classifications;
    let classbook = decode_book_or_err(books, residue.classbook as i64)?;
    let ppw = classbook.dim().max(1) as usize;
    let mut stages = 0u32;
    for &cascade in &residue.cascades {
        if cascade > u8::MAX as u64 {
            return Err(ResidueDecodeError::CascadeOutOfRange { cascade });
        }
        stages = stages.max(u64::BITS - cascade.leading_zeros());
    }

    let mut partword: Vec<Vec<i64>> = Vec::with_capacity(partvals.div_ceil(ppw));
    let mut status = ResidueStatus::Complete;
    'stages: for s in 0..stages as usize {
        let mut i = 0usize;
        let mut lg = 0usize;
        while i < partvals {
            if s == 0 {
                let entry = match decode_entry_eop(classbook, br)? {
                    Decoded::EndOfPacket => {
                        status = ResidueStatus::EndOfPacket;
                        break 'stages;
                    }
                    Decoded::Value(entry) => entry,
                };
                if entry >= classbook.entries() {
                    // libvorbis: an entry outside the phrasebook ends the
                    // residue.
                    status = ResidueStatus::EndOfPacket;
                    break 'stages;
                }
                let mut pword = vec![0i64; ppw];
                let mut temp = entry;
                for slot in pword.iter_mut().rev() {
                    *slot = temp % nclass as i64;
                    temp /= nclass as i64;
                }
                partword.push(pword);
            }
            let word = partword
                .get(lg)
                .ok_or(ResidueDecodeError::PartwordMissing {
                    group: lg,
                    groups: partword.len(),
                })?;
            for &raw in word.iter() {
                if i >= partvals {
                    break;
                }
                // A class outside the declared set reads as class 0, exactly
                // as the reference's clamp does.
                let pclass = if raw < nclass as i64 { raw } else { 0 };
                let row = residue.books.get(pclass as usize).ok_or(
                    ResidueDecodeError::ClassTablesTooShort {
                        classifications,
                        cascades: residue.cascades.len(),
                        stage_books: residue.books.len(),
                    },
                )?;
                let book_id = row[s];
                if book_id >= 0 {
                    let book = decode_book_or_err(books, book_id)?;
                    let offset = residue.begin + i as u64 * part;
                    if !decode_vv_add(book, &mut flat, offset, part, channels, br)? {
                        status = ResidueStatus::EndOfPacket;
                        break 'stages;
                    }
                }
                i += 1;
            }
            lg += 1;
        }
    }
    Ok((flat, status))
}

/// Decode residue type 0/1/2 and return per-channel coefficient rows
/// (script `decode_residue_coeffs`).
///
/// This is the coefficient-producing counterpart of the reference tree's
/// `consume_residue`: same bit schedule, but the VQ vectors are accumulated
/// into the rows instead of discarded. Type 2 codes the flat
/// `bin * channels + channel` domain and is split back into per-channel rows
/// here, so every residue type leaves through the same shape.
pub fn decode_residue_coeffs(
    br: &mut BitReader<'_>,
    residue: &ResidueSetup,
    books: &[Codebook],
    ch_used: &[bool],
    n_spectrum: usize,
) -> Result<(Vec<Vec<f64>>, ResidueStatus), ResidueDecodeError> {
    match residue.residue_type {
        0 | 1 => decode_residue_vq(br, residue, books, ch_used, n_spectrum),
        2 => {
            let channels = ch_used.len();
            let max_flat =
                n_spectrum
                    .checked_mul(channels)
                    .ok_or(ResidueDecodeError::SpectrumTooLarge {
                        n_spectrum,
                        channels,
                    })?;
            let (flat, status) = decode_residue_type2(br, residue, books, ch_used, max_flat)?;
            // coeffs[j][b] = flat[b * channels + j]; the flat extent is
            // exactly n_spectrum * channels, so every index exists.
            let rows = (0..channels)
                .map(|j| {
                    (0..n_spectrum)
                        .map(|b| flat[b * channels + j])
                        .collect::<Vec<f64>>()
                })
                .collect();
            Ok((rows, status))
        }
        other => Err(ResidueDecodeError::UnsupportedResidueType {
            residue_type: other,
        }),
    }
}

/// Multiplicative floor: residue = mdct / floor_amp (vorbis convention)
/// (Python `mdct_to_residue`).
pub fn mdct_to_residue<S: F32Sample>(mdct: &[S], floor_amp: &[f64], floor_eps: f64) -> Vec<f64> {
    let n = mdct.len().min(floor_amp.len());
    let mut out = Vec::with_capacity(mdct.len());
    for i in 0..n {
        let f = floor_amp[i];
        if f.abs() < floor_eps {
            out.push(0.0);
        } else {
            out.push((mdct[i].as_f32() as f64) / f);
        }
    }
    if mdct.len() > n {
        out.extend(mdct[n..].iter().map(|value| value.as_f32() as f64));
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn partitions_to_read_oracle() {
        // residue [0, 112), part 16 → 7; clipped spectrum 128 → 112 → 7.
        let residue = ResidueSetup {
            residue_type: 2,
            begin: 0,
            end: 112,
            partition_size: 16,
            classifications: 8,
            classbook: 0,
            cascades: vec![0; 8],
            books: vec![[0; 8]; 8],
            bit_start: 0,
            bit_end: 0,
        };
        assert_eq!(partitions_to_read(&residue, Some(128)), 7);
        assert_eq!(partitions_to_read(&residue, Some(64)), 4);
        assert_eq!(partitions_to_read(&residue, None), 7);
    }

    #[test]
    fn classify_partition_oracle() {
        // Python oracle cases on the 44.1-kHz uncoupled low metrics
        // (values are quantized first: 0.6→1, 1.4→1, 2.4→2, 4.6→5).
        assert_eq!(classify_partition(&[], 8).unwrap(), 0);
        assert_eq!(classify_partition(&[0.0], 8).unwrap(), 0);
        assert_eq!(classify_partition(&[0.6], 8).unwrap(), 2);
        assert_eq!(classify_partition(&[1.4], 8).unwrap(), 2);
        assert_eq!(classify_partition(&[2.4, 0.0], 8).unwrap(), 4);
        assert_eq!(classify_partition(&[4.6, 0.0, 0.0, 0.0], 8).unwrap(), 6);
        assert_eq!(classify_partition(&[28.0], 8).unwrap(), 6);
        assert_eq!(classify_partition(&[29.0], 8).unwrap(), 7); // fall through
    }

    #[test]
    fn quantize_residue_value_nearest_even() {
        assert_eq!(quantize_residue_value(0.4), 0);
        assert_eq!(quantize_residue_value(0.5), 0); // half-even
        assert_eq!(quantize_residue_value(1.5), 2); // half-even
        assert_eq!(quantize_residue_value(-2.5), -2); // half-even
        assert_eq!(quantize_residue_value(-0.5), -0);
    }

    #[test]
    fn classify_type2_uses_coupled_peak_metrics() {
        assert_eq!(classify_partition_type2(&[0.0, 0.0], 10, 2, false), 0);
        assert_eq!(classify_partition_type2(&[1.0, 0.0], 10, 2, false), 1);
        assert_eq!(classify_partition_type2(&[1.0, 2.0], 10, 2, false), 2);
        assert_eq!(classify_partition_type2(&[3.0, 5.0], 10, 2, false), 6);
        assert_eq!(classify_partition_type2(&[10.0, 13.0], 10, 2, false), 7);
        assert_eq!(classify_partition_type2(&[40.0, 15.0], 10, 2, false), 9);
    }

    #[test]
    fn malformed_pack_residue_inputs_return_errors() {
        let mut residue = ResidueSetup {
            residue_type: 1,
            begin: 0,
            end: 16,
            partition_size: 8,
            classifications: 8,
            classbook: 0,
            cascades: vec![0; 8],
            books: vec![[-1; 8]; 8],
            bit_start: 0,
            bit_end: 0,
        };
        let mut op = OggPack::new(16);
        let err = pack_residue_vq(&mut op, &residue, &[], &[], &[true], Some(16))
            .expect_err("missing residue row must be rejected");
        assert!(matches!(err, ResidueError::ChannelLayoutMismatch { .. }));

        residue.cascades.clear();
        let err = pack_residue_vq(&mut op, &residue, &[], &[vec![0; 16]], &[true], Some(16))
            .expect_err("short class tables must be rejected");
        assert!(matches!(err, ResidueError::ClassTablesTooShort { .. }));

        residue.residue_type = 2;
        residue.classifications = 1;
        residue.cascades = vec![0];
        residue.books = vec![[-1; 8]];
        let err = pack_residue_type2_quantized(
            &mut op,
            &residue,
            &[],
            &[vec![0; 16]],
            &[true],
            16,
            1,
            false,
        )
        .expect_err("unsupported type-2 class count must be rejected");
        assert_eq!(err, ResidueError::UnsupportedClassCount { nclass: 1 });
    }

    #[test]
    fn extreme_classifier_input_saturates_without_panicking() {
        assert_eq!(classify_partition(&[f64::NEG_INFINITY], 8).unwrap(), 7);
        assert_eq!(
            classify_partition_type2(&[f64::NEG_INFINITY, 0.0], 10, 2, false),
            9
        );
    }

    // ---- decode direction (segment 7) ----

    use crate::bitio::{BitReader, OggPack};
    use crate::codebook::{float32_unpack, StaticCodebook};

    /// `float32_unpack` word for exactly 1.0: mantissa `1 << 20` with
    /// exponent 768 (`value = mant * 2^(exp - 20 - 768)`), so a maptype-1
    /// book's unquantized values are its quantlist entries.
    const ONE_FLOAT32_WORD: i64 = 0x6010_0000;

    fn maptype0_book(dim: i64, entries: i64, length: i64) -> Codebook {
        Codebook::from_static(
            StaticCodebook {
                dim,
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

    /// A maptype-1 book whose dim-1 vectors are exactly `0..entries`, so the
    /// greedy VQ of an integer residual in that range is exact and a pack →
    /// decode round trip must be the identity.
    fn exact_vq_book(entries: i64) -> Codebook {
        Codebook::from_static(
            StaticCodebook {
                dim: 1,
                entries,
                lengthlist: vec![3; entries as usize],
                maptype: 1,
                q_min: 0,
                q_delta: ONE_FLOAT32_WORD,
                q_quant: 0,
                q_sequencep: 0,
                quantlist: Some((0..entries).collect()),
            },
            None,
            None,
            None,
        )
        .expect("synthetic maptype-1 book must build")
    }

    fn type01_residue() -> ResidueSetup {
        ResidueSetup {
            residue_type: 0,
            begin: 0,
            end: 16,
            partition_size: 4,
            classifications: 8,
            classbook: 0,
            cascades: vec![1; 8],
            books: vec![[1, -1, -1, -1, -1, -1, -1, -1]; 8],
            bit_start: 0,
            bit_end: 0,
        }
    }

    fn type2_residue() -> ResidueSetup {
        ResidueSetup {
            residue_type: 2,
            begin: 0,
            end: 16,
            partition_size: 4,
            classifications: 10,
            classbook: 0,
            cascades: vec![1; 10],
            books: vec![[1, -1, -1, -1, -1, -1, -1, -1]; 10],
            bit_start: 0,
            bit_end: 0,
        }
    }

    fn integer_rows() -> Vec<Vec<i64>> {
        vec![
            vec![0, 1, 2, 3, 4, 3, 2, 1, 0, 0, 0, 0, 1, 1, 1, 1],
            vec![4, 4, 0, 0, 1, 2, 3, 4, 2, 2, 2, 2, 0, 3, 3, 0],
        ]
    }

    fn as_f64(rows: &[Vec<i64>]) -> Vec<Vec<f64>> {
        rows.iter()
            .map(|row| row.iter().map(|&value| value as f64).collect())
            .collect()
    }

    #[test]
    fn exact_vq_book_unquantizes_to_its_quantlist() {
        // Guards the fixture: without exact integer vectors the round trips
        // below would compare VQ reconstructions, not the bit schedule.
        assert_eq!(float32_unpack(ONE_FLOAT32_WORD as u32), 1.0);
        let book = exact_vq_book(5);
        for entry in 0..5 {
            assert_eq!(book.vq_values(entry).unwrap(), vec![entry as f64]);
        }
    }

    #[test]
    fn residue_type01_round_trips_pack_then_decode() {
        let residue = type01_residue();
        let books = vec![maptype0_book(2, 64, 6), exact_vq_book(5)];
        let rows = integer_rows();
        let expected = as_f64(&rows);

        let mut op = OggPack::new(64);
        pack_residue_vq(&mut op, &residue, &books, &rows, &[true, true], Some(16)).unwrap();
        let bytes = op.into_buffer();

        let mut br = BitReader::new(&bytes);
        let (decoded, status) =
            decode_residue_vq(&mut br, &residue, &books, &[true, true], 16).unwrap();
        assert_eq!(status, ResidueStatus::Complete);
        assert_eq!(decoded, expected);
        // The decoder consumed exactly the schedule the packer wrote.
        assert!(br.bits_left() < 8);
    }

    #[test]
    fn residue_type01_skips_unused_channels_on_both_sides() {
        let residue = type01_residue();
        let books = vec![maptype0_book(2, 64, 6), exact_vq_book(5)];
        let rows = integer_rows();

        let mut op = OggPack::new(64);
        pack_residue_vq(&mut op, &residue, &books, &rows, &[true, false], Some(16)).unwrap();
        let bytes = op.into_buffer();

        let mut br = BitReader::new(&bytes);
        let (decoded, status) =
            decode_residue_vq(&mut br, &residue, &books, &[true, false], 16).unwrap();
        assert_eq!(status, ResidueStatus::Complete);
        assert_eq!(decoded[0], as_f64(&rows)[0]);
        assert_eq!(decoded[1], vec![0.0; 16]);
        assert!(br.bits_left() < 8);
    }

    #[test]
    fn residue_type2_round_trips_pack_then_decode() {
        let residue = type2_residue();
        // dim 2 phrasebook: one classword per partition-group, shared by both
        // channels; 10 classes ⇒ 100 combinations the packer can code.
        let books = vec![maptype0_book(2, 100, 7), exact_vq_book(5)];
        let rows = integer_rows();
        let n_spectrum = 8usize;
        let channels = 2usize;
        let coded: Vec<Vec<i64>> = rows.iter().map(|row| row[..n_spectrum].to_vec()).collect();
        let mut flat_source = vec![0i64; n_spectrum * channels];
        for (ch, row) in coded.iter().enumerate() {
            for (bin, &value) in row.iter().enumerate() {
                flat_source[bin * channels + ch] = value;
            }
        }

        let mut op = OggPack::new(64);
        pack_residue_type2_quantized(
            &mut op,
            &residue,
            &books,
            &coded,
            &[true, true],
            n_spectrum,
            channels,
            false,
        )
        .unwrap();
        let bytes = op.into_buffer();

        let mut br = BitReader::new(&bytes);
        let (flat, status) =
            decode_residue_type2(&mut br, &residue, &books, &[true, true], 16).unwrap();
        assert_eq!(status, ResidueStatus::Complete);
        assert_eq!(
            flat,
            flat_source.iter().map(|&v| v as f64).collect::<Vec<_>>()
        );
        assert!(br.bits_left() < 8);

        // The composite entry point splits the flat domain back into rows.
        let mut br = BitReader::new(&bytes);
        let (decoded, status) =
            decode_residue_coeffs(&mut br, &residue, &books, &[true, true], n_spectrum).unwrap();
        assert_eq!(status, ResidueStatus::Complete);
        assert_eq!(decoded, as_f64(&coded));
    }

    #[test]
    fn truncated_residue_streams_report_end_of_packet_without_panicking() {
        // Every prefix of a packed residue is a legal truncated packet: the
        // decode must keep what it read and report the early stop, never
        // panic and never fail.
        let residue = type01_residue();
        let books = vec![maptype0_book(2, 64, 6), exact_vq_book(5)];
        let rows = integer_rows();
        let mut op = OggPack::new(64);
        pack_residue_vq(&mut op, &residue, &books, &rows, &[true, true], Some(16)).unwrap();
        let bytes = op.into_buffer();
        assert!(bytes.len() > 2);

        for cut in 0..bytes.len() {
            let mut br = BitReader::new(&bytes[..cut]);
            let (_, status) = decode_residue_vq(&mut br, &residue, &books, &[true, true], 16)
                .unwrap_or_else(|err| panic!("prefix {cut} reported {err}"));
            assert!(
                matches!(status, ResidueStatus::Complete | ResidueStatus::EndOfPacket),
                "prefix {cut} reported {status:?}"
            );
        }

        let residue2 = type2_residue();
        let books2 = vec![maptype0_book(2, 100, 7), exact_vq_book(5)];
        let coded: Vec<Vec<i64>> = integer_rows().iter().map(|row| row[..8].to_vec()).collect();
        let mut op = OggPack::new(64);
        pack_residue_type2_quantized(
            &mut op,
            &residue2,
            &books2,
            &coded,
            &[true, true],
            8,
            2,
            false,
        )
        .unwrap();
        let bytes = op.into_buffer();
        for cut in 0..bytes.len() {
            let mut br = BitReader::new(&bytes[..cut]);
            let (flat, status) =
                decode_residue_type2(&mut br, &residue2, &books2, &[true, true], 16)
                    .unwrap_or_else(|err| panic!("prefix {cut} reported {err}"));
            assert_eq!(flat.len(), 16);
            assert!(
                matches!(status, ResidueStatus::Complete | ResidueStatus::EndOfPacket),
                "prefix {cut} reported {status:?}"
            );
        }
    }

    #[test]
    fn malformed_residue_inputs_return_errors() {
        let books = vec![maptype0_book(2, 64, 6), exact_vq_book(5)];
        let mut br = BitReader::new(&[0xFF, 0xFF, 0xFF, 0xFF]);

        // A residue type outside 0/1/2 is reported, not guessed at.
        let mut residue = type01_residue();
        residue.residue_type = 3;
        assert_eq!(
            decode_residue_coeffs(&mut br, &residue, &books, &[true, true], 16).unwrap_err(),
            ResidueDecodeError::UnsupportedResidueType { residue_type: 3 }
        );

        // A zero partition size cannot be walked.
        let mut residue = type01_residue();
        residue.partition_size = 0;
        assert_eq!(
            decode_residue_vq(&mut br, &residue, &books, &[true, true], 16).unwrap_err(),
            ResidueDecodeError::InvalidPartitionSize
        );

        // Class tables shorter than the declared class count.
        let mut residue = type01_residue();
        residue.cascades.truncate(2);
        assert_eq!(
            decode_residue_vq(&mut br, &residue, &books, &[true, true], 16).unwrap_err(),
            ResidueDecodeError::ClassTablesTooShort {
                classifications: 8,
                cascades: 2,
                stage_books: 8,
            }
        );

        // A classbook id the caller did not supply.
        let mut residue = type01_residue();
        residue.classbook = 9;
        assert_eq!(
            decode_residue_vq(&mut br, &residue, &books, &[true, true], 16).unwrap_err(),
            ResidueDecodeError::BookIndexOutOfRange {
                book_id: 9,
                books: 2
            }
        );

        // A stage book id the caller did not supply.
        let mut residue = type01_residue();
        residue.books = vec![[5, -1, -1, -1, -1, -1, -1, -1]; 8];
        assert_eq!(
            decode_residue_vq(&mut br, &residue, &books, &[true, true], 16).unwrap_err(),
            ResidueDecodeError::BookIndexOutOfRange {
                book_id: 5,
                books: 2
            }
        );

        // Type 2 needs at least one channel.
        let residue2 = type2_residue();
        assert_eq!(
            decode_residue_type2(&mut br, &residue2, &books, &[], 0).unwrap_err(),
            ResidueDecodeError::InvalidChannelCount
        );

        // A flat extent that overflows the index type.
        assert_eq!(
            decode_residue_coeffs(&mut br, &residue2, &books, &[true, true], usize::MAX)
                .unwrap_err(),
            ResidueDecodeError::SpectrumTooLarge {
                n_spectrum: usize::MAX,
                channels: 2
            }
        );
    }

    #[test]
    fn a_phrasebook_entry_outside_the_class_domain_is_reported() {
        // 2 classes with a dim-2 phrasebook: entries 0..8 unpack to radix
        // digits 0..7, and a digit at or above the class count is a malformed
        // classword rather than a table index.
        let mut residue = type01_residue();
        residue.classifications = 2;
        residue.cascades = vec![1, 1];
        residue.books = vec![[1, -1, -1, -1, -1, -1, -1, -1]; 2];
        let books = vec![maptype0_book(2, 8, 3), exact_vq_book(5)];
        let mut op = OggPack::new(8);
        books[0].encode(&mut op, 5).unwrap();
        let bytes = op.into_buffer();
        let mut br = BitReader::new(&bytes);
        assert_eq!(
            decode_residue_vq(&mut br, &residue, &books, &[true, true], 16).unwrap_err(),
            ResidueDecodeError::ClassOutOfRange {
                partition_class: 2,
                classifications: 2
            }
        );
    }

    #[test]
    fn decode_errors_expose_their_cause() {
        use std::error::Error;
        let err = ResidueDecodeError::Codebook(CodebookError::EmptyCodebook);
        assert!(err.source().is_some());
        assert!(ResidueDecodeError::InvalidPartitionSize.source().is_none());
    }
}
