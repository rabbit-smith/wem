//! Vorbis residue type 0/1 packing for Wwise 2013.2
//! (Python: `wwise_wem/vorbis/residue.py`).

use crate::bitio::OggPack;
use crate::codebook::Codebook;
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
    fn malformed_residue_inputs_return_errors() {
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
}
